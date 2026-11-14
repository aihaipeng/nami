use crate::{commands::on_main, platform, AppState};
use nami::{
    clipboard::{self, ClipboardRecord},
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

pub(crate) fn changed(app: &AppHandle) -> Result<(), String> {
    let revision = app
        .state::<AppState>()
        .clipboard_revision
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    if let Some(view) = app.get_webview("clipboard") {
        view.emit("clipboard_changed", revision)
            .map_err(|_| "event_emit")?;
    }
    Ok(())
}

pub(crate) async fn payload(
    app: &AppHandle,
    id: String,
) -> Result<platform::ClipboardPayload, String> {
    let root = image_root(app)?;
    database(app, move |database| {
        let record = database.clipboard_record(&id)?;
        match record.kind.as_str() {
            "text" => Ok(platform::ClipboardPayload::Text(record.text)),
            "files" => Ok(platform::ClipboardPayload::Files(
                serde_json::from_str(&record.text).map_err(|_| "storage_corrupt")?,
            )),
            "image" => {
                let path = record.image_path.ok_or("storage_corrupt")?;
                let bytes = clipboard::read_image(&root, &path)?;
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
    })
    .await
}

fn observe_pending(
    handled: &AtomicI64,
    version: i64,
    busy: bool,
    read: impl FnOnce() -> Result<Option<platform::ClipboardCapture>, &'static str>,
) -> Result<Option<(i64, platform::ClipboardCapture)>, &'static str> {
    let previous = handled.load(Ordering::SeqCst);
    if busy || previous == version {
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
                    state.selection_busy.load(Ordering::SeqCst),
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
            let result = database(&app, move |database| {
                persist_capture(database, &root, capture)
            })
            .await;
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

pub(crate) fn persist_capture(
    database: &mut HistoryDatabase,
    root: &Path,
    capture: platform::ClipboardCapture,
) -> Result<(), String> {
    let mut created_image = None;
    let (kind, text, image_path, fingerprint) = match capture.payload {
        platform::ClipboardPayload::Text(text) => {
            let fingerprint = clipboard::fingerprint("text", text.as_bytes());
            ("text", text, None, fingerprint)
        }
        platform::ClipboardPayload::Files(paths) => {
            let text = serde_json::to_string(&paths).map_err(|_| "storage_unavailable")?;
            let fingerprint = clipboard::fingerprint("files", text.as_bytes());
            ("files", text, None, fingerprint)
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
    let saved = database.clipboard_insert(&ClipboardRecord {
        id: uuid::Uuid::new_v4().to_string(),
        kind: kind.into(),
        text,
        date: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        is_favorite: false,
        is_pinned: false,
        source_app: capture.source_app,
        image_path,
        fingerprint,
    });
    if saved.is_err() {
        if let Some(path) = created_image {
            clipboard::remove_image(root, &path)?;
        }
    }
    saved.map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
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
    fn ax_selection_pause_does_not_consume_an_external_copy() {
        let handled = AtomicI64::new(7);
        assert!(observe_pending(&handled, 8, true, || panic!(
            "must not read while selecting"
        ))
        .unwrap()
        .is_none());
        assert!(
            observe_pending(&handled, 8, false, || Ok(Some(synthetic_capture(8))))
                .unwrap()
                .is_some()
        );
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
        let existing_path = database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records[0]
            .image_path
            .clone()
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
            "/synthetic/space # %.txt".to_owned(),
            "/synthetic/new\nline".to_owned(),
        ];
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
            clipboard::read_image(&root, image.image_path.as_deref().unwrap()).unwrap(),
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
