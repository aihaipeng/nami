use crate::{
    model::{OperationResult, WindowState},
    platform, windows, AppState,
};
use serde::Serialize;
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{AppHandle, Emitter, Manager, Webview};

const IMAGE_PASTE_SETTLE: Duration = Duration::from_millis(200);
const PASTE_KEY_GAP: Duration = Duration::from_millis(15);
const PASTE_FOCUS_RETURN_DELAY: Duration = Duration::from_millis(60);

fn image_history_paste(payload: &platform::ClipboardPayload) -> bool {
    matches!(
        payload,
        platform::ClipboardPayload::Image { .. }
            | platform::ClipboardPayload::FilesImage {
                image_fallback: true,
                ..
            }
    )
}

fn remaining_image_settle(image: bool, elapsed: Duration) -> Duration {
    if image {
        IMAGE_PASTE_SETTLE.saturating_sub(elapsed)
    } else {
        Duration::ZERO
    }
}

// Keyup has already been posted. A changed clipboard wins over a lost focus so
// the caller does not report the paste as delivered.
fn paste_release_result(focused: bool, version_matches: bool) -> Result<(), &'static str> {
    if !version_matches {
        Err("clipboard_changed")
    } else if !focused {
        Err("target_unavailable")
    } else {
        Ok(())
    }
}

enum PasteAttempt {
    Wait(Duration),
    Posted(u64),
}

fn pinned_clipboard_session(window: &WindowState) -> Option<u64> {
    (window.history_visible
        && window.history_pinned
        && window.record_view == nami::model::RecordView::Clipboard
        && !window.records_editor_open)
        .then_some(window.records_session)
}

#[derive(Clone, Copy)]
struct PasteFocusReturn {
    session: u64,
    operation: u64,
    activation: u64,
}

impl PasteFocusReturn {
    fn still_owned(self, window: &WindowState, operation: u64, activation: u64) -> bool {
        pinned_clipboard_session(window) == Some(self.session)
            && self.operation == operation
            && self.activation == activation
    }
}

pub(crate) fn authorize(window: &Webview, labels: &[&str]) -> Result<(), String> {
    if labels.contains(&window.label()) {
        Ok(())
    } else {
        Err("permission_denied".into())
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    capabilities: platform::Capabilities,
    locale: String,
    launch_at_login: bool,
    accessibility: bool,
    shortcut_errors: Vec<String>,
    translation_engine: String,
    translation_preferences: crate::translation::Preferences,
    storage_error: Option<String>,
    clipboard_capture_enabled: bool,
    clipboard_recording_types: nami::clipboard::RecordingTypes,
    clipboard_binding: Option<crate::shortcuts::KeyBinding>,
    clipboard_shortcut_enabled: bool,
    clipboard_controls: crate::clipboard_preferences::ClipboardControls,
    window: WindowState,
}

#[tauri::command]
pub fn app_snapshot(window: Webview, app: AppHandle) -> Result<Snapshot, String> {
    authorize(
        &window,
        &["translation", "history", "clipboard", "settings"],
    )?;
    let state = app.state::<AppState>();
    let clipboard_shortcut = state.clipboard_shortcut.lock().unwrap().clone();
    let snapshot = Snapshot {
        capabilities: platform::capabilities(),
        locale: state.locale.lock().unwrap().clone(),
        launch_at_login: platform::launch_at_login_enabled(),
        accessibility: platform::accessibility(),
        shortcut_errors: state.shortcut_errors.lock().unwrap().clone(),
        translation_engine: state.translation_engine.lock().unwrap().clone(),
        translation_preferences: state.translation_preferences.lock().unwrap().clone(),
        storage_error: state.storage_error.lock().unwrap().clone(),
        clipboard_capture_enabled: state.clipboard_enabled.load(Ordering::SeqCst),
        clipboard_recording_types: *state.clipboard_recording_types.lock().unwrap(),
        clipboard_binding: clipboard_shortcut.binding,
        clipboard_shortcut_enabled: clipboard_shortcut.enabled,
        clipboard_controls: state.clipboard_options.lock().unwrap().controls(),
        window: state.windows.lock().unwrap().clone(),
    };
    Ok(snapshot)
}

#[tauri::command]
pub async fn launch_at_login_set(
    window: Webview,
    app: AppHandle,
    enabled: bool,
) -> Result<bool, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().launch_at_login {
        return Err("platform_unavailable".into());
    }
    let actual = on_main(&app, move |_| platform::set_launch_at_login(enabled)).await??;
    let _ = app.emit("launch_at_login_changed", actual);
    Ok(actual)
}

#[tauri::command]
pub fn window_show(window: Webview, app: AppHandle, target: String) -> Result<(), String> {
    authorize(
        &window,
        &["translation", "history", "clipboard", "settings"],
    )?;
    if target == "settings" {
        return crate::records_runtime::open_settings_window(&app);
    }
    windows::show(&app, &target)
}

#[tauri::command]
pub fn window_hide(
    window: Webview,
    app: AppHandle,
    region: String,
    keep_history: Option<bool>,
) -> Result<(), String> {
    if region == "settings" {
        authorize(&window, &["settings"])?;
    } else if ["history", "translation", "clipboard"].contains(&region.as_str()) {
        authorize(&window, &["translation", "history", "clipboard"])?;
    } else {
        return Err("invalid_argument".into());
    }
    if region != "settings" {
        let state = app.state::<AppState>();
        state.image_text_operation.fetch_add(1, Ordering::SeqCst);
        state.operation.fetch_add(1, Ordering::SeqCst);
    }
    if region == "translation" && keep_history == Some(true) {
        {
            let state = app.state::<AppState>();
            let mut state = state.windows.lock().unwrap();
            state.translation_visible = false;
            state.history_visible = true;
        }
        return windows::apply_translation(&app);
    }
    windows::hide(&app, &region)
}

#[tauri::command]
pub async fn translation_sample_copy(
    window: Webview,
    app: AppHandle,
    sample_id: String,
    part: String,
) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let allow_pending = window.label() == "translation";
    let text = on_main(&app, move |app| {
        let state = app.state::<AppState>();
        let pending = state.pending_translation.lock().unwrap();
        let text = if let Some(pending) = pending
            .as_ref()
            .filter(|draft| draft.record.id == sample_id)
        {
            if !allow_pending
                || pending.revision != state.translation_revision.load(Ordering::SeqCst)
            {
                return Err("invalid_argument".into());
            }
            pending
                .copy_part(&part)
                .ok_or("invalid_argument")?
                .to_owned()
        } else {
            let record = windows::history_record(&app, &sample_id).ok_or("invalid_argument")?;
            translation_copy_part(&record, &part)
                .ok_or("invalid_argument")?
                .to_owned()
        };
        let version = platform::write_text(&text).map_err(|(code, _)| code.to_owned())?;
        state.clipboard_version.store(version, Ordering::SeqCst);
        Ok::<_, String>(text)
    })
    .await??;
    match crate::clipboard_runtime::database(&app, move |db| {
        db.clipboard_insert_translation_copy(text)
    })
    .await
    {
        Ok(()) => {
            crate::clipboard_preferences::enforce_history_locked(&app).await;
            let _ = crate::clipboard_runtime::changed(&app);
        }
        Err(error) => eprintln!("Nami clipboard history after translation copy: {error}"),
    }
    Ok(())
}

fn translation_copy_part<'a>(
    record: &'a nami::model::HistoryRecord,
    part: &str,
) -> Option<&'a str> {
    match part {
        "source" => Some(record.text.as_str()),
        "primary" => record.translations.first().map(String::as_str),
        "secondary" => record
            .translations
            .get(1)
            .filter(|text| !text.is_empty())
            .or_else(|| record.translations.first())
            .map(String::as_str),
        _ => None,
    }
}

#[tauri::command]
pub fn window_pin(
    window: Webview,
    app: AppHandle,
    region: String,
    pinned: bool,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    match region.as_str() {
        "translation" | "history" => {
            authorize(&window, &["translation", "history", "clipboard"])?;
            {
                let mut windows = state.windows.lock().unwrap();
                if region == "translation" {
                    windows.translation_pinned = pinned
                } else {
                    windows.history_pinned = pinned;
                    windows.clipboard_pinned = pinned
                }
            }
            windows::update_translation_level(&app)
        }
        "clipboard" => {
            authorize(&window, &["clipboard", "translation"])?;
            {
                let mut windows = state.windows.lock().unwrap();
                windows.history_pinned = pinned;
                windows.clipboard_pinned = pinned;
            }
            windows::update_translation_level(&app)
        }
        _ => Err("invalid_argument".into()),
    }
}

#[tauri::command]
pub fn records_view_set(window: Webview, app: AppHandle, view: String) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    let state = app.state::<AppState>();
    state.operation.fetch_add(1, Ordering::SeqCst);
    windows::set_record_view(&app, &view)
}

#[tauri::command]
pub fn records_details_toggle(
    window: Webview,
    app: AppHandle,
    record_id: Option<String>,
) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    if record_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 160)
    {
        return Err("invalid_argument".into());
    }
    windows::toggle_record_details(&app, record_id)
}

#[tauri::command]
pub fn clipboard_details_select(
    window: Webview,
    app: AppHandle,
    record_id: Option<String>,
    browsing: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    if record_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 160)
    {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    {
        let mut regions = state.windows.lock().unwrap();
        if regions.record_view != nami::model::RecordView::Clipboard
            || !regions.history_visible
            || regions.records_editor_open
            || state.records_dirty.load(Ordering::SeqCst)
        {
            return Err("cancelled".into());
        }
        if let Some(browsing) = browsing {
            regions.records_browsing = browsing;
        } else if !regions.records_browsing {
            return Ok(());
        }
        regions.clipboard_detail_id = record_id;
    }
    windows::emit_state(&app)
}

#[tauri::command]
pub fn clipboard_note_open(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    if record_id.is_empty() || record_id.len() > 160 {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    let reveal = {
        let mut regions = state.windows.lock().unwrap();
        if regions.record_view != nami::model::RecordView::Clipboard
            || (!regions.history_visible && !regions.translation_visible)
            || regions.records_editor_open
            || state.records_dirty.load(Ordering::SeqCst)
        {
            return Err("cancelled".into());
        }
        let reveal = !regions.history_visible;
        regions.history_visible = true;
        reveal
    };
    if reveal {
        windows::apply_translation(&app)?;
    }
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    view.emit("clipboard_note_requested", record_id)
        .map_err(|_| "event_emit")?;
    windows::focus_records_editor(&app)
}

#[tauri::command]
pub fn records_editor_set(
    window: Webview,
    app: AppHandle,
    dirty: bool,
    open: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    let state = app.state::<AppState>();
    state.records_dirty.store(dirty, Ordering::SeqCst);
    if let Some(open) = open {
        let mut regions = state.windows.lock().unwrap();
        if regions.records_editor_open != open {
            regions.records_editor_open = open;
            drop(regions);
            windows::emit_state(&app)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn records_navigation_resolve(
    window: Webview,
    app: AppHandle,
    proceed: bool,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    crate::records_runtime::resolve_navigation(&app, proceed)
}

#[tauri::command]
pub fn history_toggle(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    crate::records_runtime::toggle_translation_history(&app)
}

#[tauri::command]
pub fn history_visibility_set(
    window: Webview,
    app: AppHandle,
    visible: bool,
) -> Result<(), String> {
    authorize(&window, &["translation", "history"])?;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.record_view = crate::model::RecordView::Translation;
        state.history_visible = visible;
        state.show_translation();
    }
    windows::apply_translation(&app)
}

#[tauri::command]
pub fn history_width_set(
    window: Webview,
    app: AppHandle,
    width: f64,
    initialize: bool,
) -> Result<(), String> {
    authorize(&window, &["translation", "history"])?;
    windows::resize_history(&app, width, initialize)
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranslationChromeState {
    source_label: String,
    source_width: f64,
}

#[tauri::command]
pub fn translation_chrome_update(
    window: Webview,
    app: AppHandle,
    state: TranslationChromeState,
) -> Result<bool, String> {
    authorize(&window, &["translation"])?;
    if !state.source_width.is_finite()
        || !(32.0..=360.0).contains(&state.source_width)
        || state.source_label.is_empty()
        || state.source_label.chars().count() > 100
        || state.source_label.contains(['\n', '\r'])
    {
        return Err("invalid_argument".into());
    }
    if !platform::capabilities().native_chrome {
        return Ok(false);
    }
    windows::update_translation_chrome(&app, state.source_label, state.source_width)?;
    Ok(true)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistorySelection {
    revision: u64,
    sample_id: Option<String>,
    retranslate: bool,
    clear_detail: bool,
    record: Option<crate::model::HistoryRecord>,
}

#[tauri::command]
pub fn history_snapshot(
    window: Webview,
    app: AppHandle,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    let state = app.state::<AppState>();
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let snapshot = state.history.lock().unwrap().wire_snapshot();
    Ok(snapshot)
}

#[tauri::command]
pub async fn history_query(
    window: Webview,
    app: AppHandle,
    query: String,
    favorites_only: bool,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    crate::history_runtime::query_history(&app, query, favorites_only).await
}

#[tauri::command]
pub async fn history_load_more(
    window: Webview,
    app: AppHandle,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    crate::history_runtime::load_more(&app).await
}

#[tauri::command]
pub async fn history_record_update(
    window: Webview,
    app: AppHandle,
    record_id: String,
    action: String,
    title: Option<String>,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    crate::history_runtime::update_record(&app, record_id, action, title).await
}

#[tauri::command]
pub fn history_record_note(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<serde_json::Value, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    let record = windows::history_record(&app, &record_id).ok_or("not_found")?;
    Ok(serde_json::json!({"title": record.title, "text": record.text, "kind": "text"}))
}

#[tauri::command]
pub fn history_sample_select(
    window: Webview,
    app: AppHandle,
    sample_id: Option<String>,
    retranslate: bool,
    clear_detail: Option<bool>,
    preview: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    if preview == Some(true)
        && !app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .records_browsing
    {
        return Ok(());
    }
    select_history_sample(&app, sample_id, retranslate, clear_detail.unwrap_or(false))
}

pub(crate) fn select_history_sample(
    app: &AppHandle,
    sample_id: Option<String>,
    retranslate: bool,
    clear_detail: bool,
) -> Result<(), String> {
    if clear_detail && sample_id.is_some() {
        return Err("invalid_argument".into());
    }
    let record = sample_id
        .as_ref()
        .and_then(|id| windows::history_record(app, id));
    if sample_id.is_some() && record.is_none() {
        return Err("invalid_argument".into());
    }
    let repeated = app
        .state::<AppState>()
        .history
        .lock()
        .unwrap()
        .selected_id()
        .map(str::to_owned)
        == sample_id;
    let revision = windows::select_native_history(app, sample_id.as_deref())?;
    if sample_id.is_some() {
        let visible = {
            let state = app.state::<AppState>();
            let mut windows = state.windows.lock().unwrap();
            windows.records_browsing = true;
            windows.select_history_record(repeated, retranslate);
            windows.translation_visible
        };
        if visible {
            windows::apply_translation(app)?;
            if let Some(view) = app.get_webview("translation") {
                view.window().show().map_err(|_| "window_show")?;
            }
        } else {
            windows::apply_translation(app)?;
        }
    }
    let selection = HistorySelection {
        revision,
        sample_id,
        retranslate,
        clear_detail,
        record,
    };
    for label in ["translation", "history", "clipboard"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("history_sample_selected", &selection)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn translation_columns_set(
    window: Webview,
    app: AppHandle,
    secondary_enabled: bool,
) -> Result<(), String> {
    authorize(&window, &["translation", "history"])?;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.secondary_enabled = secondary_enabled;
        state.translation_width = state.translation_width.max(state.translation_min_width());
    }
    windows::apply_translation(&app)
}

#[tauri::command]
pub fn locale_set(window: Webview, app: AppHandle, locale: String) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !["system", "zh-CN", "en"].contains(&locale.as_str()) {
        return Err("invalid_argument".into());
    }
    *app.state::<AppState>().locale.lock().unwrap() = locale.clone();
    for label in ["translation", "history", "clipboard", "settings"] {
        if let Some(window) = app.get_webview(label) {
            window
                .emit("locale_changed", &locale)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

#[tauri::command]
pub fn permission_request(window: Webview) -> Result<bool, String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().accessibility_permission {
        return Err("platform_unavailable".into());
    }
    Ok(platform::request_accessibility())
}

pub(crate) async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    f: impl FnOnce(AppHandle) -> T + Send + 'static,
) -> Result<T, String> {
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let owned = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(f(owned));
    })
    .map_err(|_| "main_thread_unavailable")?;
    receiver.await.map_err(|_| "main_thread_unavailable".into())
}

#[tauri::command]
pub async fn clipboard_record_use(
    window: Webview,
    app: AppHandle,
    record_id: String,
    action: String,
) -> Result<OperationResult, String> {
    authorize(&window, &["clipboard", "translation"])?;
    use crate::clipboard_preferences::Activation;
    use crate::clipboard_runtime::PayloadMode;
    let (action, mode) = {
        let state = app.state::<AppState>();
        let options = state.clipboard_options.lock().unwrap();
        let activation = match action.as_str() {
            "copy" => Activation::Copy,
            "paste" | "pastePlain" | "pastePaths" => Activation::Paste,
            "activate" => options.activation,
            _ => return Err("invalid_argument".into()),
        };
        let mode = match action.as_str() {
            "pastePlain" => PayloadMode::PlainText,
            "pastePaths" => PayloadMode::FilePaths,
            _ => PayloadMode::Restore {
                plain_text: matches!(activation, Activation::Paste) && options.paste_as_plain_text,
            },
        };
        (activation, mode)
    };
    let operation = on_main(&app, |app| {
        app.state::<AppState>()
            .operation
            .fetch_add(1, Ordering::SeqCst)
            + 1
    })
    .await?;
    let state = app.state::<AppState>();
    let write = state.clipboard_writes.lock().await;
    let used_id = record_id.clone();
    let payload = crate::clipboard_runtime::payload(&app, record_id, mode).await?;
    // Transcode TIFF before taking the main thread. PNG bytes are left unchanged.
    let payload =
        tauri::async_runtime::spawn_blocking(move || platform::prepare_image_paste(payload))
            .await
            .map_err(|_| "storage_worker".to_string())?;
    let image_paste = image_history_paste(&payload);
    let initial = on_main(&app, move |app| {
        let state = app.state::<AppState>();
        if state.operation.load(Ordering::SeqCst) != operation {
            return Err(OperationResult::failed(
                "cancelled",
                None,
                platform::clipboard_version(),
            ));
        }
        let version = match platform::write_clipboard(&payload) {
            Ok(version) => version,
            Err((code, version)) => {
                return Err(OperationResult::failed(
                    code,
                    version,
                    platform::clipboard_version(),
                ))
            }
        };
        state.clipboard_version.store(version, Ordering::SeqCst);
        if matches!(action, Activation::Copy) {
            return Err(OperationResult::copied());
        }
        let written_at = std::time::Instant::now();
        let Some(target) = *state.target.lock().unwrap() else {
            return Err(OperationResult::failed(
                "target_unavailable",
                Some(version),
                platform::clipboard_version(),
            ));
        };
        if !platform::accessibility() {
            return Err(OperationResult::failed(
                "permission_required",
                Some(version),
                platform::clipboard_version(),
            ));
        }
        if !platform::target_alive(target) {
            return Err(OperationResult::failed(
                "target_unavailable",
                Some(version),
                platform::clipboard_version(),
            ));
        }
        let return_session = if platform::capabilities().native_history
            && window.window().is_focused().unwrap_or(false)
        {
            pinned_clipboard_session(&state.windows.lock().unwrap())
        } else {
            None
        };
        if !state.windows.lock().unwrap().history_pinned {
            let _ = windows::hide_after_clipboard_use(&app);
        }
        if !platform::activate(target) {
            return Err(OperationResult::failed(
                "target_unavailable",
                Some(version),
                platform::clipboard_version(),
            ));
        }
        Ok((target, version, written_at, return_session))
    })
    .await?;
    let (target, version, written_at, return_session) = match initial {
        Ok(values) => {
            // Clicking a row writes the clipboard before it pastes. That write is the same copy.
            save_record_use(&app, used_id, false).await?;
            values
        }
        Err(result) => {
            if result.copy_status == "current" {
                save_record_use(&app, used_id, result.status == "copied").await?;
            }
            drop(write);
            reveal_failure(&app, operation, &result).await?;
            return Ok(result);
        }
    };
    drop(write);
    let mut activation = None;
    for _ in 0..30 {
        let attempt = on_main(&app, move |app| {
            let state = app.state::<AppState>();
            let current = platform::clipboard_version();
            let code = if state.operation.load(Ordering::SeqCst) != operation {
                Some("cancelled")
            } else if current != version {
                Some("clipboard_changed")
            } else if !platform::target_alive(target) {
                Some("target_unavailable")
            } else {
                None
            };
            if let Some(code) = code {
                return Some(Err(OperationResult::failed(code, Some(version), current)));
            }
            if !platform::target_focused(target) {
                return None;
            }
            // Chromium drops an image paste that arrives while its clipboard
            // notice is still in flight. The remainder waits off the main thread.
            let settle = remaining_image_settle(image_paste, written_at.elapsed());
            if !settle.is_zero() {
                return Some(Ok(PasteAttempt::Wait(settle)));
            }
            Some(match platform::paste_keys_down(target, version) {
                Ok(()) => Ok(PasteAttempt::Posted(platform::activation_epoch())),
                Err(code) => Err(OperationResult::failed(
                    code,
                    Some(version),
                    platform::clipboard_version(),
                )),
            })
        })
        .await?;
        match attempt {
            None => tokio::time::sleep(Duration::from_millis(10)).await,
            Some(Err(result)) => {
                reveal_failure(&app, operation, &result).await?;
                return Ok(result);
            }
            Some(Ok(PasteAttempt::Wait(delay))) => tokio::time::sleep(delay).await,
            Some(Ok(PasteAttempt::Posted(epoch))) => {
                activation = Some(epoch);
                break;
            }
        }
    }
    let Some(activation) = activation else {
        let result = on_main(&app, move |_| {
            OperationResult::failed(
                "focus_timeout",
                Some(version),
                platform::clipboard_version(),
            )
        })
        .await?;
        reveal_failure(&app, operation, &result).await?;
        return Ok(result);
    };
    // The gap stays off the main thread. Keyup runs even if focus or the
    // clipboard changed while it was waiting.
    tokio::time::sleep(PASTE_KEY_GAP).await;
    let (released, focused, current) = on_main(&app, move |_| {
        (
            platform::paste_keys_up(),
            platform::target_focused(target),
            platform::clipboard_version(),
        )
    })
    .await?;
    let result = if let Err(code) = released {
        OperationResult::failed(code, Some(version), current)
    } else {
        match paste_release_result(focused, current == version) {
            Ok(()) => OperationResult {
                status: "dispatched",
                copy_status: "current",
                code: None,
            },
            Err(code) => OperationResult::failed(code, Some(version), current),
        }
    };
    if let Some(session) = return_session.filter(|_| result.status == "dispatched") {
        let handoff = PasteFocusReturn {
            session,
            operation,
            activation,
        };
        // Let the target app consume Cmd+V before reclaiming pinned panel focus.
        tokio::time::sleep(PASTE_FOCUS_RETURN_DELAY).await;
        on_main(&app, move |app| {
            let state = app.state::<AppState>();
            let owns_focus = handoff.still_owned(
                &state.windows.lock().unwrap(),
                state.operation.load(Ordering::SeqCst),
                platform::activation_epoch(),
            );
            if owns_focus
                && platform::target_focused(target)
                && platform::clipboard_version() == version
            {
                let _ = windows::focus_records_editor(&app)
                    .inspect_err(|error| eprintln!("Nami clipboard focus return: {error}"));
            }
        })
        .await?;
    }
    reveal_failure(&app, operation, &result).await?;
    Ok(result)
}

async fn save_record_use(app: &AppHandle, id: String, copied: bool) -> Result<(), String> {
    crate::clipboard_runtime::database(app, move |database| {
        if copied {
            database.clipboard_duplicate(&id)
        } else {
            database.clipboard_promote(&id)
        }
    })
    .await?;
    if copied {
        crate::clipboard_preferences::enforce_history_locked(app).await;
    }
    crate::clipboard_runtime::changed(app)
}

async fn reveal_failure(
    app: &AppHandle,
    operation: u64,
    result: &OperationResult,
) -> Result<(), String> {
    if result.status != "failed" || result.code == Some("cancelled") {
        return Ok(());
    }
    let failure = serde_json::json!({ "status": result.status, "copyStatus": result.copy_status, "code": result.code });
    on_main(app, move |app| {
        if app.state::<AppState>().operation.load(Ordering::SeqCst) == operation {
            // Restore the list without recapturing the target or cancelling this operation.
            windows::reveal_clipboard_failure(&app)?;
            if let Some(view) = app
                .get_webview("clipboard")
                .or_else(|| app.get_webview("translation"))
            {
                view.emit("clipboard_use_failed", &failure)
                    .map_err(|_| "event_emit")?;
            }
        }
        Ok::<_, String>(())
    })
    .await?
}

#[tauri::command]
pub fn selection_take(
    window: Webview,
    app: AppHandle,
) -> Result<Option<crate::selection::SelectionContext>, String> {
    authorize(&window, &["translation"])?;
    let state = app.state::<AppState>();
    if state.records_dirty.load(Ordering::SeqCst)
        || state.records_navigation.lock().unwrap().is_some()
    {
        return Ok(None);
    }
    let context = state.selection.lock().unwrap().take();
    Ok(context)
}

#[tauri::command]
pub fn selection_present(
    window: Webview,
    app: AppHandle,
    presentation_id: u64,
) -> Result<(), String> {
    authorize(&window, &["translation"])?;
    crate::selection::present(&app, presentation_id)
}

#[tauri::command]
pub fn translation_cancel(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["translation"])?;
    let state = app.state::<AppState>();
    state.translation_revision.fetch_add(1, Ordering::SeqCst);
    state.pending_translation.lock().unwrap().take();
    crate::records_runtime::protect_result(&app)
}

async fn store_preferences(
    app: &AppHandle,
    preferences: crate::translation::Preferences,
) -> Result<(), String> {
    let entries = preferences.entries();
    crate::clipboard_runtime::database(app, move |database| {
        for (key, value) in &entries {
            database.preference_set(key, value)?;
        }
        Ok(())
    })
    .await
}

fn publish_preferences(
    app: &AppHandle,
    preferences: &crate::translation::Preferences,
) -> Result<(), String> {
    for label in ["translation", "settings", "clipboard", "history"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("translation_preferences_changed", preferences)
                .map_err(|_| "event_emit")?;
            if matches!(label, "translation" | "settings") {
                view.emit("translation_engine_changed", &preferences.engine)
                    .map_err(|_| "event_emit")?;
            }
        }
    }
    Ok(())
}

// Keep the active key registered until macOS accepts the replacement and storage succeeds.
async fn stage_global_shortcuts(
    app: &AppHandle,
    next: impl IntoIterator<Item = tauri_plugin_global_shortcut::Shortcut>,
) -> Result<Vec<tauri_plugin_global_shortcut::Shortcut>, String> {
    let next: Vec<_> = next.into_iter().collect();
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    on_main(app, move |app| {
        let mut staged = Vec::new();
        for shortcut in next {
            if app.global_shortcut().is_registered(shortcut) {
                continue;
            }
            if app.global_shortcut().register(shortcut).is_err() {
                for shortcut in staged {
                    let _ = app.global_shortcut().unregister(shortcut);
                }
                return Err("shortcut_registration".into());
            }
            staged.push(shortcut);
        }
        Ok(staged)
    })
    .await?
}

async fn commit_preferences(
    app: &AppHandle,
    preferences: crate::translation::Preferences,
    engine_changed: bool,
) -> Result<(), String> {
    store_preferences(app, preferences.clone()).await?;
    on_main(app, move |app| {
        let state = app.state::<AppState>();
        *state.translation_engine.lock().unwrap() = preferences.engine.clone();
        *state.translation_preferences.lock().unwrap() = preferences.clone();
        if engine_changed {
            state.translation_revision.fetch_add(1, Ordering::SeqCst);
            state.pending_translation.lock().unwrap().take();
        }
        publish_preferences(&app, &preferences)?;
        crate::sync_global_shortcuts(&app);
        Ok(())
    })
    .await?
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn translation_preferences_set(
    window: Webview,
    app: AppHandle,
    engine: Option<String>,
    source_language: Option<String>,
    primary_language: Option<String>,
    secondary_language: Option<String>,
    proxy_url: Option<String>,
    proxy_enabled: Option<bool>,
    selection_enabled: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["settings", "translation"])?;
    let patch = crate::translation::PreferencePatch {
        engine,
        source_language,
        primary_language,
        secondary_language,
        proxy_url,
        proxy_enabled,
        selection_enabled,
    };
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let mut preferences = state.translation_preferences.lock().unwrap().clone();
    let engine_changed = preferences.apply(patch)?;
    commit_preferences(&app, preferences, engine_changed).await
}

#[tauri::command]
pub async fn translation_shortcut_set(
    window: Webview,
    app: AppHandle,
    action: String,
    binding: Option<crate::shortcuts::KeyBinding>,
    enabled: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let mut preferences = state.translation_preferences.lock().unwrap().clone();
    let shortcut_action = crate::shortcuts::ShortcutAction::parse(&action)?;
    if enabled.is_some() && binding.is_some() {
        return Err("invalid_argument".into());
    }
    let candidate = if enabled == Some(true) {
        preferences.shortcuts.get(shortcut_action)
    } else {
        binding.as_ref()
    };
    if candidate.is_some_and(|binding| {
        state
            .clipboard_options
            .lock()
            .unwrap()
            .record_shortcuts
            .conflicts_with_translation(shortcut_action, binding)
    }) {
        return Err("shortcut_clipboard_panel".into());
    }
    if enabled != Some(false) {
        let binding = if enabled.is_some() {
            candidate.cloned()
        } else {
            binding
        };
        let clipboard = state.clipboard_shortcut.lock().unwrap().binding.clone();
        preferences.set_shortcut(&action, binding, clipboard.as_ref())?;
    }
    if let Some(enabled) = enabled {
        preferences.set_shortcut_enabled(&action, enabled)?;
    }
    if !shortcut_action.is_global() || !platform::capabilities().selection {
        return commit_preferences(&app, preferences, false).await;
    }
    let next = preferences
        .global_binding(shortcut_action)
        .map(crate::binding_shortcut)
        .transpose()?;
    let staged = stage_global_shortcuts(&app, next).await?;
    if let Err(error) = store_preferences(&app, preferences.clone()).await {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let _ = on_main(&app, move |app| {
            for shortcut in staged {
                let _ = app.global_shortcut().unregister(shortcut);
            }
        })
        .await;
        return Err(error);
    }
    on_main(&app, move |app| {
        let state = app.state::<AppState>();
        state.registered_shortcuts.lock().unwrap().extend(staged);
        *state.translation_preferences.lock().unwrap() = preferences.clone();
        crate::sync_global_shortcuts(&app);
        publish_preferences(&app, &preferences)
    })
    .await?
}

#[tauri::command]
pub async fn translation_shortcuts_restore(
    window: Webview,
    app: AppHandle,
    general: Option<bool>,
) -> Result<(), String> {
    use crate::shortcuts::is_general_window_action;
    authorize(&window, &["settings"])?;
    let general = general.unwrap_or(false);
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let mut preferences = state.translation_preferences.lock().unwrap().clone();
    let current_records = state
        .clipboard_options
        .lock()
        .unwrap()
        .record_shortcuts
        .clone();
    let records = general
        .then(|| current_records.restore_general())
        .transpose()?;
    let clipboard = state.clipboard_shortcut.lock().unwrap().binding.clone();
    preferences.shortcuts = preferences.shortcuts.restore_group(
        general,
        platform::capabilities().primary_modifier,
        clipboard.as_ref(),
    )?;
    preferences
        .disabled_shortcuts
        .retain(|action| is_general_window_action(*action) != general);
    if !general {
        preferences.selection_enabled = true;
    }
    let next_records = records.as_ref().unwrap_or(&current_records);
    if preferences.shortcuts.conflicts_with_records(next_records)
        || clipboard
            .as_ref()
            .is_some_and(|binding| next_records.conflicts(binding, None))
    {
        return Err("shortcut_clipboard_panel".into());
    }
    let json =
        |value: Result<String, serde_json::Error>| value.map_err(|_| "storage_record".to_owned());
    let bindings = json(serde_json::to_string(&preferences.shortcuts))?;
    let disabled = json(serde_json::to_string(&preferences.disabled_shortcuts))?;
    let next_records_json = records
        .as_ref()
        .map(|value| json(serde_json::to_string(value)))
        .transpose()?;
    let selection_enabled = preferences.selection_enabled;
    use crate::shortcuts::ShortcutAction::{Selection, ToggleHistory};
    let next = [Selection, ToggleHistory]
        .into_iter()
        .filter(|_| !general && platform::capabilities().selection)
        .filter_map(|action| preferences.global_binding(action))
        .map(crate::binding_shortcut)
        .collect::<Result<Vec<_>, _>>()?;
    let staged = stage_global_shortcuts(&app, next).await?;
    let saved = crate::clipboard_runtime::database(&app, move |db| {
        db.reset_shortcut_group(
            &bindings,
            &disabled,
            selection_enabled,
            next_records_json.as_deref(),
        )
    })
    .await;
    on_main(&app, move |app| {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let state = app.state::<AppState>();
        if let Err(error) = saved {
            for shortcut in staged {
                let _ = app.global_shortcut().unregister(shortcut);
            }
            return Err(error);
        }
        state.registered_shortcuts.lock().unwrap().extend(staged);
        *state.translation_preferences.lock().unwrap() = preferences.clone();
        if let Some(records) = records {
            state.clipboard_options.lock().unwrap().record_shortcuts = records;
            crate::clipboard_preferences::publish_controls(&app, &state);
        }
        if !general {
            crate::sync_global_shortcuts(&app);
        }
        publish_preferences(&app, &preferences)
    })
    .await?
}

#[tauri::command]
pub async fn translation_shortcut_capture(
    window: Webview,
    app: AppHandle,
    active: bool,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    on_main(&app, move |app| {
        #[cfg(target_os = "macos")]
        crate::shortcut_capture::set_active(&app, active);
        app.state::<AppState>()
            .shortcuts_suspended
            .store(active, Ordering::SeqCst);
        crate::sync_global_shortcuts(&app);
        Ok(())
    })
    .await?
}

#[tauri::command]
pub fn translation_engine_set(
    window: Webview,
    app: AppHandle,
    engine: String,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !["edge", "google"].contains(&engine.as_str()) {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    state.translation_preferences.lock().unwrap().engine = engine.clone();
    *state.translation_engine.lock().unwrap() = engine.clone();
    let preferences = state.translation_preferences.lock().unwrap().clone();
    state.translation_revision.fetch_add(1, Ordering::SeqCst);
    state.pending_translation.lock().unwrap().take();
    let app_for_store = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = store_preferences(&app_for_store, preferences).await;
    });
    for label in ["translation", "settings"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("translation_engine_changed", &engine)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

#[derive(Serialize)]
#[serde(untagged)]
pub enum TranslationRunOutput {
    Complete(Box<crate::model::HistoryRecord>),
    Partial { partial: PartialTranslationView },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialTranslationView {
    id: String,
    translations: Vec<String>,
    source_language: String,
    errors: Vec<Option<String>>,
}

#[derive(Clone)]
pub(crate) struct PendingTranslation {
    record: crate::model::HistoryRecord,
    errors: Vec<Option<String>>,
    focus_origin: Option<crate::history_runtime::TranslationFocus>,
    revision: u64,
    pub(crate) retrying: bool,
}

impl PendingTranslation {
    fn copy_part(&self, part: &str) -> Option<&str> {
        let index = match part {
            "primary" => 0,
            "secondary" => 1,
            _ => return None,
        };
        if self.errors.get(index)?.is_some() {
            return None;
        }
        self.record
            .translations
            .get(index)
            .map(String::as_str)
            .filter(|text| !text.is_empty())
    }
}

pub(crate) enum TranslationRunMode {
    Foreground {
        reuse: Option<crate::translation::TranslationReuse>,
        retry_id: Option<String>,
        retry_engine: Option<String>,
        clipboard_session: Option<u64>,
    },
    Background {
        revision: u64,
    },
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn translation_run(
    window: Webview,
    app: AppHandle,
    text: String,
    source: String,
    targets: Vec<String>,
    from_selection: bool,
    reuse: Option<crate::translation::TranslationReuse>,
    retry_id: Option<String>,
    retry_engine: Option<String>,
    clipboard_session: Option<u64>,
) -> Result<TranslationRunOutput, String> {
    authorize(&window, &["translation"])?;
    if (retry_id.is_some() || clipboard_session.is_some()) && reuse.is_some()
        || clipboard_session.is_some() && !from_selection
    {
        return Err("invalid_argument".into());
    }
    let captured = if from_selection {
        app.state::<AppState>()
            .selection_source
            .lock()
            .unwrap()
            .clone()
    } else {
        None
    };
    run_translation(
        app,
        text,
        source,
        targets,
        captured,
        TranslationRunMode::Foreground {
            reuse,
            retry_id,
            retry_engine,
            clipboard_session,
        },
    )
    .await
}

fn reserve_translation(
    counter: &std::sync::atomic::AtomicU64,
    expected: Option<u64>,
) -> Result<u64, String> {
    match expected {
        Some(revision) => counter
            .compare_exchange(revision, revision + 1, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| revision + 1)
            .map_err(|_| "cancelled".into()),
        None => Ok(counter.fetch_add(1, Ordering::SeqCst) + 1),
    }
}

pub(crate) async fn run_translation(
    app: AppHandle,
    text: String,
    source: String,
    targets: Vec<String>,
    captured: Option<serde_json::Value>,
    mode: TranslationRunMode,
) -> Result<TranslationRunOutput, String> {
    let (reuse, retry_id, retry_engine, clipboard_session, expected_revision) = match mode {
        TranslationRunMode::Foreground {
            reuse,
            retry_id,
            retry_engine,
            clipboard_session,
        } => (reuse, retry_id, retry_engine, clipboard_session, None),
        TranslationRunMode::Background { revision } => (None, None, None, None, Some(revision)),
    };
    if let Some(error) = app
        .state::<AppState>()
        .storage_error
        .lock()
        .unwrap()
        .clone()
    {
        return Err(error);
    }
    crate::translation::validate_text(&text)?;
    #[cfg(feature = "ui-tests")]
    let local_fixture = if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
        if app.config().identifier != "dev.local.nami.ui-tests"
            || !matches!(
                std::env::var("NAMI_UI_TEST_CASE").as_deref(),
                Ok("translation-partial" | "clipboard-translate-shortcut")
            )
        {
            return Err("translation_network_disabled_in_tests".into());
        }
        true
    } else {
        false
    };
    let state = app.state::<AppState>();
    let (preferences, engine, revision, retried) = {
        let preferences = state.translation_preferences.lock().unwrap();
        let mut pending = state.pending_translation.lock().unwrap();
        let (engine, revision, draft) = if let Some(id) = retry_id.as_deref() {
            let draft = pending
                .as_mut()
                .filter(|draft| {
                    let record = &draft.record;
                    !draft.retrying
                        && record.id == id
                        && record.text == text
                        && record.target_languages == targets
                        && record
                            .translation
                            .as_ref()
                            .is_some_and(|metadata| metadata.requested_source == source)
                })
                .ok_or("invalid_argument")?;
            let engine = retry_engine
                .unwrap_or_else(|| draft.record.translation.as_ref().unwrap().engine.clone());
            let missing = targets
                .iter()
                .zip(&draft.errors)
                .filter_map(|(target, error)| error.as_ref().map(|_| target.clone()))
                .collect::<Vec<_>>();
            crate::translation::validate_options(&engine, &source, &missing)?;
            let revision = reserve_translation(&state.translation_revision, Some(draft.revision))?;
            draft.revision = revision;
            draft.retrying = true;
            (engine, revision, Some(draft.clone()))
        } else {
            let engine = retry_engine.unwrap_or_else(|| preferences.engine.clone());
            crate::translation::validate_options(&engine, &source, &targets)?;
            let revision = reserve_translation(&state.translation_revision, expected_revision)?;
            pending.take();
            (engine, revision, None)
        };
        (preferences.clone(), engine, revision, draft)
    };
    let proxy_url = preferences.proxy_url;
    let proxy_enabled = preferences.proxy_enabled;
    let focus_origin = if let Some(draft) = &retried {
        draft.focus_origin.clone()
    } else {
        reuse
            .as_ref()
            .filter(|reuse| reuse.focus_history)
            .map(|reuse| crate::history_runtime::TranslationFocus::History(reuse.record_id.clone()))
            .or_else(|| clipboard_session.map(crate::history_runtime::TranslationFocus::Clipboard))
    };
    let previous_history = reuse
        .as_ref()
        .and_then(|reuse| state.history.lock().unwrap().record(&reuse.record_id));
    let previous = retried
        .as_ref()
        .map(|draft| &draft.record)
        .or(previous_history.as_ref());
    let retained = if let Some(draft) = &retried {
        draft
            .record
            .translations
            .iter()
            .zip(&draft.errors)
            .map(|(text, error)| error.is_none().then(|| text.clone()))
            .collect()
    } else {
        crate::translation::reusable_results(
            &preferences.engine,
            &text,
            &source,
            &targets,
            previous,
            reuse.as_ref().and_then(|reuse| reuse.target_index),
        )
    };
    let missing: Vec<String> = targets
        .iter()
        .zip(&retained)
        .filter(|(_, result)| result.is_none())
        .map(|(target, _)| target.clone())
        .collect();
    let source_app = crate::translation::source_app_for_translation(&text, previous, captured);
    let translated_result = if missing.is_empty() {
        crate::translation::TranslationResult {
            translations: Vec::new(),
            errors: Vec::new(),
            source_language: source.clone(),
            detections: Vec::new(),
        }
    } else {
        let request = async {
            #[cfg(feature = "ui-tests")]
            if local_fixture {
                return partial_translation_fixture(&engine, &text, &missing).await;
            }
            crate::translation::translate(
                &engine,
                &text,
                &source,
                &missing,
                &proxy_url,
                proxy_enabled,
            )
            .await
        };
        tokio::pin!(request);
        loop {
            tokio::select! {
                result = &mut request => break match result {
                    Ok(output) => output,
                    Err(error) if retained.iter().any(Option::is_some) => crate::translation::TranslationResult {
                        translations: vec![String::new(); missing.len()],
                        errors: vec![Some(error); missing.len()],
                        source_language: source.clone(),
                        detections: Vec::new(),
                    },
                    Err(error) => return Err(error),
                },
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                    if state.translation_revision.load(Ordering::SeqCst) != revision { return Err("cancelled".into()); }
                }
            }
        }
    };
    let previous_detection = retained.first().is_some_and(Option::is_some)
        || (retained.iter().any(Option::is_some)
            && translated_result.errors.iter().all(Option::is_some));
    let detections = if previous_detection {
        previous
            .as_ref()
            .and_then(|record| record.translation.as_ref())
            .map(|metadata| metadata.detections.clone())
            .unwrap_or_default()
    } else {
        translated_result.detections
    };
    let resolved_source = if previous_detection {
        previous
            .as_ref()
            .map(|record| record.source_language.clone())
            .unwrap_or_else(|| source.clone())
    } else {
        translated_result.source_language
    };
    let mut target_engines = retained
        .iter()
        .enumerate()
        .map(|(index, result)| {
            if let Some(metadata) = previous
                .and_then(|record| record.translation.as_ref())
                .filter(|_| result.is_some())
            {
                metadata
                    .target_engines
                    .get(index)
                    .unwrap_or(&metadata.engine)
                    .clone()
            } else {
                engine.clone()
            }
        })
        .collect::<Vec<_>>();
    if target_engines
        .iter()
        .all(|target_engine| target_engine == &engine)
    {
        target_engines.clear();
    }
    let mut fresh = translated_result
        .translations
        .into_iter()
        .zip(translated_result.errors);
    let (translated, errors): (Vec<String>, Vec<Option<String>>) = retained
        .into_iter()
        .map(|cached| {
            if let Some(text) = cached {
                Ok((text, None))
            } else {
                fresh.next().ok_or("translation_result")
            }
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .unzip();
    let record = crate::model::HistoryRecord {
        title: String::new(),
        id: uuid::Uuid::new_v4().to_string(),
        text,
        source_language: resolved_source,
        translation: Some(crate::model::TranslationMetadata {
            requested_source: source,
            engine: engine.clone(),
            target_engines,
            strategy_version: crate::translation::STRATEGY_VERSION,
            detections,
        }),
        target_languages: targets,
        translations: translated,
        date: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        is_favorite: false,
        is_pinned: false,
        source_app,
    };
    if let Some(error) = errors.iter().flatten().next() {
        if expected_revision.is_some() || errors.iter().all(Option::is_some) {
            return Err(error.clone());
        }
        let mut pending = state.pending_translation.lock().unwrap();
        if state.translation_revision.load(Ordering::SeqCst) != revision {
            return Err("cancelled".into());
        }
        let partial = PartialTranslationView {
            id: record.id.clone(),
            translations: record.translations.clone(),
            source_language: record.source_language.clone(),
            errors: errors.clone(),
        };
        *pending = Some(PendingTranslation {
            record,
            errors,
            focus_origin,
            revision,
            retrying: false,
        });
        return Ok(TranslationRunOutput::Partial { partial });
    }
    let committed =
        crate::history_runtime::commit_translation(&app, record, engine, revision, focus_origin)
            .await;
    if retried.is_some() {
        let mut pending = state.pending_translation.lock().unwrap();
        if let Some(draft) = pending.as_mut().filter(|draft| draft.revision == revision) {
            if committed.is_ok() || state.translation_revision.load(Ordering::SeqCst) != revision {
                pending.take();
            } else {
                draft.retrying = false;
            }
        }
    }
    committed.map(|record| TranslationRunOutput::Complete(Box::new(record)))
}

#[cfg(feature = "ui-tests")]
async fn partial_translation_fixture(
    engine: &str,
    text: &str,
    targets: &[String],
) -> Result<crate::translation::TranslationResult, String> {
    let failed = match text {
        "Synthetic split primary" | "Synthetic held retry" => Some("ja"),
        "Synthetic split secondary" => Some("en"),
        "Synthetic split total failure" | "Synthetic clipboard Command-T" => None,
        _ => return Err("unexpected_fixture_request".into()),
    };
    if targets.len() == 1 && failed.is_some() {
        if failed != targets.first().map(String::as_str) {
            return Err("retry_requested_wrong_column".into());
        }
        if text == "Synthetic held retry" {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }
    let (translations, errors) = targets
        .iter()
        .map(|target| {
            if engine == "edge"
                && (text == "Synthetic split total failure"
                    || (targets.len() == 2 && Some(target.as_str()) == failed))
            {
                (String::new(), Some("Synthetic service unavailable".into()))
            } else {
                (
                    format!(
                        "Synthetic {}translation {target}",
                        if engine == "google" { "Google " } else { "" }
                    ),
                    None,
                )
            }
        })
        .unzip();
    Ok(crate::translation::TranslationResult {
        translations,
        errors,
        source_language: "en".into(),
        detections: Vec::new(),
    })
}

#[tauri::command]
pub async fn clipboard_query(
    window: Webview,
    app: AppHandle,
    query: String,
    favorites_only: bool,
    kind: Option<String>,
    cursor: Option<nami::clipboard::ClipboardCursor>,
) -> Result<nami::clipboard::ClipboardQueryResult, String> {
    authorize(&window, &["clipboard", "translation"])?;
    let search = app
        .state::<AppState>()
        .clipboard_options
        .lock()
        .unwrap()
        .search;
    crate::clipboard_runtime::database(&app, move |database| {
        let page = database.clipboard_page_with_search(
            &query,
            favorites_only,
            kind.as_deref(),
            cursor.as_ref(),
            crate::database::PAGE_SIZE,
            search,
        )?;
        let alternate = if cursor.is_none()
            && (query.trim().is_empty()
                || search.mode == nami::clipboard::ClipboardSearchMode::Exact)
        {
            Some(database.clipboard_page_with_search(
                &query,
                !favorites_only,
                kind.as_deref(),
                None,
                crate::database::PAGE_SIZE,
                search,
            )?)
        } else {
            None
        };
        Ok(nami::clipboard::ClipboardQueryResult { page, alternate })
    })
    .await
}

#[tauri::command]
pub async fn clipboard_record_detail(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<nami::clipboard::ClipboardDetail, String> {
    authorize(&window, &["clipboard", "translation"])?;
    let status_app = app.clone();
    let (detail, measure_files) = crate::clipboard_runtime::database(&app, move |database| {
        let mut detail = database.clipboard_detail(&record_id)?;
        if detail.kind == "image" && platform::capabilities().image_ocr {
            let state = status_app.state::<AppState>();
            let jobs = state.image_index_jobs.lock().unwrap();
            detail.image_index_status = Some(match database.clipboard_ocr_text(&record_id)? {
                Some(text) if text.trim().is_empty() => "empty",
                Some(_) => "indexed",
                None => jobs.status(&record_id),
            });
        }
        let measure_files = database
            .clipboard_file_representation(&record_id)?
            .is_none();
        Ok((detail, measure_files))
    })
    .await?;
    let root = crate::clipboard_runtime::image_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::clipboard_detail::enrich(detail, &root, measure_files)
    })
    .await
    .map_err(|_| "storage_worker")?
}

#[tauri::command]
pub async fn clipboard_image_save(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<bool, String> {
    authorize(&window, &["clipboard", "translation"])?;
    if !platform::capabilities().clipboard_image_save {
        return Err("platform_unavailable".into());
    }
    let png = crate::clipboard_runtime::image_export_png(&app, record_id).await?;
    let app_data = app
        .path()
        .app_data_dir()
        .map_err(|_| "storage_unavailable")?;
    let parent = window.window();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    on_main(&app, move |_| {
        platform::pick_image_save_path(&parent, move |result| {
            let _ = sender.send(result);
        })
    })
    .await??;
    let Some(destination) = receiver.await.map_err(|_| "clipboard_save_dialog")?? else {
        return Ok(false);
    };
    tauri::async_runtime::spawn_blocking(move || {
        crate::clipboard_runtime::save_image_export(&app_data, &destination, png)
    })
    .await
    .map_err(|_| "storage_worker")??;
    Ok(true)
}

#[tauri::command]
pub async fn clipboard_open_target(
    window: Webview,
    app: AppHandle,
    record_id: String,
    file_index: Option<usize>,
    execute: bool,
) -> Result<Option<&'static str>, String> {
    use nami::clipboard::open_target::{self, OpenTarget};
    authorize(&window, &["clipboard", "translation"])?;
    if !platform::capabilities().clipboard_open_target {
        return Err("platform_unavailable".into());
    }
    let record = crate::clipboard_runtime::database(&app, move |database| {
        database.clipboard_record(&record_id)
    })
    .await?;
    let target =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<OpenTarget>, String> {
            let target = open_target::resolve(&record.kind, &record.text, file_index)?;
            if let Some(OpenTarget::File(path)) = &target {
                if (execute || record.kind == "text") && !path.exists() {
                    return if execute {
                        Err("clipboard_file_missing".into())
                    } else {
                        Ok(None)
                    };
                }
            }
            Ok(target)
        })
        .await
        .map_err(|_| "storage_worker")??;
    let Some(target) = target else {
        return Ok(None);
    };
    let kind = target.kind();
    if execute && target != OpenTarget::ChooseFile {
        if matches!(target, OpenTarget::File(_)) {
            // Revealing runs a nested AppKit event loop, which must not run inside a
            // window event handler callback. The platform queues it to the main thread;
            // the command returns before Finder answers.
            platform::open_clipboard_target(target)?;
        } else {
            on_main(&app, |_| platform::open_clipboard_target(target)).await??;
        }
    }
    Ok(Some(kind))
}

#[tauri::command]
pub async fn clipboard_record_has_text(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<bool, String> {
    authorize(&window, &["clipboard", "translation"])?;
    crate::clipboard_text::has_text(&app, record_id).await
}

#[tauri::command]
pub async fn clipboard_record_process(
    window: Webview,
    app: AppHandle,
    record_id: String,
    action: String,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    crate::clipboard_text::process(&app, record_id, action).await
}

#[tauri::command]
pub fn clipboard_translation_take(
    window: Webview,
    app: AppHandle,
) -> Result<Option<crate::selection::SelectionContext>, String> {
    authorize(&window, &["translation"])?;
    let state = app.state::<AppState>();
    let context = state.clipboard_translation.lock().unwrap().take();
    if let Some(context) = &context {
        *state.selection_source.lock().unwrap() = context.source_app.clone();
    }
    Ok(context)
}

#[tauri::command]
pub async fn clipboard_record_update(
    window: Webview,
    app: AppHandle,
    record_id: String,
    action: String,
    title: Option<String>,
    text: Option<String>,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let root = crate::clipboard_runtime::image_root(&app)?;
    let deleting = action == "delete";
    crate::clipboard_runtime::database(&app, move |database| {
        if action == "edit" {
            database.clipboard_edit(
                &record_id,
                title.as_deref().ok_or("invalid_argument")?,
                text.as_deref(),
            )?;
        } else {
            if title.is_some() || text.is_some() {
                return Err("invalid_argument".into());
            }
            database.clipboard_update(&record_id, &action)?;
        }
        Ok(())
    })
    .await?;
    crate::clipboard_runtime::changed(&app)?;
    if deleting && crate::clipboard_preferences::retry_image_deletions_locked(&app, root).await? {
        return Err("clipboard_cleanup_failed".into());
    }
    Ok(())
}

#[tauri::command]
pub async fn clipboard_recording_type_set(
    window: Webview,
    app: AppHandle,
    kind: nami::clipboard::RecordingKind,
    enabled: bool,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let mut types = *state.clipboard_recording_types.lock().unwrap();
    let was_recording_any_type = types.any();
    types.set(kind, enabled);
    let value = serde_json::to_string(&types).map_err(|_| "storage_record")?;
    crate::clipboard_runtime::database(&app, move |database| {
        database.preference_set("clipboard_recording_types", &value)
    })
    .await?;
    on_main(&app, move |app| {
        let state = app.state::<AppState>();
        if !was_recording_any_type && types.any() {
            state
                .clipboard_version
                .store(platform::clipboard_version(), Ordering::SeqCst);
        }
        *state.clipboard_recording_types.lock().unwrap() = types;
    })
    .await?;
    let _ = window.emit_to("settings", "clipboard_recording_types_changed", types);
    Ok(())
}

#[tauri::command]
pub async fn clipboard_shortcut_set(
    window: Webview,
    app: AppHandle,
    binding: Option<crate::shortcuts::KeyBinding>,
    enabled: Option<bool>,
    restore: Option<bool>,
) -> Result<(), String> {
    use crate::shortcuts::{clipboard_binding, validate_clipboard_binding};
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    authorize(&window, &["settings"])?;
    if !platform::capabilities().selection {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let mut shortcut = state.clipboard_shortcut.lock().unwrap().clone();
    let defaults = if restore.unwrap_or(false) {
        Some(
            state
                .clipboard_options
                .lock()
                .unwrap()
                .record_shortcuts
                .restore_clipboard()?,
        )
    } else {
        None
    };
    if defaults.is_some() && (binding.is_some() || enabled.is_some()) {
        return Err("invalid_argument".into());
    }
    let (key, value) = if let Some(enabled) = enabled {
        if binding.is_some() {
            return Err("invalid_argument".into());
        }
        if let Some(binding) = shortcut.binding.as_ref().filter(|_| enabled) {
            let preferences = state.translation_preferences.lock().unwrap();
            validate_clipboard_binding(binding.clone(), &preferences.shortcuts)?;
            crate::shortcuts::validate_quick_paste_reservation(binding)?;
            if state
                .clipboard_options
                .lock()
                .unwrap()
                .record_shortcuts
                .conflicts(binding, None)
            {
                return Err("shortcut_clipboard_panel".into());
            }
        }
        shortcut.enabled = enabled;
        ("clipboard_shortcut_enabled", enabled.to_string())
    } else {
        let preferences = state.translation_preferences.lock().unwrap();
        shortcut.binding = defaults
            .as_ref()
            .map(|_| clipboard_binding())
            .or(binding)
            .map(|binding| {
                let binding = validate_clipboard_binding(binding, &preferences.shortcuts)?;
                crate::shortcuts::validate_quick_paste_reservation(&binding)?;
                Ok::<_, String>(binding)
            })
            .transpose()?;
        let options = state.clipboard_options.lock().unwrap();
        let records = defaults.as_ref().unwrap_or(&options.record_shortcuts);
        if shortcut
            .binding
            .as_ref()
            .is_some_and(|binding| records.conflicts(binding, None))
            || defaults.is_some() && preferences.shortcuts.conflicts_with_records(records)
        {
            return Err("shortcut_clipboard_panel".into());
        }
        if defaults.is_some() {
            shortcut.enabled = true;
        }
        (
            "clipboard_shortcut",
            serde_json::to_string(&shortcut.binding).map_err(|_| "storage_record")?,
        )
    };
    let next = if shortcut.enabled {
        shortcut
            .binding
            .as_ref()
            .map(crate::binding_shortcut)
            .transpose()?
    } else {
        None
    };
    let defaults_json = defaults
        .as_ref()
        .map(serde_json::to_string)
        .transpose()
        .map_err(|_| "storage_record")?;
    let staged = stage_global_shortcuts(&app, next).await?;
    let saved = crate::clipboard_runtime::database(&app, move |database| {
        if let Some(records) = defaults_json {
            database.reset_clipboard_shortcuts(&value, &records)
        } else {
            database.preference_set(key, &value)
        }
    })
    .await;
    let result = shortcut.clone();
    on_main(&app, move |app| {
        let state = app.state::<AppState>();
        if let Err(error) = saved {
            for shortcut in staged {
                let _ = app.global_shortcut().unregister(shortcut);
            }
            return Err(error);
        }
        state.registered_shortcuts.lock().unwrap().extend(staged);
        *state.clipboard_shortcut.lock().unwrap() = shortcut;
        if let Some(defaults) = defaults {
            state.clipboard_options.lock().unwrap().record_shortcuts = defaults;
            crate::clipboard_preferences::publish_controls(&app, &state);
        }
        crate::sync_global_shortcuts(&app);
        Ok(())
    })
    .await??;
    let _ = app.emit(
        "clipboard_binding_changed",
        serde_json::json!({
            "binding": result.binding,
            "enabled": result.enabled,
            "shortcutErrors": state.shortcut_errors.lock().unwrap().clone(),
        }),
    );
    Ok(())
}

#[derive(Serialize)]
pub struct ClipboardImage {
    bytes: Vec<u8>,
    mime: &'static str,
}

#[tauri::command]
pub async fn clipboard_files(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<Vec<nami::clipboard::FileEntry>, String> {
    authorize(&window, &["clipboard", "translation"])?;
    let record = crate::clipboard_runtime::database(&app, move |database| {
        database.clipboard_record(&record_id)
    })
    .await?;
    if record.kind != "files" {
        return Err("invalid_argument".into());
    }
    tauri::async_runtime::spawn_blocking(move || {
        let mut entries = nami::clipboard::file_entries(&record.text)?;
        for entry in &mut entries {
            entry.icon_data_url = platform::file_icon_data_url(&entry.path);
        }
        Ok(entries)
    })
    .await
    .map_err(|_| "storage_worker")?
}

#[tauri::command]
pub async fn clipboard_image(
    window: Webview,
    app: AppHandle,
    record_id: String,
    max_pixels: Option<u32>,
) -> Result<ClipboardImage, String> {
    authorize(&window, &["clipboard", "translation"])?;
    let max_pixels = max_pixels.unwrap_or(1024);
    if !(128..=2048).contains(&max_pixels) {
        return Err("invalid_argument".into());
    }
    let payload = {
        let state = app.state::<AppState>();
        let _write = state.clipboard_writes.lock().await;
        crate::clipboard_runtime::payload(
            &app,
            record_id,
            crate::clipboard_runtime::PayloadMode::Stored,
        )
        .await?
    };
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let bytes = match payload {
            platform::ClipboardPayload::Image { bytes, .. }
            | platform::ClipboardPayload::FilesImage {
                bytes,
                image_fallback: true,
                ..
            } => bytes,
            platform::ClipboardPayload::Files(paths)
            | platform::ClipboardPayload::FilesImage { paths, .. }
                if paths.len() == 1 =>
            {
                nami::clipboard::read_file_preview(std::path::Path::new(&paths[0]))?
            }
            _ => return Err("invalid_argument".into()),
        };
        platform::clipboard_image_preview(&bytes, max_pixels).map_err(str::to_owned)
    })
    .await
    .map_err(|_| "storage_worker")??;
    Ok(ClipboardImage {
        bytes,
        mime: "image/png",
    })
}

#[cfg(test)]
mod tests {
    use super::{
        image_history_paste, paste_release_result, remaining_image_settle, PasteFocusReturn,
        WindowState, PASTE_FOCUS_RETURN_DELAY, PASTE_KEY_GAP,
    };
    use crate::platform::ClipboardPayload;
    use std::time::Duration;

    #[test]
    fn secondary_copy_falls_back_to_the_saved_primary_without_using_source() {
        let mut record: nami::model::HistoryRecord = serde_json::from_value(serde_json::json!({
            "id":"copy-test", "text":"source", "sourceLanguage":"auto", "targetLanguages":["en"],
            "translations":["primary"], "date":"2026-11-20T00:00:00Z", "isFavorite":false, "isPinned":false
        })).unwrap();
        assert_eq!(
            super::translation_copy_part(&record, "secondary"),
            Some("primary")
        );
        record.translations.push("secondary".into());
        assert_eq!(
            super::translation_copy_part(&record, "secondary"),
            Some("secondary")
        );
        assert_eq!(
            super::translation_copy_part(&record, "primary"),
            Some("primary")
        );
        record.translations.clear();
        assert_eq!(super::translation_copy_part(&record, "secondary"), None);
    }

    #[test]
    fn late_ocr_cannot_reserve_a_translation_after_a_newer_request() {
        let revision = std::sync::atomic::AtomicU64::new(7);
        assert_eq!(super::reserve_translation(&revision, Some(7)), Ok(8));
        assert_eq!(super::reserve_translation(&revision, None), Ok(9));
        assert_eq!(
            super::reserve_translation(&revision, Some(8)),
            Err("cancelled".into())
        );
        assert_eq!(revision.load(std::sync::atomic::Ordering::SeqCst), 9);
        assert_eq!(super::reserve_translation(&revision, Some(9)), Ok(10));
    }

    #[test]
    fn history_selection_detail_clear_is_explicit_in_the_shared_event() {
        for clear_detail in [false, true] {
            let event = super::HistorySelection {
                revision: 7,
                sample_id: None,
                retranslate: false,
                clear_detail,
                record: None,
            };
            let payload = serde_json::to_value(event).unwrap();
            assert_eq!(payload["sampleId"], serde_json::Value::Null);
            assert_eq!(payload["clearDetail"], clear_detail);
            assert_eq!(payload["revision"], 7);
            assert!(payload.get("clear_detail").is_none());
        }
    }

    #[test]
    fn image_click_waits_out_the_clipboard_notice_off_the_main_thread() {
        let image = ClipboardPayload::Image {
            bytes: vec![1],
            extension: "png".into(),
        };
        let snapshot = ClipboardPayload::FilesImage {
            paths: vec!["/tmp/nami-snapshot.png".into()],
            bytes: vec![1],
            extension: "png".into(),
            image_fallback: true,
        };
        let text = ClipboardPayload::Text("hello".into());
        let file = ClipboardPayload::Files(vec!["/tmp/nami-note.txt".into()]);
        let attached = ClipboardPayload::FilesImage {
            paths: vec!["/tmp/nami-photo.png".into()],
            bytes: vec![1],
            extension: "png".into(),
            image_fallback: false,
        };
        assert!(image_history_paste(&image));
        assert!(image_history_paste(&snapshot));
        assert!(!image_history_paste(&text));
        assert!(!image_history_paste(&file));
        assert!(!image_history_paste(&attached));
        assert_eq!(
            remaining_image_settle(true, Duration::ZERO),
            Duration::from_millis(200)
        );
        assert_eq!(
            remaining_image_settle(true, Duration::from_millis(199)),
            Duration::from_millis(1)
        );
        assert_eq!(
            remaining_image_settle(true, Duration::from_millis(200)),
            Duration::ZERO
        );
        assert_eq!(
            remaining_image_settle(true, Duration::from_millis(500)),
            Duration::ZERO
        );
        assert_eq!(
            remaining_image_settle(false, Duration::ZERO),
            Duration::ZERO
        );
        assert_eq!(PASTE_KEY_GAP, Duration::from_millis(15));
        assert_eq!(PASTE_FOCUS_RETURN_DELAY, Duration::from_millis(60));
    }

    #[test]
    fn paste_focus_return_requires_the_original_pinned_clipboard_session() {
        let window = WindowState {
            history_visible: true,
            history_pinned: true,
            records_session: 3,
            ..WindowState::default()
        };
        let handoff = PasteFocusReturn {
            session: 3,
            operation: 7,
            activation: 9,
        };
        assert!(handoff.still_owned(&window, 7, 9));
        assert!(!handoff.still_owned(&window, 8, 9));
        assert!(!handoff.still_owned(&window, 7, 10));
        for changed in [
            WindowState {
                history_visible: false,
                ..window.clone()
            },
            WindowState {
                history_pinned: false,
                ..window.clone()
            },
            WindowState {
                records_editor_open: true,
                ..window.clone()
            },
            WindowState {
                record_view: nami::model::RecordView::Translation,
                ..window.clone()
            },
            WindowState {
                records_session: 4,
                ..window.clone()
            },
        ] {
            assert!(!handoff.still_owned(&changed, 7, 9));
        }
    }

    #[test]
    fn paste_release_keeps_a_changed_clipboard_ahead_of_a_lost_focus() {
        assert_eq!(paste_release_result(true, true), Ok(()));
        assert_eq!(paste_release_result(true, false), Err("clipboard_changed"));
        assert_eq!(paste_release_result(false, false), Err("clipboard_changed"));
        assert_eq!(paste_release_result(false, true), Err("target_unavailable"));
    }
}
