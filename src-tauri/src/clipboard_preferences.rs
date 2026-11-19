use crate::{
    clipboard_runtime::{changed, database, image_root},
    commands::{authorize, on_main},
    platform, AppState,
};
use nami::{
    clipboard::{CleanupPreview, HistoryPolicy},
    database::HistoryDatabase,
};
use serde::{Deserialize, Serialize};
use std::{path::Path, sync::atomic::Ordering, time::Duration};
use tauri::{AppHandle, Emitter, Manager, Webview};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExcludedApp {
    pub bundle_id: String,
    pub name: String,
    #[serde(skip_deserializing, skip_serializing_if = "Option::is_none")]
    pub icon_data_url: Option<String>,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum Activation {
    Copy,
    #[default]
    Paste,
}

#[derive(Clone, Copy, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordsOpenFocus {
    #[default]
    Search,
    List,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardOptions {
    excluded_apps: Vec<ExcludedApp>,
    pub history_policy: HistoryPolicy,
    pub activation: Activation,
    records_open_focus: RecordsOpenFocus,
    pub record_shortcuts: nami::shortcuts::ClipboardRecordShortcuts,
}

impl Default for ClipboardOptions {
    fn default() -> Self {
        Self {
            excluded_apps: [
                ("com.apple.Passwords", "Passwords"),
                ("com.apple.keychainaccess", "Keychain Access"),
            ]
            .into_iter()
            .map(|(bundle_id, name)| ExcludedApp {
                bundle_id: bundle_id.into(),
                name: name.into(),
                icon_data_url: None,
            })
            .collect(),
            history_policy: HistoryPolicy::default(),
            activation: Activation::default(),
            records_open_focus: RecordsOpenFocus::default(),
            record_shortcuts: Default::default(),
        }
    }
}

impl ClipboardOptions {
    pub(crate) fn load(db: &HistoryDatabase) -> Result<Self, String> {
        let mut options = Self::default();
        if let Some(apps) = db
            .preference("clipboard_excluded_apps")?
            .and_then(|value| serde_json::from_str::<Vec<ExcludedApp>>(&value).ok())
            .filter(|apps| apps.len() <= 256 && apps.iter().all(valid_app))
        {
            options.excluded_apps = apps;
        }
        options.history_policy = db
            .preference("clipboard_history_policy")?
            .and_then(|value| serde_json::from_str::<HistoryPolicy>(&value).ok())
            .filter(|policy| policy.valid())
            .unwrap_or_default();
        options.activation = db
            .preference("clipboard_activation")?
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        options.records_open_focus = db
            .preference("records_open_focus")?
            .and_then(|value| serde_json::from_str(&value).ok())
            .unwrap_or_default();
        options.record_shortcuts = nami::shortcuts::ClipboardRecordShortcuts::from_store(
            db.preference("clipboard_record_shortcuts")?,
        );
        Ok(options)
    }

    pub(crate) fn excludes(&self, bundle_id: &str) -> bool {
        self.excluded_apps
            .iter()
            .any(|app| app.bundle_id.eq_ignore_ascii_case(bundle_id))
    }
}

fn valid_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 255
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_".contains(&byte))
}

fn valid_app(app: &ExcludedApp) -> bool {
    valid_bundle_id(&app.bundle_id) && !app.name.trim().is_empty() && app.name.len() <= 512
}

#[tauri::command]
pub async fn clipboard_options_get(
    window: Webview,
    app: AppHandle,
) -> Result<ClipboardOptions, String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let mut options = state.clipboard_options.lock().unwrap().clone();
    options.excluded_apps = with_app_presentation(options.excluded_apps).await;
    Ok(options)
}

async fn with_app_presentation(apps: Vec<ExcludedApp>) -> Vec<ExcludedApp> {
    if apps.is_empty() {
        return apps;
    }
    let mut display = apps.clone();
    tauri::async_runtime::spawn_blocking(move || {
        for app in &mut display {
            if let Some((name, icon)) = platform::application_presentation(&app.bundle_id) {
                if let Some(name) = name {
                    app.name = name;
                }
                app.icon_data_url = icon;
            }
        }
        display
    })
    .await
    .unwrap_or(apps)
}

async fn change_exclusions(
    app: &AppHandle,
    edit: impl FnOnce(&mut Vec<ExcludedApp>) -> Result<(), String>,
) -> Result<Vec<ExcludedApp>, String> {
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let mut apps = state
        .clipboard_options
        .lock()
        .unwrap()
        .excluded_apps
        .clone();
    edit(&mut apps)?;
    let value = serde_json::to_string(&apps).map_err(|_| "storage_record")?;
    database(app, move |db| {
        db.preference_set("clipboard_excluded_apps", &value)
    })
    .await?;
    state.clipboard_options.lock().unwrap().excluded_apps = apps.clone();
    Ok(apps)
}

#[tauri::command]
pub async fn clipboard_excluded_app_pick(
    window: Webview,
    app: AppHandle,
) -> Result<Option<Vec<ExcludedApp>>, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    let Some(selected) = on_main(&app, |_| platform::pick_excluded_app()).await?? else {
        return Ok(None);
    };
    if !valid_app(&selected) {
        return Err("clipboard_invalid_app".into());
    }
    let apps = change_exclusions(&app, |apps| {
        if !apps
            .iter()
            .any(|app| app.bundle_id.eq_ignore_ascii_case(&selected.bundle_id))
        {
            if apps.len() >= 256 {
                return Err("clipboard_exclusions_full".into());
            }
            apps.push(selected);
        }
        Ok(())
    })
    .await?;
    Ok(Some(with_app_presentation(apps).await))
}

#[tauri::command]
pub async fn clipboard_excluded_app_remove(
    window: Webview,
    app: AppHandle,
    bundle_id: String,
) -> Result<Vec<ExcludedApp>, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    if !valid_bundle_id(&bundle_id) {
        return Err("invalid_argument".into());
    }
    let apps = change_exclusions(&app, |apps| {
        apps.retain(|app| !app.bundle_id.eq_ignore_ascii_case(&bundle_id));
        Ok(())
    })
    .await?;
    Ok(with_app_presentation(apps).await)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ClipboardControls {
    activation: Activation,
    records_open_focus: RecordsOpenFocus,
    shortcuts: nami::shortcuts::ClipboardRecordShortcuts,
}

impl ClipboardOptions {
    pub(crate) fn controls(&self) -> ClipboardControls {
        ClipboardControls {
            activation: self.activation,
            records_open_focus: self.records_open_focus,
            shortcuts: self.record_shortcuts.clone(),
        }
    }
}

pub(crate) fn publish_controls(app: &AppHandle, state: &AppState) {
    let controls = state.clipboard_options.lock().unwrap().controls();
    let _ = app.emit("clipboard_controls_changed", controls);
}

#[tauri::command]
pub async fn clipboard_record_shortcut_set(
    window: Webview,
    app: AppHandle,
    action: nami::shortcuts::ClipboardRecordAction,
    binding: Option<nami::shortcuts::KeyBinding>,
    enabled: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let clipboard = state.clipboard_shortcut.lock().unwrap().binding.clone();
    let preferences = state
        .translation_preferences
        .lock()
        .unwrap()
        .shortcuts
        .clone();
    let reserved = std::iter::once(clipboard.as_ref())
        .chain(preferences.general_bindings().map(Some))
        .chain(
            action
                .shared_with_translation()
                .then_some(preferences.translate.as_ref()),
        )
        .collect::<Vec<_>>();
    let mut shortcuts = state
        .clipboard_options
        .lock()
        .unwrap()
        .record_shortcuts
        .clone();
    shortcuts.set(action, binding, enabled, &reserved)?;
    let value = serde_json::to_string(&shortcuts).map_err(|_| "storage_record")?;
    database(&app, move |db| {
        db.preference_set("clipboard_record_shortcuts", &value)
    })
    .await?;
    state.clipboard_options.lock().unwrap().record_shortcuts = shortcuts;
    publish_controls(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn clipboard_behavior_set(
    window: Webview,
    app: AppHandle,
    activation: Activation,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let value = serde_json::to_string(&activation).map_err(|_| "storage_record")?;
    database(&app, move |db| {
        db.preference_set("clipboard_activation", &value)
    })
    .await?;
    state.clipboard_options.lock().unwrap().activation = activation;
    publish_controls(&app, &state);
    Ok(())
}

#[tauri::command]
pub async fn records_open_focus_set(
    window: Webview,
    app: AppHandle,
    focus: RecordsOpenFocus,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let value = serde_json::to_string(&focus).map_err(|_| "storage_record")?;
    database(&app, move |db| {
        db.preference_set("records_open_focus", &value)
    })
    .await?;
    state.clipboard_options.lock().unwrap().records_open_focus = focus;
    publish_controls(&app, &state);
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StorageInfo {
    bytes: u64,
    unprotected_count: usize,
}

// Size of the image store, including files left behind by a failed cleanup. No external file paths.
fn image_bytes(root: &Path) -> Result<u64, String> {
    use std::fs::{read_dir, symlink_metadata};
    use std::io::ErrorKind;
    for directory in [root.to_path_buf(), root.join("origin")] {
        match symlink_metadata(directory) {
            Ok(info) if info.is_dir() => (),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(0),
            _ => return Err("storage_path".into()),
        }
    }
    let read_error = |_| "storage_unavailable";
    let mut bytes = 0u64;
    for bucket in read_dir(root.join("origin")).map_err(read_error)? {
        let bucket = bucket.map_err(read_error)?;
        if !bucket.file_type().map_err(read_error)?.is_dir() {
            continue;
        }
        for file in read_dir(bucket.path()).map_err(read_error)? {
            let file = file.map_err(read_error)?;
            if file.file_type().map_err(read_error)?.is_file() {
                match file.metadata() {
                    Ok(info) => bytes = bytes.saturating_add(info.len()),
                    Err(error) if error.kind() == ErrorKind::NotFound => (),
                    Err(error) => return Err(read_error(error).into()),
                }
            }
        }
    }
    Ok(bytes)
}

#[tauri::command]
pub async fn clipboard_storage_info(
    window: Webview,
    app: AppHandle,
) -> Result<StorageInfo, String> {
    authorize(&window, &["settings"])?;
    // This estimate doesn't need to hold up capture or copy while it enumerates images.
    let root = image_root(&app)?;
    let (data, unprotected_count) = database(&app, |db| {
        Ok((
            db.clipboard_storage_bytes()?,
            db.clipboard_unprotected_count()?,
        ))
    })
    .await?;
    let media = tauri::async_runtime::spawn_blocking(move || image_bytes(&root))
        .await
        .map_err(|_| "storage_worker")??;
    Ok(StorageInfo {
        bytes: data.saturating_add(media),
        unprotected_count,
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CleanupResult {
    removed: usize,
    media_cleanup_failed: bool,
}

#[tauri::command]
pub async fn clipboard_cleanup_preview(
    window: Webview,
    app: AppHandle,
    policy: Option<HistoryPolicy>,
) -> Result<CleanupPreview, String> {
    authorize(&window, &["settings"])?;
    validate_policy(policy)?;
    database(&app, move |db| {
        db.clipboard_cleanup_preview(policy, chrono::Utc::now().timestamp_millis())
    })
    .await
}

fn validate_policy(policy: Option<HistoryPolicy>) -> Result<(), String> {
    if policy.is_some_and(|policy| !policy.valid()) {
        return Err("invalid_argument".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn clipboard_cleanup_apply(
    window: Webview,
    app: AppHandle,
    policy: Option<HistoryPolicy>,
    categories: Option<u8>,
    token: String,
) -> Result<CleanupResult, String> {
    authorize(&window, &["settings"])?;
    validate_policy(policy)?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let result = cleanup_locked(&app, policy, categories.unwrap_or(0), Some(token), true).await?;
    if let Some(policy) = policy {
        state.clipboard_options.lock().unwrap().history_policy = policy;
    }
    Ok(result)
}

async fn cleanup_locked(
    app: &AppHandle,
    policy: Option<HistoryPolicy>,
    categories: u8,
    token: Option<String>,
    notify: bool,
) -> Result<CleanupResult, String> {
    let root = image_root(app)?;
    let (removed, images) = database(app, move |db| {
        db.clipboard_cleanup(
            policy,
            categories,
            chrono::Utc::now().timestamp_millis(),
            token.as_deref(),
        )
    })
    .await?;
    // The transaction is committed. Notify records even if the filesystem is temporarily inaccessible.
    if notify && removed > 0 {
        let _ = changed(app);
    }
    let media_cleanup_failed = if images.is_empty() {
        false
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            let mut failed = false;
            for path in images {
                failed |= nami::clipboard::remove_image(&root, &path).is_err();
            }
            failed
        })
        .await
        .unwrap_or(true)
    };
    Ok(CleanupResult {
        removed,
        media_cleanup_failed,
    })
}

pub(crate) async fn enforce_history_locked(app: &AppHandle, on_capture: bool) {
    let policy = app
        .state::<AppState>()
        .clipboard_options
        .lock()
        .unwrap()
        .history_policy;
    if policy == HistoryPolicy::default() || (on_capture && policy.limit.is_none()) {
        return;
    }
    match cleanup_locked(app, Some(policy), 0, None, !on_capture).await {
        Ok(result) if result.media_cleanup_failed => {
            eprintln!("Nami clipboard maintenance: clipboard_cleanup_failed")
        }
        Err(error) => eprintln!("Nami clipboard maintenance: {error}"),
        _ => (),
    }
}

pub(crate) fn start_maintenance(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            {
                let state = app.state::<AppState>();
                let _write = state.clipboard_writes.lock().await;
                enforce_history_locked(&app, false).await;
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

pub(crate) fn recording_menu_item(
    app: &AppHandle,
) -> tauri::Result<tauri::menu::MenuItem<tauri::Wry>> {
    let state = app.state::<AppState>();
    let item = tauri::menu::MenuItem::with_id(
        app,
        "clipboard_recording",
        recording_menu_label(state.clipboard_enabled.load(Ordering::SeqCst)),
        state.storage_error.lock().unwrap().is_none(),
        None::<&str>,
    )?;
    *state.clipboard_recording_menu.lock().unwrap() = Some(item.clone());
    Ok(item)
}

pub(crate) fn recording_menu_label(enabled: bool) -> &'static str {
    if enabled {
        "Pause Recording"
    } else {
        "Resume Recording"
    }
}

pub(crate) async fn set_recording(app: &AppHandle, enabled: Option<bool>) -> Result<(), String> {
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let enabled = enabled.unwrap_or_else(|| !state.clipboard_enabled.load(Ordering::SeqCst));
    database(app, move |db| db.clipboard_capture_set(enabled)).await?;
    on_main(app, move |app| {
        let state = app.state::<AppState>();
        state
            .clipboard_version
            .store(platform::clipboard_version(), Ordering::SeqCst);
        state.clipboard_enabled.store(enabled, Ordering::SeqCst);
        let menu = state.clipboard_recording_menu.lock().unwrap().clone();
        if let Some(menu) = menu {
            let _ = menu.set_text(recording_menu_label(enabled));
        }
        for label in ["clipboard", "translation", "settings"] {
            if let Some(view) = app.get_webview(label) {
                let _ = view.emit("clipboard_capture_changed", enabled);
            }
        }
    })
    .await
}
