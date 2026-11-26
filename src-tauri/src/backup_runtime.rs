use crate::{
    clipboard_preferences, clipboard_runtime,
    commands::{authorize, on_main},
    load_clipboard_binding, parse_ui_locale, platform, sync_global_shortcuts, AppState,
    ClipboardShortcut,
};
use nami::{
    clipboard::RecordingTypes,
    database::{self, BackupImportMode, BackupImportReport, BackupSummary},
    model::WindowPositionMode,
    translation,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant, SystemTime};
use tauri::{AppHandle, Emitter, Manager, Webview};
use zeroize::Zeroizing;

const IMPORT_CONFIRM_TTL: Duration = Duration::from_secs(300);

pub(crate) struct PendingBackupImport {
    token: String,
    path: PathBuf,
    file_size: u64,
    modified: Option<SystemTime>,
    encrypted: bool,
    created_at: Instant,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupImportPreview {
    pub token: String,
    pub file_name: String,
    pub encrypted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<BackupSummary>,
}

fn validate_export_destination(app_data: &Path, destination: &Path) -> Result<PathBuf, String> {
    let app_data = app_data.canonicalize().map_err(|_| "storage_unavailable")?;
    let parent = destination.parent().ok_or("backup_export_failed")?;
    let parent = parent.canonicalize().map_err(|_| "backup_export_failed")?;
    if parent.starts_with(&app_data) {
        return Err("backup_export_failed".into());
    }
    let file_name = destination.file_name().ok_or("backup_export_failed")?;
    let resolved = parent.join(file_name);
    match resolved.symlink_metadata() {
        Ok(metadata) if metadata.is_file() => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        _ => return Err("backup_export_failed".into()),
    }
    Ok(resolved)
}

fn atomic_publish_file(staging: &Path, destination: &Path) -> Result<(), String> {
    let parent = destination.parent().ok_or("backup_export_failed")?;
    let temp = parent.join(format!(
        ".nami-backup-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let copy_result = (|| -> Result<(), String> {
        let mut src = std::fs::File::open(staging).map_err(|_| "backup_export_failed")?;
        let mut dst = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| "backup_export_failed")?;
        std::io::copy(&mut src, &mut dst).map_err(|_| "backup_export_failed")?;
        dst.sync_all().map_err(|_| "backup_export_failed")?;
        std::fs::rename(&temp, destination).map_err(|_| "backup_export_failed")?;
        Ok(())
    })();
    if copy_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    copy_result
}

#[tauri::command]
pub async fn backup_export(
    window: Webview,
    app: AppHandle,
    password: Option<String>,
) -> Result<Option<BackupSummary>, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().local_backup {
        return Err("platform_unavailable".into());
    }
    let password = password
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new);
    if password
        .as_ref()
        .is_some_and(|value| value.len() > database::MAX_BACKUP_PASSWORD_BYTES)
    {
        return Err("invalid_argument".into());
    }
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|_| "storage_unavailable")?;
    let image_root = clipboard_runtime::image_root(&app)?;
    let parent = window.window();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    on_main(&app, move |_| {
        platform::pick_backup_save_path(&parent, move |result| {
            let _ = sender.send(result);
        })
    })
    .await??;
    let Some(destination) = receiver.await.map_err(|_| "backup_dialog_failed")?? else {
        return Ok(None);
    };

    let resolved_dest = validate_export_destination(&app_data, &destination)?;
    let staging = database::TempFileGuard(std::env::temp_dir().join(format!(
        "nami-backup-staging-{}.namibak",
        uuid::Uuid::new_v4().simple()
    )));

    let state = app.state::<AppState>();
    let _clipboard_lock = state.clipboard_writes.lock().await;

    let staging_for_db = staging.0.clone();
    let mut summary = clipboard_runtime::database(&app, move |db| {
        db.export_backup(&image_root, &staging_for_db)
    })
    .await?;

    let staging_for_publish = staging.0.clone();
    let encrypted_size =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<u64>, String> {
            if let Some(password) = password {
                let encrypted_staging = database::TempFileGuard(std::env::temp_dir().join(
                    format!("nami-backup-enc-{}.namibak", uuid::Uuid::new_v4().simple()),
                ));
                let size = database::encrypt_backup_file(
                    &staging_for_publish,
                    &encrypted_staging.0,
                    &password,
                )?;
                atomic_publish_file(&encrypted_staging.0, &resolved_dest)?;
                Ok(Some(size))
            } else {
                atomic_publish_file(&staging_for_publish, &resolved_dest)?;
                Ok(None)
            }
        })
        .await
        .map_err(|_| "storage_worker")??;

    if let Some(size) = encrypted_size {
        summary.file_size = size;
    }
    Ok(Some(summary))
}

#[tauri::command]
pub async fn backup_import_pick(
    window: Webview,
    app: AppHandle,
) -> Result<Option<BackupImportPreview>, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().local_backup {
        return Err("platform_unavailable".into());
    }
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|_| "storage_unavailable")?;
    let parent = window.window();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    on_main(&app, move |_| {
        platform::pick_backup_open_path(&parent, move |result| {
            let _ = sender.send(result);
        })
    })
    .await??;
    let Some(selected) = receiver.await.map_err(|_| "backup_dialog_failed")?? else {
        return Ok(None);
    };

    let inspected = tauri::async_runtime::spawn_blocking(move || -> Result<_, String> {
        let app_data = app_data.canonicalize().map_err(|_| "storage_unavailable")?;
        let canonical = selected.canonicalize().map_err(|_| "backup_invalid")?;
        if canonical.starts_with(&app_data) {
            return Err("backup_invalid".into());
        }
        let meta = std::fs::symlink_metadata(&canonical).map_err(|_| "backup_invalid")?;
        if !meta.is_file() {
            return Err("backup_invalid".into());
        }
        let encrypted = database::is_encrypted_backup(&canonical)?;
        let summary = if encrypted {
            None
        } else {
            Some(database::inspect_backup(&canonical)?)
        };
        let file_name = canonical
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "backup.namibak".into());
        Ok((
            canonical,
            meta.len(),
            meta.modified().ok(),
            file_name,
            encrypted,
            summary,
        ))
    })
    .await
    .map_err(|_| "storage_worker")??;

    let (canonical, file_size, modified, file_name, encrypted, summary) = inspected;
    let token = uuid::Uuid::new_v4().to_string();
    *app.state::<AppState>()
        .pending_backup_import
        .lock()
        .unwrap() = Some(PendingBackupImport {
        token: token.clone(),
        path: canonical,
        file_size,
        modified,
        encrypted,
        created_at: Instant::now(),
    });
    Ok(Some(BackupImportPreview {
        token,
        file_name,
        encrypted,
        summary,
    }))
}

#[tauri::command]
pub async fn backup_import_unlock(
    window: Webview,
    app: AppHandle,
    token: String,
    password: String,
) -> Result<BackupSummary, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().local_backup {
        return Err("platform_unavailable".into());
    }
    let password = Zeroizing::new(password);
    if password.is_empty() {
        return Err("backup_password_required".into());
    }
    if password.len() > database::MAX_BACKUP_PASSWORD_BYTES {
        return Err("backup_wrong_password".into());
    }
    let (path, file_size, modified, encrypted) = {
        let state = app.state::<AppState>();
        let slot = state.pending_backup_import.lock().unwrap();
        let Some(pending) = slot.as_ref() else {
            return Err("backup_token_expired".into());
        };
        if pending.token != token || pending.created_at.elapsed() > IMPORT_CONFIRM_TTL {
            return Err("backup_token_expired".into());
        }
        (
            pending.path.clone(),
            pending.file_size,
            pending.modified,
            pending.encrypted,
        )
    };
    if !encrypted {
        return Err("invalid_argument".into());
    }

    tauri::async_runtime::spawn_blocking(move || -> Result<BackupSummary, String> {
        let current_meta = std::fs::symlink_metadata(&path).map_err(|_| "backup_invalid")?;
        if !current_meta.is_file()
            || current_meta.len() != file_size
            || current_meta.modified().ok() != modified
        {
            return Err("backup_changed".into());
        }
        database::inspect_backup_with_password(&path, Some(&password))
    })
    .await
    .map_err(|_| "storage_worker")?
}

#[tauri::command]
pub async fn backup_import_apply(
    window: Webview,
    app: AppHandle,
    token: String,
    mode: BackupImportMode,
    password: Option<String>,
) -> Result<BackupImportReport, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().local_backup {
        return Err("platform_unavailable".into());
    }
    let password = password
        .filter(|value| !value.is_empty())
        .map(Zeroizing::new);
    let pending = {
        let state = app.state::<AppState>();
        let mut slot = state.pending_backup_import.lock().unwrap();
        let valid = slot.as_ref().is_some_and(|pending| {
            pending.token == token && pending.created_at.elapsed() <= IMPORT_CONFIRM_TTL
        });
        if !valid {
            return Err("backup_token_expired".into());
        }
        if slot
            .as_ref()
            .is_some_and(|pending| pending.encrypted && password.is_none())
        {
            return Err("backup_password_required".into());
        }
        slot.take().unwrap()
    };

    let current_meta = std::fs::symlink_metadata(&pending.path).map_err(|_| "backup_invalid")?;
    if !current_meta.is_file()
        || current_meta.len() != pending.file_size
        || current_meta.modified().ok() != pending.modified
    {
        return Err("backup_changed".into());
    }

    let image_root = clipboard_runtime::image_root(&app)?;
    let state = app.state::<AppState>();
    let _clipboard_lock = state.clipboard_writes.lock().await;
    let _shortcut_lock = state.shortcut_writes.lock().await;

    let import_path = pending.path.clone();
    let image_root_for_import = image_root.clone();
    let primary_modifier = platform::capabilities().primary_modifier;

    let (report, reloaded_prefs) = clipboard_runtime::database(&app, move |db| {
        let report = db.import_backup_with_password(
            &import_path,
            &image_root_for_import,
            mode,
            password.as_deref().map(String::as_str),
        )?;
        let reloaded = if mode == BackupImportMode::Replace {
            let ui_locale = parse_ui_locale(db.preference("ui_locale")?.as_deref());
            let window_position_mode =
                WindowPositionMode::parse(db.preference("window_position_mode")?.as_deref());
            let capture = db.clipboard_capture_enabled()?;
            let mut preferences = translation::Preferences::from_store(
                |key| db.preference(key).ok().flatten(),
                primary_modifier,
            );
            let recording_types = db
                .preference("clipboard_recording_types")?
                .and_then(|value| serde_json::from_str::<RecordingTypes>(&value).ok())
                .unwrap_or_default();
            let mut clipboard_options = clipboard_preferences::ClipboardOptions::load(db)?;
            let clipboard_shortcut = ClipboardShortcut {
                binding: load_clipboard_binding(
                    db.preference("clipboard_shortcut")?.as_deref(),
                    &preferences.shortcuts,
                ),
                enabled: db.preference("clipboard_shortcut_enabled")?.as_deref() != Some("false"),
            };
            clipboard_options
                .record_shortcuts
                .upgrade_translate_binding(
                    &preferences.shortcuts,
                    clipboard_shortcut.binding.as_ref(),
                );
            nami::shortcuts::upgrade_legacy_pin_defaults(
                &mut clipboard_options.record_shortcuts,
                &mut preferences.shortcuts,
                clipboard_shortcut.binding.as_ref(),
            );
            Some((
                ui_locale,
                window_position_mode,
                capture,
                preferences,
                recording_types,
                clipboard_options,
                clipboard_shortcut,
            ))
        } else {
            None
        };
        Ok((report, reloaded))
    })
    .await?;

    let _ = clipboard_runtime::changed(&app);

    if let Some((
        ui_locale,
        window_position_mode,
        capture,
        preferences,
        recording_types,
        clipboard_options,
        clipboard_shortcut,
    )) = reloaded_prefs
    {
        let capture_active = capture && platform::capabilities().clipboard_write;
        let controls = clipboard_options.controls();
        let binding_payload = serde_json::json!({
            "binding": clipboard_shortcut.binding,
            "enabled": clipboard_shortcut.enabled,
            "shortcutErrors": Vec::<String>::new(),
        });
        *state.locale.lock().unwrap() = ui_locale.clone();
        *state.window_position_mode.lock().unwrap() = window_position_mode;
        *state.translation_engine.lock().unwrap() = preferences.engine.clone();
        *state.translation_preferences.lock().unwrap() = preferences.clone();
        *state.clipboard_recording_types.lock().unwrap() = recording_types;
        *state.clipboard_options.lock().unwrap() = clipboard_options;
        *state.clipboard_shortcut.lock().unwrap() = clipboard_shortcut;
        state
            .clipboard_enabled
            .store(capture_active, Ordering::SeqCst);

        if let Some(menu) = state.clipboard_recording_menu.lock().unwrap().as_ref() {
            let _ = menu.set_text(if capture_active {
                "Pause Clipboard Recording"
            } else {
                "Resume Clipboard Recording"
            });
        }

        let sync_app = app.clone();
        let _ = on_main(&app, move |_| {
            sync_global_shortcuts(&sync_app);
        })
        .await;

        for label in ["translation", "clipboard", "settings"] {
            if let Some(view) = app.get_webview(label) {
                let _ = view.emit("locale_changed", &ui_locale);
                let _ = view.emit("window_position_mode_changed", window_position_mode);
                let _ = view.emit("translation_preferences_changed", &preferences);
                let _ = view.emit("translation_engine_changed", &preferences.engine);
                let _ = view.emit("clipboard_capture_changed", capture_active);
                let _ = view.emit("clipboard_controls_changed", &controls);
                let _ = view.emit("clipboard_binding_changed", &binding_payload);
                if label == "settings" {
                    let _ = view.emit("clipboard_recording_types_changed", recording_types);
                }
            }
        }

        let _ = clipboard_preferences::retry_image_deletions_locked(&app, image_root).await;
    }

    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn export_destination_rejects_app_data_and_symlinks() {
        let root =
            std::env::temp_dir().join(format!("nami-backup-dest-test-{}", uuid::Uuid::new_v4()));
        let app_data = root.join("app_data");
        let outside = root.join("outside");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&outside).unwrap();

        let inside_dest = app_data.join("inside.namibak");
        assert_eq!(
            validate_export_destination(&app_data, &inside_dest).unwrap_err(),
            "backup_export_failed"
        );

        let valid_dest = outside.join("valid.namibak");
        assert!(validate_export_destination(&app_data, &valid_dest).is_ok());

        #[cfg(unix)]
        {
            let target = outside.join("real.namibak");
            std::fs::write(&target, b"data").unwrap();
            let link = outside.join("link.namibak");
            std::os::unix::fs::symlink(&target, &link).unwrap();
            assert_eq!(
                validate_export_destination(&app_data, &link).unwrap_err(),
                "backup_export_failed"
            );
        }

        let _ = std::fs::remove_dir_all(root);
    }
}
