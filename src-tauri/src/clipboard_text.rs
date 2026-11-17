use crate::{
    clipboard_runtime, commands::on_main, platform, selection::SelectionContext, windows, AppState,
};
use nami::clipboard::ClipboardRecord;
use std::{collections::VecDeque, sync::atomic::Ordering};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Default)]
pub(crate) struct ImageTextCache(VecDeque<(String, String)>);

impl ImageTextCache {
    fn get(&mut self, key: &str) -> Option<String> {
        let index = self.0.iter().position(|(stored, _)| stored == key)?;
        let entry = self.0.remove(index)?;
        let text = entry.1.clone();
        self.0.push_back(entry);
        Some(text)
    }

    fn insert(&mut self, key: String, text: String) {
        const BUDGET: usize = 4 * 1024 * 1024;
        if text.len() > BUDGET {
            return;
        }
        self.0.retain(|(stored, _)| stored != &key);
        while self.0.len() >= 64
            || self.0.iter().map(|(_, text)| text.len()).sum::<usize>() + text.len() > BUDGET
        {
            self.0.pop_front();
        }
        self.0.push_back((key, text));
    }
}

fn eligible(record: &ClipboardRecord) -> Result<(), String> {
    if !["text", "image", "files"].contains(&record.kind.as_str()) {
        return Err("invalid_argument".into());
    }
    Ok(())
}

fn revalidate(record: &ClipboardRecord, kind: &str, fingerprint: &str) -> Result<(), String> {
    eligible(record)?;
    if record.kind != kind || record.fingerprint != fingerprint {
        return Err("cancelled".into());
    }
    Ok(())
}

fn image_bytes(
    record: &ClipboardRecord,
    root: &std::path::Path,
) -> Result<Option<Vec<u8>>, String> {
    if record.kind == "image" {
        return nami::clipboard::read_image(
            root,
            record.image_path.as_deref().ok_or("storage_corrupt")?,
        )
        .map(Some);
    }
    let paths: Vec<String> = serde_json::from_str(&record.text).map_err(|_| "storage_corrupt")?;
    if paths.len() != 1 {
        return Ok(None);
    }
    let entries = nami::clipboard::file_entries(&record.text)?;
    let entry = &entries[0];
    if !entry.exists || entry.is_dir || !entry.is_image {
        return Ok(None);
    }
    nami::clipboard::read_file_preview(std::path::Path::new(&entry.path)).map(Some)
}

async fn text(app: &AppHandle, id: String) -> Result<(ClipboardRecord, String), String> {
    let record =
        clipboard_runtime::database(app, move |database| database.clipboard_record(&id)).await?;
    eligible(&record)?;
    if record.kind == "text" {
        return Ok((record.clone(), record.text));
    }
    if !platform::capabilities().image_ocr {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    // One Vision worker, shared by detection and explicit actions; never hold the database mutex here.
    let mut cache = state.image_text.lock().await;
    let root = clipboard_runtime::image_root(app)?;
    let source = record.clone();
    // File records reference mutable paths; cache by the bytes actually recognized.
    let image = tauri::async_runtime::spawn_blocking(move || {
        image_bytes(&source, &root).map(|bytes| {
            bytes.map(|bytes| {
                let fingerprint = nami::clipboard::fingerprint("image", &bytes);
                (bytes, fingerprint)
            })
        })
    })
    .await
    .map_err(|_| "storage_worker")??;
    let Some((bytes, fingerprint)) = image else {
        return Ok((record, String::new()));
    };
    if let Some(text) = cache.get(&fingerprint) {
        return Ok((record, text));
    }
    let result =
        tauri::async_runtime::spawn_blocking(move || platform::recognize_image_text(&bytes))
            .await
            .map_err(|_| "storage_worker")??;
    cache.insert(fingerprint, result.clone());
    Ok((record, result))
}

pub(crate) async fn has_text(app: &AppHandle, id: String) -> Result<bool, String> {
    text(app, id).await.map(|(_, text)| !text.trim().is_empty())
}

pub(crate) async fn process(app: &AppHandle, id: String, action: String) -> Result<(), String> {
    if !["translate", "ocr"].contains(&action.as_str()) {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    let operation = state.image_text_operation.fetch_add(1, Ordering::SeqCst) + 1;
    let clipboard_operation = state.operation.fetch_add(1, Ordering::SeqCst) + 1;
    let translation_revision = state.translation_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let session = state.windows.lock().unwrap().records_session;
    on_main(app, |app| crate::records_runtime::protect_result(&app)).await??;
    let (record, text) = text(app, id).await?;
    if text.trim().is_empty() {
        return Err("ocr_no_text".into());
    }
    if action == "translate" {
        nami::translation::validate_text(&text)?;
    }
    if action == "ocr" && record.kind == "text" {
        return Err("invalid_argument".into());
    }
    let _write = state.clipboard_writes.lock().await;
    let record_id = record.id.clone();
    let fingerprint = record.fingerprint.clone();
    let kind = record.kind.clone();
    clipboard_runtime::database(app, move |database| {
        let current = database.clipboard_record(&record_id)?;
        revalidate(&current, &kind, &fingerprint)
    })
    .await?;
    let copied = text.clone();
    let source = record.source_app.clone();
    let background_source = source.clone();
    let is_ocr = action == "ocr";
    let background = on_main(app, move |app| {
        let state = app.state::<AppState>();
        {
            let regions = state.windows.lock().unwrap();
            if state.image_text_operation.load(Ordering::SeqCst) != operation
                || !is_ocr
                    && state.translation_revision.load(Ordering::SeqCst) != translation_revision
                || regions.records_session != session
                || (!regions.history_visible && !regions.translation_visible)
                || state.records_dirty.load(Ordering::SeqCst)
            {
                return Err("cancelled".into());
            }
        }
        if !is_ocr && state.windows.lock().unwrap().records_browsing {
            return Ok(true);
        }
        if action == "ocr" {
            if state.operation.load(Ordering::SeqCst) == clipboard_operation {
                let version =
                    platform::write_text(&copied).map_err(|(code, _)| code.to_string())?;
                state.clipboard_version.store(version, Ordering::SeqCst);
            }
        } else {
            *state.clipboard_translation.lock().unwrap() = Some(SelectionContext {
                text: copied,
                presentation_id: None,
                source_app: source.clone(),
                error: None,
            });
            *state.selection_source.lock().unwrap() = source;
            {
                let mut regions = state.windows.lock().unwrap();
                regions.set_record_view(nami::model::RecordView::Translation);
                regions.translation_visible = true;
            }
            let delivery = windows::apply_translation(&app).and_then(|_| {
                let view = app.get_webview("translation").ok_or("window_unavailable")?;
                view.emit("clipboard_translation_ready", ())
                    .map_err(|_| "event_emit")?;
                view.set_focus().map_err(|_| "window_focus".to_string())
            });
            if delivery.is_err() {
                state.clipboard_translation.lock().unwrap().take();
            }
            delivery?;
        }
        Ok::<_, String>(false)
    })
    .await??;
    if background {
        drop(_write);
        if state.image_text_operation.load(Ordering::SeqCst) != operation {
            return Err("cancelled".into());
        }
        let preferences = state.translation_preferences.lock().unwrap().clone();
        let mut targets = vec![preferences.primary_language];
        if preferences.secondary_language != "none" {
            targets.push(preferences.secondary_language);
        }
        crate::commands::run_translation(
            app.clone(),
            text,
            preferences.source_language,
            targets,
            background_source,
            None,
            Some(translation_revision),
        )
        .await?;
        return Ok(());
    }
    if is_ocr {
        let captured = ClipboardRecord {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "text".into(),
            fingerprint: nami::clipboard::fingerprint("text", text.as_bytes()),
            text,
            date: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            is_favorite: false,
            is_pinned: false,
            source_app: Some(serde_json::json!({"name":"Nami"})),
            image_path: None,
        };
        let saved_id = clipboard_runtime::database(app, move |database| {
            database.clipboard_insert(&captured).map(|record| record.id)
        })
        .await?;
        drop(_write);
        let selected_id = saved_id.clone();
        let follow_result = on_main(app, move |app| {
            let state = app.state::<AppState>();
            let mut windows = state.windows.lock().unwrap();
            let follow = windows.records_session == session
                && !windows.records_browsing
                && state.image_text_operation.load(Ordering::SeqCst) == operation
                && state.operation.load(Ordering::SeqCst) == clipboard_operation;
            if follow {
                windows.clipboard_detail_id = Some(selected_id);
            }
            drop(windows);
            windows::emit_state(&app)?;
            Ok::<_, String>(follow)
        })
        .await??;
        clipboard_runtime::changed(app)?;
        if !follow_result {
            return Ok(());
        }
        for label in ["clipboard", "translation"] {
            if let Some(view) = app.get_webview(label) {
                view.emit("clipboard_text_created", saved_id.clone())
                    .map_err(|_| "event_emit")?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_retains_no_text_and_evicts_old_results_with_a_memory_bound() {
        let mut cache = ImageTextCache::default();
        cache.insert("empty".into(), String::new());
        assert_eq!(cache.get("empty"), Some(String::new()));
        for index in 0..64 {
            cache.insert(index.to_string(), "recognized text".into());
        }
        assert_eq!(cache.get("empty"), None);
        assert!(cache.get("0").is_some());
        cache.insert("new".into(), "recognized text".into());
        assert!(cache.get("0").is_some());
        assert_eq!(cache.get("1"), None);
        cache.insert("large".into(), "x".repeat(4 * 1024 * 1024));
        assert_eq!(cache.0.len(), 1);
        cache.insert("too-large".into(), "x".repeat(4 * 1024 * 1024 + 1));
        assert_eq!(cache.0.len(), 1);
    }

    #[test]
    fn file_ocr_reads_only_one_existing_image_and_tracks_changed_bytes() {
        let root = std::env::temp_dir().join(format!("nami-ocr-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("image.png");
        let document = root.join("document.txt");
        std::fs::write(&image, b"first image bytes").unwrap();
        std::fs::write(&document, b"ordinary file text").unwrap();
        let mut record = ClipboardRecord {
            id: "synthetic".into(),
            kind: "files".into(),
            text: serde_json::to_string(&[&image]).unwrap(),
            date: String::new(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: "path-fingerprint".into(),
        };
        assert_eq!(
            image_bytes(&record, &root).unwrap(),
            Some(b"first image bytes".to_vec())
        );
        std::fs::write(&image, b"changed image bytes").unwrap();
        assert_eq!(
            image_bytes(&record, &root).unwrap(),
            Some(b"changed image bytes".to_vec())
        );
        for paths in [
            vec![&document],
            vec![&root],
            vec![&image, &document],
            vec![],
        ] {
            record.text = serde_json::to_string(&paths).unwrap();
            assert_eq!(image_bytes(&record, &root).unwrap(), None);
        }
        record.text = serde_json::to_string(&[&image]).unwrap();
        std::fs::remove_file(&image).unwrap();
        assert_eq!(image_bytes(&record, &root).unwrap(), None);
        record.text = "not a file list".into();
        assert_eq!(image_bytes(&record, &root), Err("storage_corrupt".into()));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn actions_accept_supported_record_kinds_regardless_of_favorite() {
        let mut record = ClipboardRecord {
            id: "synthetic".into(),
            kind: "text".into(),
            text: "Synthetic text".into(),
            date: "2026-11-16T00:00:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: "synthetic".into(),
        };
        assert!(eligible(&record).is_ok());
        record.is_favorite = true;
        assert!(eligible(&record).is_ok());
        assert!(revalidate(&record, "text", "synthetic").is_ok());
        record.fingerprint = "edited-text".into();
        assert_eq!(
            revalidate(&record, "text", "synthetic"),
            Err("cancelled".into())
        );
        record.kind = "image".into();
        record.is_favorite = false;
        assert!(eligible(&record).is_ok());
        assert_eq!(
            revalidate(&record, "text", "edited-text"),
            Err("cancelled".into())
        );
        record.kind = "files".into();
        assert!(eligible(&record).is_ok());
        record.kind = "unknown".into();
        assert_eq!(eligible(&record), Err("invalid_argument".into()));
    }
}
