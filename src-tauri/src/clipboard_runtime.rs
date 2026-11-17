use crate::{commands::on_main, platform, AppState};
use nami::{
    clipboard::{self, ClipboardFileRepresentation, ClipboardRecord},
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

pub(crate) async fn payload(
    app: &AppHandle,
    id: String,
    restore_files: bool,
) -> Result<platform::ClipboardPayload, String> {
    let state = app.state::<AppState>();
    // Keep deletion and capture serialized while file reads run without the database mutex.
    let _write = state.clipboard_writes.lock().await;
    let root = image_root(app)?;
    let (record, representation) = database(app, move |database| {
        Ok((
            database.clipboard_record(&id)?,
            if restore_files {
                database.clipboard_file_representation(&id)?
            } else {
                None
            },
        ))
    })
    .await?;
    tauri::async_runtime::spawn_blocking(move || record_payload(&root, record, representation))
        .await
        .map_err(|_| "storage_worker")?
}

fn record_payload(
    root: &Path,
    record: ClipboardRecord,
    representation: Option<ClipboardFileRepresentation>,
) -> Result<platform::ClipboardPayload, String> {
    if let Some(mut representation) = representation {
        let bytes = clipboard::read_image(root, &representation.image_path)?;
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
            extension: if representation.image_path.ends_with(".png") {
                "png"
            } else {
                "tiff"
            }
            .into(),
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
            let extension = if path.ends_with(".png") {
                "png"
            } else {
                "tiff"
            };
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
    if !platform::capabilities().clipboard_write {
        return;
    }
    let state = app.state::<AppState>();
    state
        .clipboard_version
        .store(platform::clipboard_version(), Ordering::SeqCst);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut failure = None;
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(350)).await;
            if !app
                .state::<AppState>()
                .clipboard_enabled
                .load(Ordering::SeqCst)
            {
                failure = None;
                continue;
            }
            let read = on_main(&app, |app| {
                let state = app.state::<AppState>();
                if !state.clipboard_enabled.load(Ordering::SeqCst) {
                    return Ok(None);
                }
                observe_pending(
                    &state.clipboard_version,
                    platform::clipboard_version(),
                    state.selection_copy_busy.load(Ordering::SeqCst),
                    platform::read_clipboard,
                )
                .map_err(str::to_owned)
            })
            .await;
            let (baseline, capture) = match read {
                Ok(Ok(Some(pending))) => pending,
                Ok(Ok(None)) => continue,
                Ok(Err(error)) | Err(error) => {
                    retry_capture(&app, error, &mut failure).await;
                    continue;
                }
            };
            let state = app.state::<AppState>();
            let _write = state.clipboard_writes.lock().await;
            if !state.clipboard_enabled.load(Ordering::SeqCst) {
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
            let worker = app.clone();
            let result = tauri::async_runtime::spawn_blocking(move || {
                persist_capture_with(&root, capture, |record, representation| {
                    let state = worker.state::<AppState>();
                    if let Some(error) = state.storage_error.lock().unwrap().clone() {
                        return Err(error);
                    }
                    let mut database = state.database.lock().unwrap();
                    database
                        .as_mut()
                        .ok_or("storage_unavailable")?
                        .clipboard_insert_with_file_representation(record, representation)
                        .map(|_| ())
                })
            })
            .await
            .map_err(|_| "storage_worker".to_owned())
            .and_then(|result| result);
            drop(_write);
            match result {
                Ok(()) => {
                    acknowledge(&state.clipboard_version, baseline, captured_version);
                    failure = None;
                    let _ = changed(&app);
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

fn capture_error(app: &AppHandle, error: &str) {
    if let Some(view) = app.get_webview("clipboard") {
        let _ = view.emit("clipboard_capture_error", error);
    }
}

pub(crate) fn image_root(app: &AppHandle) -> Result<PathBuf, String> {
    app.path()
        .app_data_dir()
        .map(|root| root.join("resources").join("clipboard-images"))
        .map_err(|_| "storage_unavailable".into())
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
    persist_capture_with(root, capture, |record, representation| {
        database
            .clipboard_insert_with_file_representation(record, representation)
            .map(|_| ())
    })
}

fn image_file(path: &Path) -> Option<(Vec<u8>, String)> {
    use std::io::Read;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > clipboard::MAX_IMAGE_BYTES as u64 {
        return None;
    }
    // Reject ordinary files with a bounded header read, before loading or decoding their contents.
    let mut header = Vec::with_capacity(32);
    std::fs::File::open(path)
        .ok()?
        .take(32)
        .read_to_end(&mut header)
        .ok()?;
    let raster = header.starts_with(b"\x89PNG\r\n\x1a\n")
        || header.starts_with(b"II*\0")
        || header.starts_with(b"MM\0*")
        || header.starts_with(b"\xff\xd8\xff")
        || header.starts_with(b"GIF87a")
        || header.starts_with(b"GIF89a")
        || header.starts_with(b"BM")
        || header.starts_with(b"\0\0\x01\0")
        || header.starts_with(b"RIFF") && header.get(8..12) == Some(b"WEBP")
        || header.get(4..8) == Some(b"ftyp")
            && header.get(8..12).is_some_and(|brand| {
                [
                    b"heic", b"heix", b"hevc", b"hevx", b"mif1", b"msf1", b"avif", b"avis",
                ]
                .contains(&brand.try_into().unwrap())
            });
    if !raster {
        return None;
    }
    let bytes = clipboard::read_file_preview(path).ok()?;
    let (width, height) = platform::image_dimensions(&bytes).ok()?;
    if u64::from(width) * u64::from(height) > 64 * 1024 * 1024 {
        return None;
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some((bytes, "png".into()))
    } else if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") {
        Some((bytes, "tiff".into()))
    } else {
        platform::clipboard_image_preview(&bytes, width.max(height))
            .ok()
            .filter(|bytes| bytes.len() <= clipboard::MAX_IMAGE_BYTES)
            .map(|bytes| (bytes, "png".into()))
    }
}

fn persist_capture_with(
    root: &Path,
    capture: platform::ClipboardCapture,
    insert: impl FnOnce(&ClipboardRecord, Option<&ClipboardFileRepresentation>) -> Result<(), String>,
) -> Result<(), String> {
    let mut created_image = None;
    let mut representation = None;
    let (kind, text, image_path, fingerprint) = match capture.payload {
        platform::ClipboardPayload::Text(text) => {
            let fingerprint = clipboard::fingerprint("text", text.as_bytes());
            ("text", text, None, fingerprint)
        }
        payload @ (platform::ClipboardPayload::Files(_)
        | platform::ClipboardPayload::FilesImage { .. }) => {
            let (paths, image) = match payload {
                platform::ClipboardPayload::Files(paths) => (paths, None),
                platform::ClipboardPayload::FilesImage {
                    paths,
                    bytes,
                    extension,
                    ..
                } => (paths, Some((bytes, extension))),
                _ => unreachable!(),
            };
            let text = serde_json::to_string(&paths).map_err(|_| "storage_unavailable")?;
            let content = if paths.len() == 1 {
                image_file(Path::new(&paths[0]))
            } else {
                None
            };
            let is_image = content.is_some();
            let image = content
                .or_else(|| image.filter(|(bytes, _)| platform::image_dimensions(bytes).is_ok()));
            if let Some((bytes, extension)) = image {
                let (path, created) = clipboard::store_image_with_status(root, &bytes, &extension)?;
                if created {
                    created_image = Some(path.clone());
                }
                let kind = if is_image { "image" } else { "files" };
                let fingerprint =
                    clipboard::fingerprint(kind, format!("{text}\0{path}").as_bytes());
                representation = Some(ClipboardFileRepresentation {
                    paths,
                    image_path: path.clone(),
                });
                (
                    kind,
                    if is_image { String::new() } else { text },
                    is_image.then_some(path),
                    fingerprint,
                )
            } else {
                let fingerprint = clipboard::fingerprint("files", text.as_bytes());
                ("files", text, None, fingerprint)
            }
        }
        platform::ClipboardPayload::Image { bytes, extension } => {
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
    let saved = insert(&record, representation.as_ref());
    if saved.is_err() {
        if let Some(path) = created_image {
            clipboard::remove_image(root, &path)?;
        }
    }
    saved
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn single_image_files_use_verified_content_and_keep_file_copy_representations() {
        let root = std::env::temp_dir().join(format!("nami-image-kind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("screenshot.data");
        let bytes = include_bytes!("../../tests/fixtures/clipboard-wide.png");
        std::fs::write(&source, bytes).unwrap();
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
                &root,
                platform::ClipboardCapture {
                    version: 1,
                    payload,
                    source_app: None,
                },
            )
            .unwrap();
            let page = database
                .clipboard_page_filtered("", false, Some("image"), None, 100)
                .unwrap();
            assert_eq!(
                page.total_records, 1,
                "different source representations retain one content classification"
            );
            let id = &page.records[0].id;
            let record = database.clipboard_record(id).unwrap();
            assert_eq!(
                clipboard::read_image(&root, record.image_path.as_deref().unwrap()).unwrap(),
                bytes
            );
            let representation = database.clipboard_file_representation(id).unwrap().unwrap();
            assert_eq!(representation.paths, paths);
            let snapshot_path = root
                .join(record.image_path.as_deref().unwrap())
                .to_string_lossy()
                .into_owned();
            assert_eq!(
                record_payload(&root, record, Some(representation)).unwrap(),
                platform::ClipboardPayload::FilesImage {
                    paths: vec![snapshot_path],
                    bytes: bytes.to_vec(),
                    extension: "png".into(),
                    image_fallback: true
                }
            );
        }
        let id = database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records[0]
            .id
            .clone();
        for state in ["changed", "missing", "directory"] {
            match state {
                "changed" => {
                    std::fs::write(&source, b"replacement file must never be pasted").unwrap()
                }
                "missing" => std::fs::remove_file(&source).unwrap(),
                _ => std::fs::create_dir(&source).unwrap(),
            }
            let record = database.clipboard_record(&id).unwrap();
            let saved_path = root
                .join(record.image_path.as_deref().unwrap())
                .to_string_lossy()
                .into_owned();
            let restored = record_payload(
                &root,
                record,
                database.clipboard_file_representation(&id).unwrap(),
            )
            .unwrap();
            assert!(
                matches!(restored, platform::ClipboardPayload::FilesImage { paths, bytes: pixels, image_fallback: true, .. } if paths == vec![saved_path] && pixels == bytes),
                "{state} source must restore the saved image as both pixels and file"
            );
        }
        assert_eq!(
            database
                .clipboard_page_filtered("", false, Some("image"), None, 100)
                .unwrap()
                .total_records,
            1,
            "classification is persisted, not re-evaluated during list reads"
        );
        assert_eq!(
            database
                .clipboard_page_filtered("", false, Some("files"), None, 100)
                .unwrap()
                .total_records,
            0
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn image_classification_bounds_pixels_and_never_reads_oversized_files() {
        let root = std::env::temp_dir().join(format!("nami-image-bounds-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("large.png");
        let png = include_bytes!("../../tests/fixtures/clipboard-oversized.png");
        assert_eq!(platform::image_dimensions(png).unwrap(), (8193, 8193));
        std::fs::write(&source, png).unwrap();
        assert!(
            image_file(&source).is_none(),
            "huge compressed dimensions must not allocate pixels"
        );
        std::fs::OpenOptions::new()
            .write(true)
            .open(&source)
            .unwrap()
            .set_len(clipboard::MAX_IMAGE_BYTES as u64 + 1)
            .unwrap();
        assert!(image_file(&source).is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn jpeg_file_content_is_normalized_once_without_changing_its_dimensions() {
        let root = std::env::temp_dir().join(format!("nami-jpeg-kind-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("no-extension");
        let jpeg = include_bytes!("../../tests/fixtures/clipboard-wide.jpg");
        std::fs::write(&source, jpeg).unwrap();
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        persist_capture(
            &mut database,
            &root,
            platform::ClipboardCapture {
                version: 1,
                payload: platform::ClipboardPayload::Files(vec![source
                    .to_string_lossy()
                    .into_owned()]),
                source_app: None,
            },
        )
        .unwrap();
        let page = database
            .clipboard_page_filtered("", false, Some("image"), None, 100)
            .unwrap();
        assert_eq!(page.total_records, 1);
        let record = database.clipboard_record(&page.records[0].id).unwrap();
        let png = clipboard::read_image(&root, record.image_path.as_deref().unwrap()).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(
            platform::image_dimensions(&png).unwrap(),
            platform::image_dimensions(jpeg).unwrap()
        );
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
        let report = serde_json::json!({"scope":"synthetic 2048x256 PNG file; background classification, content-addressed snapshot and SQLite FULL commit; no UI or polling interval", "firstMs":first,"p50Ms":samples[15],"p95Ms":samples[28],"samples":samples.len(),"imageBytes":bytes.len()});
        std::fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/clipboard-image-capture-benchmark.json"),
            serde_json::to_vec_pretty(&report).unwrap(),
        )
        .unwrap();
        println!("{report}");
        assert_eq!(
            database
                .clipboard_page_filtered("", false, Some("image"), None, 100)
                .unwrap()
                .total_records,
            1
        );
        drop(database);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn file_icons_multiple_images_and_text_paths_do_not_override_file_semantics() {
        let root = std::env::temp_dir().join(format!("nami-file-kind-{}", uuid::Uuid::new_v4()));
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
                &root,
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
            assert!(matches!(
                record_payload(
                    &root,
                    record,
                    database.clipboard_file_representation(id).unwrap()
                )
                .unwrap(),
                platform::ClipboardPayload::FilesImage {
                    image_fallback: false,
                    ..
                }
            ));
        }
        persist_capture(
            &mut database,
            &root,
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
    fn mixed_image_preparation_stays_outside_sql_and_cleans_up_after_failed_commit() {
        let root =
            std::env::temp_dir().join(format!("nami-mixed-rollback-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let source = root.join("screenshot.png");
        std::fs::write(
            &source,
            include_bytes!("../../tests/fixtures/clipboard-wide.png"),
        )
        .unwrap();
        let database = std::sync::Mutex::new(HistoryDatabase::open(Path::new(":memory:")).unwrap());
        let snapshot = std::cell::RefCell::new(String::new());
        let result = persist_capture_with(
            &root,
            platform::ClipboardCapture {
                version: 1,
                payload: platform::ClipboardPayload::Files(vec![source
                    .to_string_lossy()
                    .into_owned()]),
                source_app: None,
            },
            |record, representation| {
                let _guard = database
                    .try_lock()
                    .expect("file IO and image recognition must precede the SQL lock");
                assert_eq!(record.kind, "image");
                let path = representation.unwrap().image_path.clone();
                assert!(root.join(&path).is_file());
                *snapshot.borrow_mut() = path;
                Err("storage_busy".into())
            },
        );
        assert_eq!(result.unwrap_err(), "storage_busy");
        assert!(!root.join(snapshot.into_inner()).exists());
        assert!(source.exists());
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
        persist_capture_with(&root, capture(), |record, representation| {
            assert_eq!(clipboard::read_image(&root, &relative).unwrap(), bytes);
            let mut guard = database
                .try_lock()
                .expect("preparation must not hold the SQL lock");
            *saved.borrow_mut() =
                Some(guard.clipboard_insert_with_file_representation(record, representation)?);
            Ok(())
        })
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
            |record, _representation| {
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
}
