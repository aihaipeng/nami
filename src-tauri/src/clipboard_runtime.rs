use crate::{commands::on_main, platform, AppState};
use nami::{
    clipboard::{
        self, ClipboardFileRepresentation, ClipboardRecord, ClipboardTextFormatting, RecordingTypes,
    },
    database::HistoryDatabase,
};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) async fn database<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce(&mut HistoryDatabase) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let app = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<AppState>();
        if let Some(error) = state.storage_error.lock().unwrap().clone() {
            return Err(error);
        }
        let mut database = state.database.lock().unwrap();
        work(database.as_mut().ok_or("storage_unavailable")?)
    })
    .await
    .map_err(|_| "storage_worker")?
}

pub(crate) fn visibility(app: &AppHandle, visible: bool) -> Result<(), String> {
    if let Some(view) = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
    {
        view.emit(
            "window://visibility",
            serde_json::json!({"label":"clipboard","visible":visible}),
        )
        .map_err(|_| "event_emit")?;
    }
    Ok(())
}

pub(crate) fn changed(app: &AppHandle) -> Result<(), String> {
    let revision = app
        .state::<AppState>()
        .clipboard_revision
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    for label in ["clipboard", "translation"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("clipboard_changed", revision)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

pub(crate) async fn sync_pin_paste_keys(app: &AppHandle) -> Result<(), String> {
    let changed_keys = database(app, HistoryDatabase::reconcile_pin_paste_keys).await?;
    if changed_keys {
        changed(app)?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
pub(crate) enum PayloadMode {
    Stored,
    Restore { plain_text: bool },
    PlainText,
    FilePaths,
}

fn file_paths_payload(
    record: &ClipboardRecord,
    representation: Option<&ClipboardFileRepresentation>,
) -> Result<platform::ClipboardPayload, String> {
    let paths: Vec<String> = match record.kind.as_str() {
        "files" => serde_json::from_str(&record.text).map_err(|_| "storage_corrupt")?,
        "image" => match representation {
            Some(value) => value.paths.clone(),
            None => return Err("invalid_argument".into()),
        },
        _ => return Err("invalid_argument".into()),
    };
    if paths.is_empty()
        || paths.len() > clipboard::MAX_FILE_COUNT
        || paths
            .iter()
            .any(|path| !Path::new(path).is_absolute() || path.contains('\0'))
    {
        return Err("storage_corrupt".into());
    }
    Ok(platform::ClipboardPayload::Text(paths.join("\n")))
}

// Caller holds clipboard_writes while reading the record and its image files.
pub(crate) async fn payload(
    app: &AppHandle,
    id: String,
    mode: PayloadMode,
) -> Result<platform::ClipboardPayload, String> {
    let root = image_root(app)?;
    let (record, files, formatting) = database(app, move |database| {
        let record = database.clipboard_record(&id)?;
        let files = if matches!(mode, PayloadMode::Restore { .. } | PayloadMode::FilePaths)
            && record.kind != "text"
        {
            database.clipboard_file_representation(&id)?
        } else {
            None
        };
        let formatting = if matches!(mode, PayloadMode::Restore { plain_text: false })
            && record.kind == "text"
        {
            database.clipboard_text_formatting(&id)?
        } else {
            None
        };
        Ok((record, files, formatting))
    })
    .await?;
    match mode {
        PayloadMode::PlainText if record.kind == "text" => {
            return Ok(platform::ClipboardPayload::Text(record.text));
        }
        PayloadMode::FilePaths => {
            return file_paths_payload(&record, files.as_ref());
        }
        PayloadMode::PlainText => return Err("invalid_argument".into()),
        _ => (),
    }
    if let Some(formatting) = formatting {
        return Ok(platform::ClipboardPayload::RichText {
            text: record.text,
            formatting,
        });
    }
    tauri::async_runtime::spawn_blocking(move || record_payload(&root, record, files))
        .await
        .map_err(|_| "storage_worker")?
}

fn export_png(payload: platform::ClipboardPayload) -> Result<Vec<u8>, String> {
    let (bytes, extension) = match platform::prepare_image_paste(payload) {
        platform::ClipboardPayload::Image { bytes, extension }
        | platform::ClipboardPayload::FilesImage {
            bytes,
            extension,
            image_fallback: true,
            ..
        } => (bytes, extension),
        _ => return Err("invalid_argument".into()),
    };
    if extension != "png" || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("clipboard_image_format".into());
    }
    Ok(bytes)
}

const DRAG_EXPORT_TTL_SECS: u64 = 600;

fn prune_stale_drag_exports(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let expired = meta
            .modified()
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age.as_secs() >= DRAG_EXPORT_TTL_SECS);
        if expired {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn prepare_drag_payload(
    cache_dir: &Path,
    payload: platform::ClipboardPayload,
) -> Result<platform::DragItem, String> {
    match payload {
        platform::ClipboardPayload::Text(text)
        | platform::ClipboardPayload::RichText { text, .. } => {
            if text.is_empty() {
                return Err("invalid_argument".into());
            }
            Ok(platform::DragItem::Text(text))
        }
        platform::ClipboardPayload::Files(paths)
        | platform::ClipboardPayload::FilesImage {
            paths,
            image_fallback: false,
            ..
        } => {
            if paths.is_empty() || paths.len() > clipboard::MAX_FILE_COUNT {
                return Err("storage_corrupt".into());
            }
            let mut resolved = Vec::with_capacity(paths.len());
            for path in paths {
                if path.contains('\0') {
                    return Err("storage_corrupt".into());
                }
                let candidate = PathBuf::from(&path);
                if !candidate.is_absolute() {
                    return Err("storage_corrupt".into());
                }
                if !candidate.exists() {
                    return Err("clipboard_file_missing".into());
                }
                resolved.push(candidate);
            }
            let preview_image = (resolved.len() == 1)
                .then(|| resolved[0].clone())
                .filter(|path| {
                    path.is_file()
                        && path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .is_some_and(|ext| {
                                matches!(
                                    ext.to_ascii_lowercase().as_str(),
                                    "png"
                                        | "jpg"
                                        | "jpeg"
                                        | "webp"
                                        | "gif"
                                        | "bmp"
                                        | "tif"
                                        | "tiff"
                                        | "heic"
                                )
                            })
                });
            Ok(platform::DragItem::Files {
                paths: resolved,
                preview_image,
            })
        }
        image_payload @ (platform::ClipboardPayload::Image { .. }
        | platform::ClipboardPayload::FilesImage {
            image_fallback: true,
            ..
        }) => {
            let png = export_png(image_payload)?;
            let directory = cache_dir.join("drag-exports");
            std::fs::create_dir_all(&directory).map_err(|_| "clipboard_drag_failed")?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700));
            }
            prune_stale_drag_exports(&directory);
            let filename = format!(
                "Nami_{}_{}.png",
                chrono::Local::now().format("%Y-%m-%d_%H-%M-%S"),
                &uuid::Uuid::new_v4().simple().to_string()[..8]
            );
            let target = directory.join(filename);
            std::fs::write(&target, png).map_err(|_| "clipboard_drag_failed")?;
            Ok(platform::DragItem::Files {
                paths: vec![target.clone()],
                preview_image: Some(target),
            })
        }
    }
}

pub(crate) async fn prepare_drag_item(
    app: &AppHandle,
    id: String,
) -> Result<platform::DragItem, String> {
    let cache_dir = app
        .path()
        .app_cache_dir()
        .map_err(|_| "storage_unavailable")?;
    let payload = {
        let state = app.state::<AppState>();
        let _write = state.clipboard_writes.lock().await;
        payload(app, id, PayloadMode::Stored).await?
    };
    let item =
        tauri::async_runtime::spawn_blocking(move || prepare_drag_payload(&cache_dir, payload))
            .await
            .map_err(|_| "storage_worker")??;
    app.state::<AppState>()
        .cache_cleanup_dirty
        .store(true, Ordering::SeqCst);
    Ok(item)
}

fn record_payload(
    root: &Path,
    record: ClipboardRecord,
    representation: Option<ClipboardFileRepresentation>,
) -> Result<platform::ClipboardPayload, String> {
    if let Some(mut representation) = representation {
        let bytes = clipboard::read_image(root, &representation.image_path)?;
        let extension = clipboard::image_extension_from_path(&representation.image_path)
            .ok_or("storage_corrupt")?;
        if record.kind == "image" {
            // Image history is immutable; never let a reused screenshot path change its file payload.
            representation.paths = vec![root
                .join(&representation.image_path)
                .to_string_lossy()
                .into_owned()];
        }
        return Ok(platform::ClipboardPayload::FilesImage {
            paths: representation.paths,
            bytes,
            extension: extension.into(),
            image_fallback: record.kind == "image",
        });
    }
    match record.kind.as_str() {
        "text" => Ok(platform::ClipboardPayload::Text(record.text)),
        "files" => Ok(platform::ClipboardPayload::Files(
            serde_json::from_str(&record.text).map_err(|_| "storage_corrupt")?,
        )),
        "image" => {
            let path = record.image_path.ok_or("storage_corrupt")?;
            let bytes = clipboard::read_image(root, &path)?;
            let extension = clipboard::image_extension_from_path(&path).ok_or("storage_corrupt")?;
            Ok(platform::ClipboardPayload::Image {
                bytes,
                extension: extension.into(),
            })
        }
        _ => Err("storage_corrupt".into()),
    }
}

fn observe_pending(
    handled: &AtomicI64,
    version: i64,
    copy_busy: bool,
    read: impl FnOnce() -> Result<Option<platform::ClipboardCapture>, &'static str>,
) -> Result<Option<(i64, platform::ClipboardCapture)>, &'static str> {
    let previous = handled.load(Ordering::SeqCst);
    if copy_busy || previous == version {
        return Ok(None);
    }
    let capture = match read() {
        Ok(capture) => capture,
        Err(error) => {
            if !retryable(error) {
                acknowledge(handled, previous, version);
            }
            return Err(error);
        }
    };
    if capture.is_none() {
        acknowledge(handled, previous, version);
    }
    Ok(capture.map(|capture| (previous, capture)))
}

fn retryable(error: &str) -> bool {
    matches!(
        error,
        "storage_busy"
            | "storage_unavailable"
            | "storage_worker"
            | "clipboard_read"
            | "clipboard_changed"
            | "main_thread_unavailable"
    )
}

fn acknowledge(handled: &AtomicI64, baseline: i64, version: i64) {
    let _ = handled.compare_exchange(baseline, version, Ordering::SeqCst, Ordering::SeqCst);
}

async fn retry_capture(app: &AppHandle, error: String, previous: &mut Option<String>) {
    if previous.as_ref() != Some(&error) {
        capture_error(app, &error);
        *previous = Some(error);
    }
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
}

pub(crate) fn start(app: &AppHandle) {
    start_with_poll_interval(app, std::time::Duration::from_millis(150));
}

pub(crate) fn start_with_poll_interval(app: &AppHandle, interval: std::time::Duration) {
    if !platform::capabilities().clipboard_write {
        return;
    }
    let state = app.state::<AppState>();
    state
        .clipboard_version
        .store(platform::clipboard_version(), Ordering::SeqCst);
    let maintenance = app.clone();
    tauri::async_runtime::spawn(async move {
        // Always run retention cleanup once on launch, even when periodic cleanup is set to 0.
        {
            let state = maintenance.state::<AppState>();
            let _write = state.clipboard_writes.lock().await;
            crate::clipboard_preferences::enforce_history(&maintenance, true).await;
        }
        loop {
            let interval_hours = maintenance
                .state::<AppState>()
                .clipboard_options
                .lock()
                .unwrap()
                .history_policy
                .auto_cleanup_interval_hours;
            if interval_hours == 0 {
                maintenance
                    .state::<AppState>()
                    .cleanup_schedule_notify
                    .notified()
                    .await;
                continue;
            }
            let delay = std::time::Duration::from_secs(u64::from(interval_hours) * 3600);
            let state = maintenance.state::<AppState>();
            let wakeup = state.cleanup_schedule_notify.notified();
            tokio::select! {
                _ = tokio::time::sleep(delay) => {
                    let _write = state.clipboard_writes.lock().await;
                    crate::clipboard_preferences::enforce_history(&maintenance, true).await;
                }
                _ = wakeup => {}
            }
        }
    });
    let cache_maintenance = app.clone();
    tauri::async_runtime::spawn(async move {
        const CACHE_CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30 * 60);
        // Run lossless cache maintenance independently on launch and every fixed 30 minutes.
        loop {
            {
                let state = cache_maintenance.state::<AppState>();
                let _write = state.clipboard_writes.lock().await;
                crate::storage_location::run_auto_cache_cleanup_locked(&cache_maintenance).await;
            }
            tokio::time::sleep(CACHE_CLEANUP_INTERVAL).await;
        }
    });
    let (index_tx, mut index_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    if platform::capabilities().image_ocr {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some(id) = index_rx.recv().await {
                let result = crate::clipboard_text::index_image(&app, id.clone()).await;
                app.state::<AppState>()
                    .image_index_jobs
                    .lock()
                    .unwrap()
                    .finish(id, &result);
                let _ = changed(&app);
            }
        });
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut failure = None;
        loop {
            tokio::time::sleep(interval).await;
            if !app
                .state::<AppState>()
                .clipboard_enabled
                .load(Ordering::SeqCst)
            {
                failure = None;
                continue;
            }
            let state = app.state::<AppState>();
            // Keep exclusions stable from the pre-read check through persistence.
            let _write = state.clipboard_writes.lock().await;
            let read = on_main(&app, |app| {
                let state = app.state::<AppState>();
                if !state.clipboard_enabled.load(Ordering::SeqCst) {
                    return Ok(None);
                }
                observe_pending(
                    &state.clipboard_version,
                    platform::clipboard_version(),
                    state.selection_copy_busy.load(Ordering::SeqCst),
                    || {
                        if state.clipboard_recording_types.lock().unwrap().any() {
                            let options = state.clipboard_options.lock().unwrap();
                            platform::read_clipboard_filtered(
                                |bundle_id| options.excludes(bundle_id),
                                |kind| options.ignores_pasteboard_type(kind),
                            )
                        } else {
                            Ok(None)
                        }
                    },
                )
                .map_err(str::to_owned)
            })
            .await;
            let (baseline, capture) = match read {
                Ok(Ok(Some(pending))) => pending,
                Ok(Ok(None)) => continue,
                Ok(Err(error)) | Err(error) => {
                    drop(_write);
                    retry_capture(&app, error, &mut failure).await;
                    continue;
                }
            };
            if !state.clipboard_enabled.load(Ordering::SeqCst)
                || state.clipboard_version.load(Ordering::SeqCst) != baseline
            {
                continue;
            }
            if payload_ignored_by_text_rules(&capture.payload, |text| {
                state.clipboard_options.lock().unwrap().ignores_text(text)
            }) {
                acknowledge(&state.clipboard_version, baseline, capture.version);
                failure = None;
                continue;
            }
            let root = match image_root(&app) {
                Ok(root) => root,
                Err(error) => {
                    drop(_write);
                    retry_capture(&app, error, &mut failure).await;
                    continue;
                }
            };
            let captured_version = capture.version;
            let types = *state.clipboard_recording_types.lock().unwrap();
            let worker = app.clone();
            let auto_index = platform::capabilities().image_ocr;
            let result = tauri::async_runtime::spawn_blocking(move || {
                let had_image = matches!(capture.payload, platform::ClipboardPayload::Image { .. });
                let mut index_job = Ok(None);
                let saved = persist_capture_with(
                    &root,
                    capture,
                    types,
                    |record, representation, formatting, directory_paths| {
                        let state = worker.state::<AppState>();
                        if let Some(error) = state.storage_error.lock().unwrap().clone() {
                            return Err(error);
                        }
                        let mut database = state.database.lock().unwrap();
                        let db = database.as_mut().ok_or("storage_unavailable")?;
                        let persisted = db.clipboard_insert_capture_with_dirs(
                            record,
                            representation,
                            formatting,
                            directory_paths,
                        )?;
                        // OCR lookup happens after commit; its failure must not trigger image cleanup.
                        index_job = pending_image_index(db, persisted, auto_index);
                        Ok(())
                    },
                )?;
                if had_image {
                    platform::release_allocator_memory();
                }
                index_job.map(|job| (saved, job))
            })
            .await
            .map_err(|_| "storage_worker".to_owned())
            .and_then(|result| result);
            if matches!(result, Ok((true, _))) {
                crate::clipboard_preferences::enforce_history_locked(&app).await;
            }
            if let Ok((_, Some(id))) = &result {
                if state.image_index_jobs.lock().unwrap().start(id) {
                    let _ = index_tx.send(id.clone());
                }
            }
            drop(_write);
            match result {
                Ok((saved, _)) => {
                    acknowledge(&state.clipboard_version, baseline, captured_version);
                    failure = None;
                    if saved {
                        let _ = changed(&app);
                    }
                }
                Err(error) => {
                    if !retryable(&error) {
                        acknowledge(&state.clipboard_version, baseline, captured_version);
                    }
                    retry_capture(&app, error, &mut failure).await;
                }
            }
        }
    });
}

fn pending_image_index(
    db: &HistoryDatabase,
    saved: ClipboardRecord,
    auto_index: bool,
) -> Result<Option<String>, String> {
    if !auto_index || saved.kind != "image" {
        return Ok(None);
    }
    Ok(db
        .clipboard_ocr_text(&saved.id)?
        .is_none()
        .then_some(saved.id))
}

fn payload_ignored_by_text_rules(
    payload: &platform::ClipboardPayload,
    ignores: impl FnOnce(&str) -> bool,
) -> bool {
    match payload {
        platform::ClipboardPayload::Text(text)
        | platform::ClipboardPayload::RichText { text, .. } => ignores(text),
        _ => false,
    }
}

fn capture_error(app: &AppHandle, error: &str) {
    if let Some(view) = app.get_webview("clipboard") {
        let _ = view.emit("clipboard_capture_error", error);
    }
}

pub(crate) fn image_root(app: &AppHandle) -> Result<PathBuf, String> {
    crate::storage_location::active_data_dir(app)
        .map(|root| root.join("resources").join("clipboard-images"))
}

#[cfg(any(
    test,
    all(
        feature = "ui-tests",
        target_os = "macos",
        not(feature = "portable-ui")
    )
))]
pub(crate) fn persist_capture(
    database: &mut HistoryDatabase,
    root: &Path,
    capture: platform::ClipboardCapture,
) -> Result<(), String> {
    persist_capture_with(
        root,
        capture,
        RecordingTypes::default(),
        |record, representation, formatting, directory_paths| {
            database
                .clipboard_insert_capture_with_dirs(
                    record,
                    representation,
                    formatting,
                    directory_paths,
                )
                .map(|_| ())
        },
    )
    .map(|_| ())
}

fn persist_capture_with(
    root: &Path,
    capture: platform::ClipboardCapture,
    types: RecordingTypes,
    insert: impl FnOnce(
        &ClipboardRecord,
        Option<&ClipboardFileRepresentation>,
        Option<&ClipboardTextFormatting>,
        Option<&[String]>,
    ) -> Result<(), String>,
) -> Result<bool, String> {
    let mut created_image = None;
    let representation = None;
    let mut formatting = None;
    let mut directory_paths = None;
    let (kind, text, image_path, fingerprint) = match capture.payload {
        platform::ClipboardPayload::Text(text) => {
            if !types.text {
                return Ok(false);
            }
            let fingerprint = clipboard::fingerprint("text", text.as_bytes());
            ("text", text, None, fingerprint)
        }
        platform::ClipboardPayload::RichText {
            text,
            formatting: rich,
        } => {
            if !types.text {
                return Ok(false);
            }
            let fingerprint = rich.fingerprint(&text);
            formatting = Some(rich);
            ("text", text, None, fingerprint)
        }
        platform::ClipboardPayload::Files(paths)
        | platform::ClipboardPayload::FilesImage { paths, .. } => {
            if !types.files {
                return Ok(false);
            }
            let text = serde_json::to_string(&paths).map_err(|_| "storage_unavailable")?;
            directory_paths = Some(clipboard::detect_directory_paths(&paths));
            let fingerprint = clipboard::fingerprint("files", text.as_bytes());
            ("files", text, None, fingerprint)
        }
        platform::ClipboardPayload::Image { bytes, extension } => {
            if !types.image {
                return Ok(false);
            }
            let fingerprint = clipboard::fingerprint("image", &bytes);
            let (path, created) = clipboard::store_image_with_status(root, &bytes, &extension)?;
            if created {
                created_image = Some(path.clone());
            }
            ("image", String::new(), Some(path), fingerprint)
        }
    };
    let record = ClipboardRecord {
        id: uuid::Uuid::new_v4().to_string(),
        kind: kind.into(),
        text,
        date: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        is_favorite: false,
        is_pinned: false,
        source_app: capture.source_app,
        image_path,
        fingerprint,
    };
    let saved = insert(
        &record,
        representation.as_ref(),
        formatting.as_ref(),
        directory_paths.as_deref(),
    );
    if saved.is_err() {
        if let Some(path) = created_image {
            clipboard::remove_image(root, &path)?;
        }
    }
    saved.map(|_| true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recopy_queues_only_unindexed_images() {
        for text in ["", "Indexed text"] {
            let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
            let mut image = ClipboardRecord {
                id: "original".into(),
                kind: "image".into(),
                text: "Image".into(),
                date: "2026-11-20T00:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: None,
                image_path: Some("fixture.png".into()),
                fingerprint: "fixture".into(),
            };
            let original = db.clipboard_insert_capture(&image, None, None).unwrap();
            assert_eq!(pending_image_index(&db, original, false).unwrap(), None);
            image.id = "recopy".into();
            let recopy = db.clipboard_insert_capture(&image, None, None).unwrap();
            assert_eq!(
                pending_image_index(&db, recopy.clone(), true).unwrap(),
                Some("original".into())
            );
            assert_eq!(
                db.clipboard_page("", false, None, 100)
                    .unwrap()
                    .total_records,
                1
            );
            db.clipboard_index_image("original", "fixture", text)
                .unwrap();
            assert_eq!(
                pending_image_index(&db, recopy.clone(), true).unwrap(),
                None
            );
            assert_eq!(
                db.clipboard_ocr_text("original").unwrap().as_deref(),
                Some(text)
            );
        }
    }

    #[test]
    fn recording_types_skip_unselected_items_before_saving() {
        let root = Path::new("no-media-write-expected");
        let cases = [
            (
                platform::ClipboardPayload::Text("synthetic text".into()),
                RecordingTypes {
                    text: false,
                    ..Default::default()
                },
            ),
            (
                platform::ClipboardPayload::Image {
                    bytes: vec![1, 2, 3],
                    extension: "png".into(),
                },
                RecordingTypes {
                    image: false,
                    ..Default::default()
                },
            ),
            (
                platform::ClipboardPayload::Files(vec!["/nami-synthetic.txt".into()]),
                RecordingTypes {
                    files: false,
                    ..Default::default()
                },
            ),
        ];
        for (payload, types) in cases {
            let capture = platform::ClipboardCapture {
                version: 1,
                payload,
                source_app: None,
            };
            assert!(
                !persist_capture_with(root, capture, types, |_, _, _, _| panic!(
                    "filtered copy was saved"
                ))
                .unwrap()
            );
        }
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn recording_types_keep_local_image_files_in_files_and_only_raw_images_in_image() {
        let root =
            std::env::temp_dir().join(format!("nami-recording-kinds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../tests/fixtures/clipboard-wide.png");
        let picture = root.join("picture.png");
        std::fs::write(&picture, bytes).unwrap();
        let picture = picture.to_string_lossy().into_owned();
        let file_capture = |paths| platform::ClipboardCapture {
            version: 1,
            source_app: None,
            payload: platform::ClipboardPayload::FilesImage {
                paths,
                bytes: bytes.to_vec(),
                extension: "png".into(),
                image_fallback: false,
            },
        };
        let raw_image_capture = || platform::ClipboardCapture {
            version: 2,
            source_app: None,
            payload: platform::ClipboardPayload::Image {
                bytes: bytes.to_vec(),
                extension: "png".into(),
            },
        };
        let files_only = RecordingTypes {
            image: false,
            ..Default::default()
        };
        let images_only = RecordingTypes {
            files: false,
            ..Default::default()
        };
        let store = root.join("images");
        // Local image files follow the files toggle and never write into images/origin/.
        assert!(persist_capture_with(
            &store,
            file_capture(vec![picture.clone()]),
            files_only,
            |record, representation, formatting, _| {
                assert_eq!(record.kind, "files");
                assert!(record.image_path.is_none());
                assert!(formatting.is_none());
                assert!(representation.is_none());
                Ok(())
            }
        )
        .unwrap());
        assert!(
            !store.exists(),
            "local image file capture must not copy bytes into private image storage"
        );
        assert!(!persist_capture_with(
            &store,
            file_capture(vec![picture]),
            images_only,
            |_, _, _, _| panic!("local file must not be saved when files recording is disabled")
        )
        .unwrap());
        assert!(!persist_capture_with(
            &store,
            raw_image_capture(),
            files_only,
            |_, _, _, _| panic!(
                "raw clipboard image must not be saved when image recording is disabled"
            )
        )
        .unwrap());
        assert!(persist_capture_with(
            &store,
            raw_image_capture(),
            images_only,
            |record, representation, formatting, _| {
                assert_eq!(record.kind, "image");
                assert!(record.image_path.is_some());
                assert!(formatting.is_none());
                assert!(representation.is_none());
                Ok(())
            }
        )
        .unwrap());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn local_image_files_stay_path_references_and_fail_cleanly_when_deleted() {
        let root = std::env::temp_dir().join(format!("nami-image-kind-{}", uuid::Uuid::new_v4()));
        let store = root.join("images");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("photo.jpg");
        let jpeg = include_bytes!("../../tests/fixtures/clipboard-wide.jpg");
        std::fs::write(&source, jpeg).unwrap();
        let paths = vec![source.to_string_lossy().into_owned()];
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for payload in [
            platform::ClipboardPayload::Files(paths.clone()),
            platform::ClipboardPayload::FilesImage {
                paths: paths.clone(),
                bytes: include_bytes!("../icons/icon.png").to_vec(),
                extension: "png".into(),
                image_fallback: false,
            },
        ] {
            persist_capture(
                &mut database,
                &store,
                platform::ClipboardCapture {
                    version: 1,
                    payload,
                    source_app: None,
                },
            )
            .unwrap();
            let page = database
                .clipboard_page_filtered("", false, Some("files"), None, 100)
                .unwrap();
            assert_eq!(page.total_records, 1);
            assert_eq!(
                database
                    .clipboard_page_filtered("", false, Some("image"), None, 100)
                    .unwrap()
                    .total_records,
                0
            );
            let id = &page.records[0].id;
            let record = database.clipboard_record(id).unwrap();
            assert_eq!(record.kind, "files");
            assert!(record.image_path.is_none());
            assert!(database
                .clipboard_file_representation(id)
                .unwrap()
                .is_none());
            assert!(
                !store.exists(),
                "local image file capture must not create images/origin files"
            );
            assert_eq!(
                record_payload(&store, record, None).unwrap(),
                platform::ClipboardPayload::Files(paths.clone())
            );
        }

        let id = database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records[0]
            .id
            .clone();
        // While the original file exists: detail reports original size & dimensions, drag uses original file path.
        let live_detail = crate::clipboard_detail::enrich(
            database.clipboard_detail(&id).unwrap(),
            &store,
            true,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(live_detail.files.len(), 1);
        assert!(live_detail.files[0].exists);
        assert!(live_detail.files[0].is_image);
        assert_eq!(live_detail.size, Some(jpeg.len() as u64));
        assert_eq!(
            (live_detail.width, live_detail.height),
            platform::image_dimensions(jpeg)
                .map(|(w, h)| (Some(w), Some(h)))
                .unwrap()
        );
        let live_drag = prepare_drag_payload(
            &root.join("cache"),
            record_payload(&store, database.clipboard_record(&id).unwrap(), None).unwrap(),
        )
        .unwrap();
        assert_eq!(
            live_drag,
            platform::DragItem::Files {
                paths: vec![source.clone()],
                preview_image: Some(source.clone()),
            }
        );

        // Once the original file is removed/moved: detail cleanly drops image metadata & drag fails with missing.
        std::fs::remove_file(&source).unwrap();
        let missing_detail = crate::clipboard_detail::enrich(
            database.clipboard_detail(&id).unwrap(),
            &store,
            true,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(missing_detail.files.len(), 1);
        assert!(!missing_detail.files[0].exists);
        assert_eq!(
            (
                missing_detail.width,
                missing_detail.height,
                missing_detail.size
            ),
            (None, None, None)
        );
        assert_eq!(
            prepare_drag_payload(
                &root.join("cache"),
                record_payload(&store, database.clipboard_record(&id).unwrap(), None).unwrap(),
            )
            .unwrap_err(),
            "clipboard_file_missing"
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn oversized_local_image_files_capture_without_decoding_and_skip_dimensions() {
        let root = std::env::temp_dir().join(format!("nami-image-bounds-{}", uuid::Uuid::new_v4()));
        let store = root.join("images");
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("large.png");
        let png = include_bytes!("../../tests/fixtures/clipboard-oversized.png");
        assert_eq!(platform::image_dimensions(png).unwrap(), (8193, 8193));
        std::fs::write(&source, png).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(clipboard::MAX_IMAGE_BYTES as u64 + 1)
            .unwrap();
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        persist_capture(
            &mut database,
            &store,
            platform::ClipboardCapture {
                version: 1,
                payload: platform::ClipboardPayload::Files(vec![source
                    .to_string_lossy()
                    .into_owned()]),
                source_app: None,
            },
        )
        .unwrap();
        assert!(!store.exists());
        let id = database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records[0]
            .id
            .clone();
        let detail = crate::clipboard_detail::enrich(
            database.clipboard_detail(&id).unwrap(),
            &store,
            true,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(detail.kind, "files");
        assert_eq!((detail.width, detail.height), (None, None));
        assert_eq!(detail.size, Some(clipboard::MAX_IMAGE_BYTES as u64 + 1));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn raw_non_png_image_payload_is_stored_verbatim_and_converts_to_png_on_paste_and_export() {
        let root = std::env::temp_dir().join(format!("nami-jpeg-kind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let jpeg = include_bytes!("../../tests/fixtures/clipboard-wide.jpg");
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        persist_capture(
            &mut database,
            &root,
            platform::ClipboardCapture {
                version: 1,
                payload: platform::ClipboardPayload::Image {
                    bytes: jpeg.to_vec(),
                    extension: "jpg".into(),
                },
                source_app: None,
            },
        )
        .unwrap();
        let page = database
            .clipboard_page_filtered("", false, Some("image"), None, 100)
            .unwrap();
        assert_eq!(page.total_records, 1);
        let record = database.clipboard_record(&page.records[0].id).unwrap();
        let image_path = record.image_path.as_deref().unwrap();
        assert!(
            image_path.ends_with(".jpg"),
            "non-PNG image should be stored verbatim without PNG expansion, got {image_path}"
        );
        let stored = clipboard::read_image(&root, image_path).unwrap();
        assert_eq!(stored.as_slice(), jpeg);
        let detail = crate::clipboard_detail::enrich(
            database.clipboard_detail(&record.id).unwrap(),
            &root,
            false,
            None,
            &[],
        )
        .unwrap();
        assert_eq!(detail.size, Some(jpeg.len() as u64));
        assert_eq!(
            (detail.width, detail.height),
            platform::image_dimensions(jpeg)
                .map(|(w, h)| (Some(w), Some(h)))
                .unwrap()
        );

        let stored_payload = record_payload(&root, record, None).unwrap();
        let exported = export_png(stored_payload.clone()).unwrap();
        assert!(exported.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            platform::image_dimensions(&exported).unwrap(),
            platform::image_dimensions(jpeg).unwrap()
        );

        let drag_cache = root.join("cache");
        let drag = prepare_drag_payload(&drag_cache, stored_payload).unwrap();
        match drag {
            platform::DragItem::Files {
                paths,
                preview_image,
            } => {
                assert_eq!(paths.len(), 1);
                assert_eq!(preview_image.as_ref(), Some(&paths[0]));
                let drag_bytes = std::fs::read(&paths[0]).unwrap();
                assert!(drag_bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            }
            other => panic!("expected dragged image file, got {other:?}"),
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    #[ignore = "explicit synthetic image capture and durable-write performance measurement"]
    fn benchmark_image_file_capture() {
        let root =
            std::env::temp_dir().join(format!("nami-image-performance-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("screenshot.png");
        let bytes = include_bytes!("../../tests/fixtures/clipboard-wide.png");
        std::fs::write(&source, bytes).unwrap();
        let mut database = HistoryDatabase::open(&root.join("history.sqlite3")).unwrap();
        let mut samples = Vec::new();
        for version in 0..31 {
            let started = std::time::Instant::now();
            persist_capture(
                &mut database,
                &root,
                platform::ClipboardCapture {
                    version,
                    payload: platform::ClipboardPayload::Files(vec![source
                        .to_string_lossy()
                        .into_owned()]),
                    source_app: None,
                },
            )
            .unwrap();
            samples.push(started.elapsed().as_secs_f64() * 1000.0);
        }
        let first = samples.remove(0);
        samples.sort_by(f64::total_cmp);
        let report = serde_json::json!({"scope":"synthetic 2048x256 PNG file; path-only files capture and SQLite FULL commit; no UI or polling interval", "firstMs":first,"p50Ms":samples[15],"p95Ms":samples[28],"samples":samples.len(),"imageBytes":bytes.len()});
        std::fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/clipboard-image-capture-benchmark.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{report}");
        assert_eq!(
            database
                .clipboard_page_filtered("", false, Some("files"), None, 100)
                .unwrap()
                .total_records,
            1
        );
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    #[ignore = "explicit synthetic JPEG path-only capture vs verbatim copy vs legacy PNG normalization measurement"]
    fn benchmark_jpeg_verbatim_vs_png_normalization() {
        let root =
            std::env::temp_dir().join(format!("nami-jpeg-benchmark-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();

        // Synthesize a deterministic 2560x1440 photographic-texture BMP and encode it as JPEG (q=85).
        let width: u32 = 2560;
        let height: u32 = 1440;
        let row_bytes = (width as usize) * 3;
        let pixel_bytes = row_bytes * (height as usize);
        let file_size = 54 + pixel_bytes;
        let mut bmp = Vec::with_capacity(file_size);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&(file_size as u32).to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(width as i32).to_le_bytes());
        bmp.extend_from_slice(&(height as i32).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&24u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&(pixel_bytes as u32).to_le_bytes());
        bmp.extend_from_slice(&2835i32.to_le_bytes());
        bmp.extend_from_slice(&2835i32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        for y in 0..height {
            for x in 0..width {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                let grain = (state & 0x1f) as u8;
                let r = (((x * 255) / width) as u8).saturating_add(grain);
                let g = (((y * 255) / height) as u8).saturating_add(grain >> 1);
                let b = ((((x ^ y) & 0xff) as u8) / 2).saturating_add(grain);
                bmp.push(b);
                bmp.push(g);
                bmp.push(r);
            }
        }
        let bmp_path = root.join("synthetic-2560x1440.bmp");
        let jpg_path = root.join("synthetic-2560x1440.jpg");
        std::fs::write(&bmp_path, &bmp).unwrap();
        let status = std::process::Command::new("sips")
            .args([
                "-s",
                "format",
                "jpeg",
                "-s",
                "formatOptions",
                "85",
                bmp_path.to_str().unwrap(),
                "--out",
                jpg_path.to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(status.status.success(), "sips jpeg encoding failed");
        let jpg_bytes = std::fs::read(&jpg_path).unwrap();

        let mut legacy_ms = Vec::new();
        let mut verbatim_ms = Vec::new();
        let mut path_only_ms = Vec::new();
        let mut legacy_png_len = 0usize;
        for round in 0..11 {
            let legacy_root = root.join(format!("legacy-{round}"));
            let t0 = std::time::Instant::now();
            let raw = clipboard::read_file_preview(&jpg_path).unwrap();
            let (w, h) = platform::image_dimensions(&raw).unwrap();
            let png = platform::clipboard_image_preview(&raw, w.max(h)).unwrap();
            legacy_png_len = png.len();
            let _ = clipboard::store_image(&legacy_root, &png, "png").unwrap();
            legacy_ms.push(t0.elapsed().as_secs_f64() * 1000.0);

            let verbatim_root = root.join(format!("verbatim-{round}"));
            let t1 = std::time::Instant::now();
            let raw = clipboard::read_file_preview(&jpg_path).unwrap();
            let _ = platform::image_dimensions(&raw).unwrap();
            let _ = clipboard::store_image(&verbatim_root, &raw, "jpg").unwrap();
            verbatim_ms.push(t1.elapsed().as_secs_f64() * 1000.0);

            let path_root = root.join(format!("path-only-{round}"));
            let mut db =
                HistoryDatabase::open(&root.join(format!("path-only-{round}.sqlite3"))).unwrap();
            let t2 = std::time::Instant::now();
            persist_capture(
                &mut db,
                &path_root,
                platform::ClipboardCapture {
                    version: round as i64,
                    payload: platform::ClipboardPayload::Files(vec![jpg_path
                        .to_string_lossy()
                        .into_owned()]),
                    source_app: None,
                },
            )
            .unwrap();
            path_only_ms.push(t2.elapsed().as_secs_f64() * 1000.0);
        }
        legacy_ms.remove(0);
        verbatim_ms.remove(0);
        path_only_ms.remove(0);
        legacy_ms.sort_by(f64::total_cmp);
        verbatim_ms.sort_by(f64::total_cmp);
        path_only_ms.sort_by(f64::total_cmp);
        let report = serde_json::json!({
            "dimensions": [width, height],
            "jpegBytes": jpg_bytes.len(),
            "legacyPngBytes": legacy_png_len,
            "pathOnlyPrivateBytes": 0,
            "sizeRatio": (legacy_png_len as f64) / (jpg_bytes.len() as f64),
            "legacyMedianMs": legacy_ms[legacy_ms.len() / 2],
            "verbatimMedianMs": verbatim_ms[verbatim_ms.len() / 2],
            "pathOnlyMedianMs": path_only_ms[path_only_ms.len() / 2],
            "rounds": legacy_ms.len(),
        });
        println!("{report}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn file_icons_multiple_images_and_text_paths_do_not_override_file_semantics() {
        let root = std::env::temp_dir().join(format!("nami-file-kind-{}", uuid::Uuid::new_v4()));
        let store = root.join("images");
        std::fs::create_dir_all(&root).unwrap();
        let ordinary = root.join("not-an-image.png");
        let picture = root.join("actual-image.txt");
        std::fs::write(&ordinary, b"a synthetic ordinary file").unwrap();
        std::fs::write(
            &picture,
            include_bytes!("../../tests/fixtures/clipboard-wide.png"),
        )
        .unwrap();
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for paths in [
            vec![ordinary.clone()],
            vec![root.clone()],
            vec![picture.clone(), ordinary.clone()],
            vec![picture.clone(), picture.clone()],
        ] {
            let paths: Vec<String> = paths
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned())
                .collect();
            persist_capture(
                &mut database,
                &store,
                platform::ClipboardCapture {
                    version: 1,
                    payload: platform::ClipboardPayload::FilesImage {
                        paths: paths.clone(),
                        bytes: include_bytes!("../icons/icon.png").to_vec(),
                        extension: "png".into(),
                        image_fallback: false,
                    },
                    source_app: None,
                },
            )
            .unwrap();
            let page = database.clipboard_page("", false, None, 100).unwrap();
            let id = &page.records[0].id;
            let record = database.clipboard_record(id).unwrap();
            assert_eq!(record.kind, "files");
            assert_eq!(
                serde_json::from_str::<Vec<String>>(&record.text).unwrap(),
                paths
            );
            assert!(database
                .clipboard_file_representation(id)
                .unwrap()
                .is_none());
            assert_eq!(
                record_payload(&store, record, None).unwrap(),
                platform::ClipboardPayload::Files(paths)
            );
        }
        assert!(!store.exists());
        persist_capture(
            &mut database,
            &store,
            platform::ClipboardCapture {
                version: 1,
                payload: platform::ClipboardPayload::Text(picture.to_string_lossy().into_owned()),
                source_app: None,
            },
        )
        .unwrap();
        assert_eq!(
            database
                .clipboard_page_filtered("", false, Some("text"), None, 100)
                .unwrap()
                .total_records,
            1
        );
        assert_eq!(
            database
                .clipboard_page_filtered("", false, Some("image"), None, 100)
                .unwrap()
                .total_records,
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn raw_image_preparation_stays_outside_sql_and_cleans_up_after_failed_commit() {
        let root =
            std::env::temp_dir().join(format!("nami-image-rollback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../tests/fixtures/clipboard-wide.png").to_vec();
        let database = std::sync::Mutex::new(HistoryDatabase::open(Path::new(":memory:")).unwrap());
        let snapshot = std::cell::RefCell::new(String::new());
        let result = persist_capture_with(
            &root,
            platform::ClipboardCapture {
                version: 1,
                payload: platform::ClipboardPayload::Image {
                    bytes,
                    extension: "png".into(),
                },
                source_app: None,
            },
            RecordingTypes::default(),
            |record, representation, _formatting, _dirs| {
                let _guard = database
                    .try_lock()
                    .expect("file IO and image storage must precede the SQL lock");
                assert_eq!(record.kind, "image");
                assert!(representation.is_none());
                let path = record.image_path.clone().unwrap();
                assert!(root.join(&path).is_file());
                *snapshot.borrow_mut() = path;
                Err("storage_busy".into())
            },
        );
        assert_eq!(result.unwrap_err(), "storage_busy");
        assert!(!root.join(snapshot.into_inner()).exists());
        assert!(database
            .lock()
            .unwrap()
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records
            .is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn synthetic_capture(version: i64) -> platform::ClipboardCapture {
        platform::ClipboardCapture {
            version,
            payload: platform::ClipboardPayload::Text("Synthetic new copy".into()),
            source_app: None,
        }
    }

    #[test]
    fn read_failure_remains_pending_for_the_same_version() {
        let handled = AtomicI64::new(7);
        assert!(observe_pending(&handled, 8, false, || Err("clipboard_read")).is_err());
        let next = observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8)))).unwrap();
        assert!(
            next.is_some(),
            "a transient read failure must not consume the copy"
        );
    }

    #[test]
    fn persistence_failure_leaves_capture_pending() {
        let handled = AtomicI64::new(7);
        let first = observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8)))).unwrap();
        assert!(first.is_some());
        assert_eq!(
            handled.load(Ordering::SeqCst),
            7,
            "reading is not a database commit"
        );
        assert!(
            observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8))))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn temporary_selection_copy_is_not_read_or_acknowledged() {
        let handled = AtomicI64::new(7);
        assert!(observe_pending(&handled, 8, true, || panic!(
            "temporary selection must not be captured"
        ))
        .unwrap()
        .is_none());
        assert_eq!(handled.load(Ordering::SeqCst), 7);
        assert!(
            observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8))))
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn ax_busy_does_not_pause_an_external_copy_or_consume_it_before_commit() {
        let state = AppState::default();
        state.selection_busy.store(true, Ordering::SeqCst);
        state.clipboard_version.store(7, Ordering::SeqCst);
        let (baseline, capture) = observe_pending(&state.clipboard_version, 8, false, || {
            Ok(Some(synthetic_capture(8)))
        })
        .unwrap()
        .expect("AX reads cannot block an independent external copy");
        assert_eq!(state.clipboard_version.load(Ordering::SeqCst), 7);
        assert!(state.selection_busy.load(Ordering::SeqCst));
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        persist_capture(&mut database, Path::new("unused-for-text-capture"), capture).unwrap();
        acknowledge(&state.clipboard_version, baseline, 8);
        assert_eq!(state.clipboard_version.load(Ordering::SeqCst), 8);
        assert!(state.selection_busy.load(Ordering::SeqCst));
        let records = database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records;
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].text, "Synthetic new copy");
    }

    #[test]
    fn ax_only_activity_neither_reads_nor_acknowledges_an_unchanged_clipboard() {
        let state = AppState::default();
        state.clipboard_version.store(7, Ordering::SeqCst);
        for busy in [true, false] {
            state.selection_busy.store(busy, Ordering::SeqCst);
            assert!(
                observe_pending(&state.clipboard_version, 7, false, || panic!(
                    "AX activity did not change the clipboard"
                ))
                .unwrap()
                .is_none()
            );
            assert_eq!(state.clipboard_version.load(Ordering::SeqCst), 7);
        }
    }

    #[test]
    fn permanent_capture_errors_are_reported_once_without_rereading_the_payload() {
        for error in [
            "clipboard_capture_limit",
            "clipboard_text_encoding",
            "clipboard_file_url",
        ] {
            let handled = AtomicI64::new(7);
            assert!(observe_pending(&handled, 8, false, || Err(error)).is_err());
            assert_eq!(handled.load(Ordering::SeqCst), 8);
            assert!(observe_pending(&handled, 8, false, || panic!(
                "permanent payload failure must not loop"
            ))
            .unwrap()
            .is_none());
        }
        assert!(!retryable("clipboard_image_format"));
        assert!(!retryable("clipboard_image_limit"));
        assert!(retryable("storage_busy"));
        assert!(retryable("storage_unavailable"));
    }

    #[test]
    fn a_busy_database_recovers_the_same_copy_without_another_version_change() {
        let root =
            std::env::temp_dir().join(format!("nami-capture-retry-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.sqlite3");
        let mut database = HistoryDatabase::open(&path).unwrap();
        let lock = rusqlite::Connection::open(&path).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let handled = AtomicI64::new(7);
        let (_, capture) = observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8))))
            .unwrap()
            .unwrap();
        assert_eq!(
            persist_capture(&mut database, &root, capture).unwrap_err(),
            "storage_busy"
        );
        assert_eq!(handled.load(Ordering::SeqCst), 7);
        lock.execute_batch("ROLLBACK").unwrap();
        let (baseline, capture) =
            observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8))))
                .unwrap()
                .unwrap();
        let version = capture.version;
        persist_capture(&mut database, &root, capture).unwrap();
        acknowledge(&handled, baseline, version);
        assert_eq!(handled.load(Ordering::SeqCst), 8);
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .records
                .len(),
            1
        );
        drop(lock);
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_commit_acknowledges_only_its_baseline() {
        let handled = AtomicI64::new(7);
        let (baseline, capture) =
            observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8))))
                .unwrap()
                .unwrap();
        acknowledge(&handled, baseline, capture.version);
        assert_eq!(handled.load(Ordering::SeqCst), 8);
        assert!(observe_pending(&handled, 8, false, || panic!(
            "committed copy must not be read again"
        ))
        .unwrap()
        .is_none());
        handled.store(10, Ordering::SeqCst);
        acknowledge(&handled, baseline, capture.version);
        assert_eq!(
            handled.load(Ordering::SeqCst),
            10,
            "an in-flight save must not undo app-write suppression"
        );
    }

    #[test]
    fn excluded_content_is_acknowledged_without_persistence() {
        let handled = AtomicI64::new(7);
        assert!(observe_pending(&handled, 8, false, || Ok(None))
            .unwrap()
            .is_none());
        assert_eq!(handled.load(Ordering::SeqCst), 8);
    }

    #[test]
    fn image_preparation_and_failed_commit_cleanup_surround_only_the_database_insert() {
        let root = std::env::temp_dir().join(format!("nami-capture-lock-{}", uuid::Uuid::new_v4()));
        let database = std::sync::Mutex::new(HistoryDatabase::open(Path::new(":memory:")).unwrap());
        let bytes = b"\x89PNG\r\n\x1a\nprepared outside SQL".to_vec();
        let capture = || platform::ClipboardCapture {
            version: 1,
            payload: platform::ClipboardPayload::Image {
                bytes: bytes.clone(),
                extension: "png".into(),
            },
            source_app: None,
        };
        let hash = clipboard::fingerprint("image", &bytes);
        let relative = format!("origin/{}/{}.png", &hash[..2], hash);
        let saved = std::cell::RefCell::new(None);
        persist_capture_with(
            &root,
            capture(),
            RecordingTypes::default(),
            |record, representation, formatting, directory_paths| {
                assert_eq!(clipboard::read_image(&root, &relative).unwrap(), bytes);
                let mut guard = database
                    .try_lock()
                    .expect("preparation must not hold the SQL lock");
                *saved.borrow_mut() = Some(guard.clipboard_insert_capture_with_dirs(
                    record,
                    representation,
                    formatting,
                    directory_paths,
                )?);
                Ok(())
            },
        )
        .unwrap();
        let record = saved.into_inner().unwrap();
        let guard = database.lock().unwrap();
        assert_eq!(
            record_payload(&root, record, None).unwrap(),
            platform::ClipboardPayload::Image {
                bytes: bytes.clone(),
                extension: "png".into()
            }
        );
        drop(guard);
        let fresh = b"\x89PNG\r\n\x1a\nuncommitted outside SQL".to_vec();
        let fresh_hash = clipboard::fingerprint("image", &fresh);
        let fresh_path = format!("origin/{}/{}.png", &fresh_hash[..2], fresh_hash);
        let result = persist_capture_with(
            &root,
            platform::ClipboardCapture {
                version: 2,
                payload: platform::ClipboardPayload::Image {
                    bytes: fresh,
                    extension: "png".into(),
                },
                source_app: None,
            },
            RecordingTypes::default(),
            |record, _representation, _formatting, _dirs| {
                let guard = database
                    .try_lock()
                    .expect("resource preparation must release the SQL lock");
                assert!(root.join(record.image_path.as_deref().unwrap()).is_file());
                drop(guard);
                Err("storage_busy".into())
            },
        );
        assert_eq!(result.unwrap_err(), "storage_busy");
        assert!(!root.join(fresh_path).exists());
        assert_eq!(clipboard::read_image(&root, &relative).unwrap(), bytes);
        assert_eq!(
            database
                .try_lock()
                .unwrap()
                .clipboard_page("", false, None, 100)
                .unwrap()
                .records
                .len(),
            1
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn failed_capture_removes_only_new_images() {
        let root =
            std::env::temp_dir().join(format!("nami-capture-failure-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("test.sqlite3");
        let mut database = HistoryDatabase::open(&path).unwrap();
        let existing = b"\x89PNG\r\n\x1a\nexisting".to_vec();
        let capture = |bytes: Vec<u8>| platform::ClipboardCapture {
            version: 1,
            payload: platform::ClipboardPayload::Image {
                bytes,
                extension: "png".into(),
            },
            source_app: None,
        };
        persist_capture(&mut database, &root, capture(existing.clone())).unwrap();
        let existing_id = database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records[0]
            .id
            .clone();
        let existing_path = database
            .clipboard_record(&existing_id)
            .unwrap()
            .image_path
            .unwrap();
        let lock = rusqlite::Connection::open(&path).unwrap();
        lock.execute_batch("BEGIN IMMEDIATE").unwrap();
        let fresh = b"\x89PNG\r\n\x1a\nnew".to_vec();
        let hash = clipboard::fingerprint("image", &fresh);
        assert_eq!(
            persist_capture(&mut database, &root, capture(fresh)).unwrap_err(),
            "storage_busy"
        );
        assert!(!root
            .join(format!("origin/{}/{}.png", &hash[..2], hash))
            .exists());
        assert_eq!(
            persist_capture(&mut database, &root, capture(existing.clone())).unwrap_err(),
            "storage_busy"
        );
        assert_eq!(
            clipboard::read_image(&root, &existing_path).unwrap(),
            existing
        );
        lock.execute_batch("ROLLBACK").unwrap();
        drop(lock);
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn synthetic_capture_persists_each_kind_and_preserves_file_paths() {
        let root = std::env::temp_dir().join(format!("nami-capture-{}", uuid::Uuid::new_v4()));
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let paths = vec![
            root.join("space # %.txt").to_string_lossy().into_owned(),
            root.join("new\nline").to_string_lossy().into_owned(),
        ];
        assert!(paths.iter().all(|path| Path::new(path).is_absolute()));
        let png = b"\x89PNG\r\n\x1a\nsynthetic".to_vec();
        for payload in [
            platform::ClipboardPayload::Text("full synthetic text".into()),
            platform::ClipboardPayload::Files(paths.clone()),
            platform::ClipboardPayload::Image {
                bytes: png.clone(),
                extension: "png".into(),
            },
        ] {
            persist_capture(
                &mut database,
                &root,
                platform::ClipboardCapture {
                    version: 1,
                    payload,
                    source_app: None,
                },
            )
            .unwrap();
        }
        let page = database.clipboard_page("", false, None, 100).unwrap();
        assert_eq!(page.records.len(), 3);
        let files = page
            .records
            .iter()
            .find(|record| record.kind == "files")
            .unwrap();
        assert_eq!(
            serde_json::from_str::<Vec<String>>(
                &database.clipboard_record(&files.id).unwrap().text
            )
            .unwrap(),
            paths
        );
        let image = page
            .records
            .iter()
            .find(|record| record.kind == "image")
            .unwrap();
        assert_eq!(
            record_payload(&root, database.clipboard_record(&image.id).unwrap(), None)
                .map(|payload| match payload {
                    platform::ClipboardPayload::Image { bytes, .. } => bytes,
                    _ => panic!("expected image"),
                })
                .unwrap(),
            png
        );
        persist_capture(
            &mut database,
            &root,
            platform::ClipboardCapture {
                version: 2,
                payload: platform::ClipboardPayload::Text("full synthetic text".into()),
                source_app: None,
            },
        )
        .unwrap();
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .records
                .len(),
            3
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn text_regex_ignore_skips_plain_and_rich_text_without_affecting_media() {
        let regex = nami::clipboard::compile_ignored_regex("^tok-[0-9]{4}$").unwrap();
        let matches = |text: &str| regex.is_match(text);
        assert!(payload_ignored_by_text_rules(
            &platform::ClipboardPayload::Text("tok-1234".into()),
            matches
        ));
        assert!(payload_ignored_by_text_rules(
            &platform::ClipboardPayload::RichText {
                text: "tok-5678".into(),
                formatting: ClipboardTextFormatting {
                    html: Some(b"<b>tok-5678</b>".to_vec()),
                    rtf: None,
                },
            },
            matches
        ));
        assert!(!payload_ignored_by_text_rules(
            &platform::ClipboardPayload::Text("hello".into()),
            matches
        ));
        assert!(!payload_ignored_by_text_rules(
            &platform::ClipboardPayload::Files(vec!["/tmp/tok-1234".into()]),
            matches
        ));
    }

    #[test]
    fn file_paths_payload_supports_files_and_local_image_representations_only() {
        let root = std::env::temp_dir();
        let photo = root.join("photo.png").to_string_lossy().into_owned();
        let first = root.join("a.txt").to_string_lossy().into_owned();
        let second = root.join("b.txt").to_string_lossy().into_owned();
        let local_image = ClipboardRecord {
            id: "local-img".into(),
            kind: "image".into(),
            text: String::new(),
            date: "2026-11-25T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: Some("clipboard-images/local.png".into()),
            fingerprint: "fp-local-img".into(),
        };
        let representation = ClipboardFileRepresentation {
            paths: vec![photo.clone()],
            image_path: "clipboard-images/local.png".into(),
        };
        assert_eq!(
            file_paths_payload(&local_image, Some(&representation)).unwrap(),
            platform::ClipboardPayload::Text(photo.clone())
        );
        assert_eq!(
            file_paths_payload(&local_image, None).unwrap_err(),
            "invalid_argument"
        );
        let text_record = ClipboardRecord {
            kind: "text".into(),
            text: photo,
            ..local_image.clone()
        };
        assert_eq!(
            file_paths_payload(&text_record, None).unwrap_err(),
            "invalid_argument"
        );
        let files_record = ClipboardRecord {
            kind: "files".into(),
            text: serde_json::to_string(&[&first, &second]).unwrap(),
            image_path: None,
            ..local_image
        };
        assert_eq!(
            file_paths_payload(&files_record, None).unwrap(),
            platform::ClipboardPayload::Text(format!("{first}\n{second}"))
        );
    }

    #[test]
    fn drag_payload_prepares_text_files_images_and_rejects_missing() {
        let root = std::env::temp_dir().join(format!("nami-drag-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let existing = root.join("sample.txt");
        std::fs::write(&existing, b"hello").unwrap();

        assert_eq!(
            prepare_drag_payload(&root, platform::ClipboardPayload::Text("hello drag".into()))
                .unwrap(),
            platform::DragItem::Text("hello drag".into())
        );
        assert_eq!(
            prepare_drag_payload(&root, platform::ClipboardPayload::Text(String::new()))
                .unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            prepare_drag_payload(
                &root,
                platform::ClipboardPayload::Files(vec![existing.to_string_lossy().into_owned()])
            )
            .unwrap(),
            platform::DragItem::Files {
                paths: vec![existing.clone()],
                preview_image: None,
            }
        );
        assert_eq!(
            prepare_drag_payload(
                &root,
                platform::ClipboardPayload::Files(vec![root
                    .join("missing.txt")
                    .to_string_lossy()
                    .into_owned()])
            )
            .unwrap_err(),
            "clipboard_file_missing"
        );

        let png = vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, b'I', b'D', b'A', b'T', 0x08,
            0xD7, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59,
            0xE7, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
        ];
        let image_item = prepare_drag_payload(
            &root,
            platform::ClipboardPayload::Image {
                bytes: png.clone(),
                extension: "png".into(),
            },
        )
        .unwrap();
        match image_item {
            platform::DragItem::Files {
                paths,
                preview_image,
            } => {
                assert_eq!(paths.len(), 1);
                assert_eq!(preview_image.as_deref(), Some(paths[0].as_path()));
                assert_eq!(std::fs::read(&paths[0]).unwrap(), png);
            }
            other => panic!("unexpected drag item: {other:?}"),
        }

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn captured_folder_retains_directory_flag_after_deletion() {
        let root =
            std::env::temp_dir().join(format!("nami-folder-capture-{}", uuid::Uuid::new_v4()));
        let folder = root.join("ArchivedFolder");
        let file = root.join("note.txt");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(&file, b"synthetic").unwrap();
        let folder_path = folder.to_string_lossy().into_owned();
        let file_path = file.to_string_lossy().into_owned();

        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let capture = platform::ClipboardCapture {
            version: 1,
            payload: platform::ClipboardPayload::Files(vec![
                folder_path.clone(),
                file_path.clone(),
            ]),
            source_app: None,
        };
        let saved_id = std::cell::RefCell::new(String::new());
        persist_capture_with(
            &root,
            capture,
            RecordingTypes::default(),
            |record, representation, formatting, directory_paths| {
                let persisted = db.clipboard_insert_capture_with_dirs(
                    record,
                    representation,
                    formatting,
                    directory_paths,
                )?;
                *saved_id.borrow_mut() = persisted.id;
                Ok(())
            },
        )
        .unwrap();
        let id = saved_id.into_inner();
        assert_eq!(
            db.clipboard_directory_paths(&id).unwrap(),
            vec![folder_path.clone()]
        );

        // Delete the folder from disk (simulating Move to Trash / delete).
        std::fs::remove_dir_all(&folder).unwrap();
        let record = db.clipboard_record(&id).unwrap();
        let dirs = db.clipboard_directory_paths(&id).unwrap();
        let entries = clipboard::file_entries_with_dirs(&record.text, &dirs).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].path, folder_path);
        assert!(!entries[0].exists);
        assert!(entries[0].is_dir);
        assert!(entries[1].exists && !entries[1].is_dir);

        let _ = std::fs::remove_dir_all(root);
    }
}
