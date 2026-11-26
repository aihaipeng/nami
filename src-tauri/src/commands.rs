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
    (window.clipboard_visible && window.clipboard_pinned && !window.records_editor_open)
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
    window_position_mode: nami::model::WindowPositionMode,
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
    authorize(&window, &["translation", "clipboard", "settings"])?;
    let state = app.state::<AppState>();
    let clipboard_shortcut = state.clipboard_shortcut.lock().unwrap().clone();
    let snapshot = Snapshot {
        capabilities: platform::capabilities(),
        locale: state.locale.lock().unwrap().clone(),
        window_position_mode: *state.window_position_mode.lock().unwrap(),
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
    authorize(&window, &["translation", "clipboard", "settings"])?;
    if target == "settings" {
        return crate::records_runtime::open_settings_window(&app);
    }
    windows::show(&app, &target)
}

#[tauri::command]
pub fn window_hide(window: Webview, app: AppHandle, region: String) -> Result<(), String> {
    if region == "settings" {
        authorize(&window, &["settings"])?;
    } else if ["translation", "clipboard"].contains(&region.as_str()) {
        authorize(&window, &["translation", "clipboard"])?;
    } else {
        return Err("invalid_argument".into());
    }
    if region != "settings" {
        let state = app.state::<AppState>();
        state.image_text_operation.fetch_add(1, Ordering::SeqCst);
        state.operation.fetch_add(1, Ordering::SeqCst);
    }
    windows::hide(&app, &region)
}

#[tauri::command]
pub async fn translation_copy(
    window: Webview,
    app: AppHandle,
    scope: nami::model::TranslationScope,
    part: String,
    source_text: Option<String>,
) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    let text = on_main(&app, move |app| {
        let state = app.state::<AppState>();
        let slot = state.translation_slot(scope);
        let text = if part == "source" {
            if let Some(text) = source_text.filter(|value| !value.is_empty()) {
                nami::translation::validate_text(&text)?;
                text
            } else {
                let pending = slot.pending.lock().unwrap();
                let completed = slot.completed.lock().unwrap();
                pending
                    .as_ref()
                    .map(|draft| draft.output.text.clone())
                    .or_else(|| completed.as_ref().map(|item| item.text.clone()))
                    .ok_or("invalid_argument")?
            }
        } else {
            let pending = slot.pending.lock().unwrap();
            if let Some(pending) = pending.as_ref() {
                if pending.revision != slot.revision() {
                    return Err("invalid_argument".into());
                }
                pending
                    .copy_part(&part)
                    .ok_or("invalid_argument")?
                    .to_owned()
            } else {
                let completed = slot.completed.lock().unwrap();
                let output = completed.as_ref().ok_or("invalid_argument")?;
                translation_copy_part(output, &part)
                    .ok_or("invalid_argument")?
                    .to_owned()
            }
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
    record: &'a nami::model::TranslationOutput,
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
        "translation" => {
            authorize(&window, &["translation"])?;
            state.windows.lock().unwrap().translation_pinned = pinned;
            windows::update_translation_level(&app)
        }
        "clipboard" => {
            authorize(&window, &["clipboard", "translation"])?;
            state.windows.lock().unwrap().clipboard_pinned = pinned;
            windows::update_translation_level(&app)
        }
        _ => Err("invalid_argument".into()),
    }
}

fn restore_record_focus(app: &AppHandle, record_id: Option<String>) -> Result<(), String> {
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    windows::focus_records(app)?;
    view.emit("clipboard_focus_requested", record_id)
        .map_err(|_| "event_emit".into())
}

#[tauri::command]
pub fn records_details_toggle(
    window: Webview,
    app: AppHandle,
    record_id: Option<String>,
    return_focus: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    if record_id
        .as_ref()
        .is_some_and(|id| id.is_empty() || id.len() > 160)
    {
        return Err("invalid_argument".into());
    }
    windows::toggle_record_details(&app, record_id)?;
    if return_focus == Some(true) {
        let target = {
            let regions = app.state::<AppState>();
            let regions = regions.windows.lock().unwrap();
            (regions.clipboard_visible && !regions.clipboard_detail_visible)
                .then(|| regions.clipboard_detail_id.clone())
        };
        if let Some(target) = target {
            return restore_record_focus(&app, target);
        }
    }
    Ok(())
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
        if !regions.clipboard_visible
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
        regions.select_clipboard_detail(record_id);
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
    {
        let regions = state.windows.lock().unwrap();
        if !regions.clipboard_visible
            || regions.records_editor_open
            || state.records_dirty.load(Ordering::SeqCst)
        {
            return Err("cancelled".into());
        }
    }
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    view.emit("clipboard_note_requested", record_id)
        .map_err(|_| "event_emit")?;
    windows::focus_records(&app)
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
pub fn clipboard_width_set(
    window: Webview,
    app: AppHandle,
    width: f64,
    initialize: bool,
) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    windows::resize_clipboard(&app, width, initialize)
}

#[tauri::command]
pub fn clipboard_detail_width_set(
    window: Webview,
    app: AppHandle,
    width: f64,
) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    if !width.is_finite() {
        return Err("invalid_argument".into());
    }
    {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        if !regions.clipboard_visible {
            return Ok(());
        }
        regions.set_clipboard_detail_width(width);
    }
    windows::emit_state(&app)
}

#[tauri::command]
pub fn clipboard_translation_close(
    window: Webview,
    app: AppHandle,
    return_focus: Option<bool>,
) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    let state = app.state::<AppState>();
    let target = {
        let regions = state.windows.lock().unwrap();
        (return_focus == Some(true)
            && regions.clipboard_visible
            && regions.clipboard_translation_visible)
            .then(|| {
                if regions.clipboard_detail_visible {
                    regions.clipboard_detail_id.clone()
                } else {
                    regions.clipboard_translation_record_id.clone()
                }
            })
    };
    state.image_text_operation.fetch_add(1, Ordering::SeqCst);
    state.clipboard_side_translation.clear();
    state.clipboard_translation.lock().unwrap().take();
    windows::close_clipboard_translation(&app)?;
    if let Some(target) = target {
        restore_record_focus(&app, target)?;
    }
    Ok(())
}

#[tauri::command]
pub fn clipboard_translation_clear_record(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    let changed = app
        .state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .clear_clipboard_translation_record();
    if changed {
        windows::emit_state(&app)?;
    }
    Ok(())
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

#[tauri::command]
pub fn translation_columns_set(
    window: Webview,
    app: AppHandle,
    secondary_enabled: bool,
) -> Result<(), String> {
    authorize(&window, &["translation"])?;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.secondary_enabled = secondary_enabled;
        state.translation_width = state
            .translation_width
            .max(state.standalone_translation_min_width());
    }
    windows::apply_translation(&app)
}

#[tauri::command]
pub async fn locale_set(window: Webview, app: AppHandle, locale: String) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !["system", "zh-CN", "en"].contains(&locale.as_str()) {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let stored = locale.clone();
    crate::clipboard_runtime::database(&app, move |database| {
        database.preference_set("ui_locale", &stored)
    })
    .await?;
    *state.locale.lock().unwrap() = locale.clone();
    for label in ["translation", "clipboard", "settings"] {
        if let Some(window) = app.get_webview(label) {
            window
                .emit("locale_changed", &locale)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn window_position_mode_set(
    window: Webview,
    app: AppHandle,
    mode: nami::model::WindowPositionMode,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let stored = mode.as_str().to_owned();
    crate::clipboard_runtime::database(&app, move |database| {
        database.preference_set("window_position_mode", &stored)
    })
    .await?;
    *state.window_position_mode.lock().unwrap() = mode;
    for label in ["translation", "clipboard", "settings"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("window_position_mode_changed", mode)
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
            "copy" | "copyPlain" => Activation::Copy,
            "paste" | "pastePlain" | "pastePaths" => Activation::Paste,
            "activate" => options.activation,
            "activateAlternate" => match options.activation {
                Activation::Paste => Activation::Copy,
                Activation::Copy => Activation::Paste,
            },
            _ => return Err("invalid_argument".into()),
        };
        let mode = match action.as_str() {
            "pastePlain" | "copyPlain" => PayloadMode::PlainText,
            "pastePaths" => PayloadMode::FilePaths,
            "activateAlternate" => PayloadMode::Restore { plain_text: false },
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
    let duplicate_plain = matches!(mode, PayloadMode::PlainText);
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
        if !state.windows.lock().unwrap().clipboard_pinned {
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
            save_record_use(&app, used_id, false, false).await?;
            values
        }
        Err(result) => {
            if result.copy_status == "current" {
                save_record_use(&app, used_id, result.status == "copied", duplicate_plain).await?;
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
                let _ = windows::focus_records(&app)
                    .inspect_err(|error| eprintln!("Nami clipboard focus return: {error}"));
            }
        })
        .await?;
    }
    reveal_failure(&app, operation, &result).await?;
    Ok(result)
}

async fn save_record_use(
    app: &AppHandle,
    id: String,
    copied: bool,
    plain_text: bool,
) -> Result<(), String> {
    crate::clipboard_runtime::database(app, move |database| {
        if copied {
            if plain_text {
                database.clipboard_duplicate_plain_text(&id)
            } else {
                database.clipboard_duplicate(&id)
            }
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
pub fn translation_cancel(
    window: Webview,
    app: AppHandle,
    scope: Option<nami::model::TranslationScope>,
) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard"])?;
    let state = app.state::<AppState>();
    let scope = scope.unwrap_or(nami::model::TranslationScope::Standalone);
    state.translation_slot(scope).bump_revision();
    Ok(())
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
    for label in ["translation", "settings", "clipboard"] {
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
            state.standalone_translation.bump_revision();
            state.clipboard_side_translation.bump_revision();
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
        commit_preferences(&app, preferences, false).await?;
        return crate::clipboard_runtime::sync_pin_paste_keys(&app).await;
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
    .await??;
    crate::clipboard_runtime::sync_pin_paste_keys(&app).await
}

#[tauri::command]
pub async fn translation_shortcuts_restore(
    window: Webview,
    app: AppHandle,
    general: Option<bool>,
    group: Option<nami::shortcuts::ShortcutRestoreGroup>,
) -> Result<(), String> {
    use crate::shortcuts::{
        clipboard_binding, is_general_window_action, validate_clipboard_binding,
        validate_quick_paste_reservation, ShortcutAction, ShortcutRestoreGroup,
    };
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let primary = platform::capabilities().primary_modifier;
    let current_preferences = state.translation_preferences.lock().unwrap().clone();
    let current_records = state
        .clipboard_options
        .lock()
        .unwrap()
        .record_shortcuts
        .clone();
    let current_clipboard = state.clipboard_shortcut.lock().unwrap().clone();

    let mut next_preferences: Option<crate::translation::Preferences> = None;
    let mut next_records: Option<nami::shortcuts::ClipboardRecordShortcuts> = None;
    let mut next_clipboard: Option<crate::ClipboardShortcut> = None;
    let mut update_selection_enabled = false;
    let mut sync_globals = false;
    let mut stage_bindings = Vec::new();

    match group {
        Some(ShortcutRestoreGroup::Global) => {
            if !platform::capabilities().selection {
                return Err("platform_unavailable".into());
            }
            let default_clipboard = clipboard_binding();
            let mut preferences = current_preferences.clone();
            preferences.shortcuts = preferences.shortcuts.restore_actions(
                &[ShortcutAction::OpenTranslation, ShortcutAction::Selection],
                primary,
                Some(&default_clipboard),
            )?;
            validate_clipboard_binding(default_clipboard.clone(), &preferences.shortcuts)?;
            validate_quick_paste_reservation(&default_clipboard)?;
            preferences
                .disabled_shortcuts
                .retain(|action| !action.is_global_translation());
            preferences.selection_enabled = true;
            if preferences
                .shortcuts
                .conflicts_with_records(&current_records)
                || current_records.conflicts(&default_clipboard, None)
            {
                return Err("shortcut_clipboard_panel".into());
            }
            stage_bindings.push(crate::binding_shortcut(&default_clipboard)?);
            if let Some(binding) = preferences.global_binding(ShortcutAction::Selection) {
                stage_bindings.push(crate::binding_shortcut(binding)?);
            }
            next_preferences = Some(preferences);
            next_clipboard = Some(crate::ClipboardShortcut {
                binding: Some(default_clipboard),
                enabled: true,
            });
            update_selection_enabled = true;
            sync_globals = true;
        }
        Some(ShortcutRestoreGroup::Window) => {
            let records = current_records.restore_window()?;
            let mut preferences = current_preferences.clone();
            let actions = [
                ShortcutAction::FocusSearch,
                ShortcutAction::Pin,
                ShortcutAction::ShowShortcuts,
                ShortcutAction::OpenSettings,
            ];
            preferences.shortcuts = preferences.shortcuts.restore_actions(
                &actions,
                primary,
                current_clipboard.binding.as_ref(),
            )?;
            preferences
                .disabled_shortcuts
                .retain(|action| !is_general_window_action(*action));
            if preferences.shortcuts.conflicts_with_records(&records)
                || current_clipboard
                    .binding
                    .as_ref()
                    .is_some_and(|binding| records.conflicts(binding, None))
            {
                return Err("shortcut_clipboard_panel".into());
            }
            next_preferences = Some(preferences);
            next_records = Some(records);
        }
        Some(ShortcutRestoreGroup::Records) => {
            let records = current_records.restore_records()?;
            if current_preferences
                .shortcuts
                .conflicts_with_records(&records)
                || current_clipboard
                    .binding
                    .as_ref()
                    .is_some_and(|binding| records.conflicts(binding, None))
            {
                return Err("shortcut_clipboard_panel".into());
            }
            next_records = Some(records);
        }
        Some(ShortcutRestoreGroup::Translation) => {
            let mut preferences = current_preferences.clone();
            let actions = [
                ShortcutAction::Translate,
                ShortcutAction::CopyPrimary,
                ShortcutAction::CopySecondary,
            ];
            preferences.shortcuts = preferences.shortcuts.restore_actions(
                &actions,
                primary,
                current_clipboard.binding.as_ref(),
            )?;
            preferences
                .disabled_shortcuts
                .retain(|action| !actions.contains(action));
            if preferences
                .shortcuts
                .conflicts_with_records(&current_records)
            {
                return Err("shortcut_clipboard_panel".into());
            }
            next_preferences = Some(preferences);
        }
        None => {
            let general = general.unwrap_or(false);
            let records = general
                .then(|| current_records.restore_general())
                .transpose()?;
            let mut preferences = current_preferences.clone();
            preferences.shortcuts = preferences.shortcuts.restore_group(
                general,
                primary,
                current_clipboard.binding.as_ref(),
            )?;
            preferences
                .disabled_shortcuts
                .retain(|action| is_general_window_action(*action) != general);
            if !general {
                preferences.selection_enabled = true;
                update_selection_enabled = true;
                sync_globals = true;
            }
            let effective_records = records.as_ref().unwrap_or(&current_records);
            if preferences
                .shortcuts
                .conflicts_with_records(effective_records)
                || current_clipboard
                    .binding
                    .as_ref()
                    .is_some_and(|binding| effective_records.conflicts(binding, None))
            {
                return Err("shortcut_clipboard_panel".into());
            }
            if !general && platform::capabilities().selection {
                if let Some(binding) = preferences.global_binding(ShortcutAction::Selection) {
                    stage_bindings.push(crate::binding_shortcut(binding)?);
                }
            }
            next_preferences = Some(preferences);
            next_records = records;
        }
    }

    let json =
        |value: Result<String, serde_json::Error>| value.map_err(|_| "storage_record".to_owned());
    let bindings_json = next_preferences
        .as_ref()
        .map(|value| json(serde_json::to_string(&value.shortcuts)))
        .transpose()?;
    let disabled_json = next_preferences
        .as_ref()
        .map(|value| json(serde_json::to_string(&value.disabled_shortcuts)))
        .transpose()?;
    let selection_enabled = update_selection_enabled
        .then(|| {
            next_preferences
                .as_ref()
                .map(|value| value.selection_enabled)
        })
        .flatten();
    let records_json = next_records
        .as_ref()
        .map(|value| json(serde_json::to_string(value)))
        .transpose()?;
    let clipboard_json = next_clipboard
        .as_ref()
        .map(|value| json(serde_json::to_string(&value.binding)))
        .transpose()?;

    let staged = stage_global_shortcuts(&app, stage_bindings).await?;
    let saved = crate::clipboard_runtime::database(&app, move |db| {
        db.reset_shortcut_group(
            bindings_json.as_deref(),
            disabled_json.as_deref(),
            selection_enabled,
            records_json.as_deref(),
            clipboard_json.as_deref(),
        )
    })
    .await;
    let published_clipboard = next_clipboard.clone();
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
        if let Some(clipboard) = next_clipboard {
            *state.clipboard_shortcut.lock().unwrap() = clipboard;
        }
        if let Some(records) = next_records {
            state.clipboard_options.lock().unwrap().record_shortcuts = records;
            crate::clipboard_preferences::publish_controls(&app, &state);
        }
        if let Some(preferences) = next_preferences {
            *state.translation_preferences.lock().unwrap() = preferences.clone();
            if sync_globals {
                crate::sync_global_shortcuts(&app);
            }
            publish_preferences(&app, &preferences)?;
        } else if sync_globals {
            crate::sync_global_shortcuts(&app);
        }
        Ok(())
    })
    .await??;
    crate::clipboard_runtime::sync_pin_paste_keys(&app).await?;
    if let Some(clipboard) = published_clipboard {
        let _ = app.emit(
            "clipboard_binding_changed",
            serde_json::json!({
                "binding": clipboard.binding,
                "enabled": clipboard.enabled,
                "shortcutErrors": state.shortcut_errors.lock().unwrap().clone(),
            }),
        );
    }
    Ok(())
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
pub fn translation_language_settings_open(window: Webview) -> Result<(), String> {
    authorize(&window, &["translation", "clipboard", "settings"])?;
    platform::open_translation_language_settings()
}

#[tauri::command]
pub fn translation_engine_set(
    window: Webview,
    app: AppHandle,
    engine: String,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    if !crate::translation::engine_available(&engine) {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    state.translation_preferences.lock().unwrap().engine = engine.clone();
    *state.translation_engine.lock().unwrap() = engine.clone();
    let preferences = state.translation_preferences.lock().unwrap().clone();
    state.standalone_translation.bump_revision();
    state.clipboard_side_translation.bump_revision();
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
    Complete(Box<crate::model::TranslationOutput>),
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
    pub(crate) output: crate::model::TranslationOutput,
    errors: Vec<Option<String>>,
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
        self.output
            .translations
            .get(index)
            .map(String::as_str)
            .filter(|text| !text.is_empty())
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn translation_run(
    window: Webview,
    app: AppHandle,
    text: String,
    source: String,
    targets: Vec<String>,
    scope: Option<nami::model::TranslationScope>,
    reuse_target_index: Option<usize>,
    retry_id: Option<String>,
    retry_engine: Option<String>,
) -> Result<TranslationRunOutput, String> {
    authorize(&window, &["translation"])?;
    if retry_id.is_some() && reuse_target_index.is_some() {
        return Err("invalid_argument".into());
    }
    let scope = scope.unwrap_or(nami::model::TranslationScope::Standalone);
    run_translation(
        app,
        scope,
        text,
        source,
        targets,
        reuse_target_index,
        retry_id,
        retry_engine,
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

#[allow(clippy::too_many_arguments)]
pub(crate) async fn run_translation(
    app: AppHandle,
    scope: nami::model::TranslationScope,
    text: String,
    source: String,
    targets: Vec<String>,
    reuse_target_index: Option<usize>,
    retry_id: Option<String>,
    retry_engine: Option<String>,
) -> Result<TranslationRunOutput, String> {
    crate::translation::validate_text(&text)?;
    #[cfg(feature = "ui-tests")]
    let local_fixture = if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
        if app.config().identifier != "dev.local.nami.ui-tests" {
            return Err("translation_network_disabled_in_tests".into());
        }
        match std::env::var("NAMI_UI_TEST_CASE").as_deref() {
            Ok("translation-partial" | "clipboard-translate-shortcut" | "clipboard-translate") => {
                true
            }
            // Only the explicit on-device engine may run outside the synthetic fixture.
            Ok("apple-translate") if retry_engine.as_deref() == Some("apple") => false,
            _ => return Err("translation_network_disabled_in_tests".into()),
        }
    } else {
        false
    };
    let state = app.state::<AppState>();
    let slot = state.translation_slot(scope);
    let (preferences, engine, revision, retried, previous_completed) = {
        let preferences = state.translation_preferences.lock().unwrap();
        let mut pending = slot.pending.lock().unwrap();
        let completed = slot.completed.lock().unwrap().clone();
        let (engine, revision, draft) = if let Some(id) = retry_id.as_deref() {
            let draft = pending
                .as_mut()
                .filter(|draft| {
                    let record = &draft.output;
                    !draft.retrying
                        && record.id == id
                        && record.text == text
                        && record.target_languages == targets
                        && record.translation.requested_source == source
                })
                .ok_or("invalid_argument")?;
            let engine = retry_engine.unwrap_or_else(|| draft.output.translation.engine.clone());
            let missing = targets
                .iter()
                .zip(&draft.errors)
                .filter_map(|(target, error)| error.as_ref().map(|_| target.clone()))
                .collect::<Vec<_>>();
            crate::translation::validate_options(&engine, &source, &missing)?;
            let revision = reserve_translation(&slot.revision, Some(draft.revision))?;
            draft.revision = revision;
            draft.retrying = true;
            (engine, revision, Some(draft.clone()))
        } else {
            let engine = retry_engine.unwrap_or_else(|| preferences.engine.clone());
            crate::translation::validate_options(&engine, &source, &targets)?;
            let revision = reserve_translation(&slot.revision, None)?;
            pending.take();
            (engine, revision, None)
        };
        (preferences.clone(), engine, revision, draft, completed)
    };
    let proxy_url = preferences.proxy_url;
    let proxy_enabled = preferences.proxy_enabled;
    let previous = retried
        .as_ref()
        .map(|draft| &draft.output)
        .or(previous_completed.as_ref());
    let retained = if let Some(draft) = &retried {
        draft
            .output
            .translations
            .iter()
            .zip(&draft.errors)
            .map(|(text, error)| error.is_none().then(|| text.clone()))
            .collect()
    } else {
        crate::translation::reusable_results(
            &engine,
            &text,
            &source,
            &targets,
            previous,
            reuse_target_index,
        )
    };
    let missing: Vec<String> = targets
        .iter()
        .zip(&retained)
        .filter(|(_, result)| result.is_none())
        .map(|(target, _)| target.clone())
        .collect();
    let batch = if missing.is_empty() {
        ResolvedTranslationBatch {
            translations: Vec::new(),
            errors: Vec::new(),
            engines: Vec::new(),
            source_language: source.clone(),
            detections: Vec::new(),
        }
    } else {
        let text_ref = &text;
        let source_ref = &source;
        let proxy_url_ref = &proxy_url;
        translate_with_google_fallback(&engine, source_ref, &missing, |run_engine, run_targets| {
            let run_engine = run_engine.to_string();
            let run_targets = run_targets.to_vec();
            let text_owned = text_ref.clone();
            let source_owned = source_ref.clone();
            let proxy_url_owned = proxy_url_ref.clone();
            async move {
                let request = async {
                    #[cfg(feature = "ui-tests")]
                    if local_fixture {
                        return partial_translation_fixture(&run_engine, &text_owned, &run_targets)
                            .await;
                    }
                    crate::translation::translate(
                        &run_engine,
                        &text_owned,
                        &source_owned,
                        &run_targets,
                        &proxy_url_owned,
                        proxy_enabled,
                    )
                    .await
                };
                tokio::pin!(request);
                loop {
                    tokio::select! {
                        result = &mut request => break result,
                        _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                            if slot.revision() != revision { return Err("cancelled".into()); }
                        }
                    }
                }
            }
        })
        .await?
    };
    let previous_detection = retained.first().is_some_and(Option::is_some)
        || (retained.iter().any(Option::is_some) && batch.errors.iter().all(Option::is_some));
    let detections = if previous_detection {
        previous
            .map(|record| record.translation.detections.clone())
            .unwrap_or_default()
    } else {
        batch.detections
    };
    let resolved_source = if previous_detection {
        previous
            .map(|record| record.source_language.clone())
            .unwrap_or_else(|| source.clone())
    } else {
        batch.source_language
    };
    let mut fresh_engines = batch.engines.into_iter();
    let mut target_engines = retained
        .iter()
        .enumerate()
        .map(|(index, result)| {
            if let Some(metadata) = previous
                .map(|record| &record.translation)
                .filter(|_| result.is_some())
            {
                metadata
                    .target_engines
                    .get(index)
                    .unwrap_or(&metadata.engine)
                    .clone()
            } else {
                fresh_engines.next().unwrap_or_else(|| engine.clone())
            }
        })
        .collect::<Vec<_>>();
    let effective_engine = if let Some(first) = target_engines.first() {
        if target_engines
            .iter()
            .all(|target_engine| target_engine == first)
        {
            let uniform = first.clone();
            target_engines.clear();
            uniform
        } else {
            engine.clone()
        }
    } else {
        engine.clone()
    };
    let mut fresh = batch.translations.into_iter().zip(batch.errors);
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
    let output = crate::model::TranslationOutput {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        source_language: resolved_source,
        translation: crate::model::TranslationMetadata {
            requested_source: source,
            engine: effective_engine,
            target_engines,
            strategy_version: crate::translation::STRATEGY_VERSION,
            detections,
        },
        target_languages: targets,
        translations: translated,
    };
    if let Some(error) = errors.iter().flatten().next() {
        if errors.iter().all(Option::is_some) {
            return Err(error.clone());
        }
        let mut pending = slot.pending.lock().unwrap();
        if slot.revision() != revision {
            return Err("cancelled".into());
        }
        let partial = PartialTranslationView {
            id: output.id.clone(),
            translations: output.translations.clone(),
            source_language: output.source_language.clone(),
            errors: errors.clone(),
        };
        *pending = Some(PendingTranslation {
            output,
            errors,
            revision,
            retrying: false,
        });
        return Ok(TranslationRunOutput::Partial { partial });
    }
    {
        let mut pending = slot.pending.lock().unwrap();
        if slot.revision() != revision {
            return Err("cancelled".into());
        }
        pending.take();
        *slot.completed.lock().unwrap() = Some(output.clone());
    }
    Ok(TranslationRunOutput::Complete(Box::new(output)))
}

struct ResolvedTranslationBatch {
    translations: Vec<String>,
    errors: Vec<Option<String>>,
    engines: Vec<String>,
    source_language: String,
    detections: Vec<crate::model::LanguageDetection>,
}

async fn translate_with_google_fallback<F, Fut>(
    primary_engine: &str,
    source: &str,
    missing: &[String],
    mut execute: F,
) -> Result<ResolvedTranslationBatch, String>
where
    F: FnMut(&str, &[String]) -> Fut,
    Fut: std::future::Future<Output = Result<crate::translation::TranslationResult, String>>,
{
    let primary = execute(primary_engine, missing).await;
    let (mut translations, mut errors, mut source_language, mut detections) = match primary {
        Ok(result) => (
            result.translations,
            result.errors,
            result.source_language,
            result.detections,
        ),
        Err(error) if error == "cancelled" => return Err(error),
        Err(error) => (
            vec![String::new(); missing.len()],
            vec![Some(error); missing.len()],
            source.to_string(),
            Vec::new(),
        ),
    };
    let mut engines = vec![primary_engine.to_string(); missing.len()];
    if primary_engine == "google" {
        let failed_indices: Vec<usize> = errors
            .iter()
            .enumerate()
            .filter_map(|(index, error)| error.as_ref().map(|_| index))
            .filter(|&index| {
                crate::translation::validate_options(
                    "edge",
                    source,
                    std::slice::from_ref(&missing[index]),
                )
                .is_ok()
            })
            .collect();
        if !failed_indices.is_empty() {
            let failed_targets: Vec<String> = failed_indices
                .iter()
                .map(|&index| missing[index].clone())
                .collect();
            match execute("edge", &failed_targets).await {
                Ok(fallback) => {
                    for (slot_index, (text, error)) in failed_indices
                        .into_iter()
                        .zip(fallback.translations.into_iter().zip(fallback.errors))
                    {
                        if error.is_none() {
                            translations[slot_index] = text;
                            errors[slot_index] = None;
                            engines[slot_index] = "edge".to_string();
                        }
                    }
                    if detections.is_empty() && !fallback.detections.is_empty() {
                        detections = fallback.detections;
                    }
                    if (source_language == source || source_language == "auto")
                        && !fallback.source_language.is_empty()
                        && fallback.source_language != "auto"
                    {
                        source_language = fallback.source_language;
                    }
                }
                Err(error) if error == "cancelled" => return Err(error),
                Err(_) => {}
            }
        }
    }
    Ok(ResolvedTranslationBatch {
        translations,
        errors,
        engines,
        source_language,
        detections,
    })
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
        let mut page = database.clipboard_page_with_search(
            &query,
            favorites_only,
            kind.as_deref(),
            cursor.as_ref(),
            crate::database::PAGE_SIZE,
            search,
        )?;
        crate::media::rewrite_page_source_icons(&mut page);
        let alternate = if cursor.is_none()
            && (query.trim().is_empty()
                || search.mode == nami::clipboard::ClipboardSearchMode::Exact)
        {
            let mut alternate = database.clipboard_page_with_search(
                &query,
                !favorites_only,
                kind.as_deref(),
                None,
                crate::database::PAGE_SIZE,
                search,
            )?;
            crate::media::rewrite_page_source_icons(&mut alternate);
            Some(alternate)
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
    let (detail, measure_files, image_file_paths) =
        crate::clipboard_runtime::database(&app, move |database| {
            let mut detail = database.clipboard_detail(&record_id)?;
            if detail.kind == "image" && platform::capabilities().image_ocr {
                let state = status_app.state::<AppState>();
                let jobs = state.image_index_jobs.lock().unwrap();
                detail.image_index_status =
                    Some(match database.clipboard_ocr_text(&record_id)? {
                        Some(text) if text.trim().is_empty() => "empty",
                        Some(_) => "indexed",
                        None => jobs.status(&record_id),
                    });
            }
            let representation = database.clipboard_file_representation(&record_id)?;
            let measure_files = representation.is_none();
            let image_file_paths = (detail.kind == "image")
                .then(|| representation.map(|value| value.paths))
                .flatten();
            Ok((detail, measure_files, image_file_paths))
        })
        .await?;
    let root = crate::clipboard_runtime::image_root(&app)?;
    tauri::async_runtime::spawn_blocking(move || {
        crate::clipboard_detail::enrich(detail, &root, measure_files, image_file_paths)
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
pub async fn clipboard_record_drag(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    if !platform::capabilities().clipboard_drag {
        return Err("platform_unavailable".into());
    }
    let item = crate::clipboard_runtime::prepare_drag_item(&app, record_id).await?;
    let drag_view = window.clone();
    let (sender, receiver) = tokio::sync::oneshot::channel();
    let dispatched = on_main(&app, move |app| {
        let state = app.state::<AppState>();
        if state.clipboard_dragging.swap(true, Ordering::SeqCst) {
            return Err("cancelled".into());
        }
        let end_app = app.clone();
        let fail_app = app.clone();
        let result = platform::start_clipboard_drag(
            &drag_view,
            item,
            move || {
                let state = end_app.state::<AppState>();
                state.clipboard_dragging.store(false, Ordering::SeqCst);
                if let Some(win) = end_app.get_window("translation") {
                    if win.is_focused().ok() == Some(false) {
                        state.windows.lock().unwrap().blur();
                        let _ = windows::apply_translation(&end_app);
                    }
                }
            },
            move |outcome| {
                if outcome.is_err() {
                    fail_app
                        .state::<AppState>()
                        .clipboard_dragging
                        .store(false, Ordering::SeqCst);
                }
                let _ = sender.send(outcome);
            },
        );
        if result.is_err() {
            state.clipboard_dragging.store(false, Ordering::SeqCst);
        }
        result
    })
    .await?;
    dispatched?;
    receiver.await.map_err(|_| "clipboard_drag_failed")?
}

#[tauri::command]
pub async fn clipboard_image_qr_codes(
    window: Webview,
    app: AppHandle,
    record_id: String,
) -> Result<Vec<String>, String> {
    authorize(&window, &["clipboard", "translation"])?;
    if !platform::capabilities().image_ocr {
        return Ok(Vec::new());
    }
    crate::clipboard_text::detail_qr_codes(&app, record_id).await
}

#[tauri::command]
pub async fn clipboard_text_copy(
    window: Webview,
    app: AppHandle,
    text: String,
) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    if text.is_empty() || text.len() > nami::clipboard::MAX_TEXT_BYTES {
        return Err("invalid_argument".into());
    }
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    on_main(&app, move |app| {
        let version = platform::write_clipboard(&platform::ClipboardPayload::Text(text))
            .map_err(|(code, _)| code.to_owned())?;
        app.state::<AppState>()
            .clipboard_version
            .store(version, Ordering::SeqCst);
        Ok(())
    })
    .await?
}

#[tauri::command]
pub async fn clipboard_open_url(
    window: Webview,
    app: AppHandle,
    url: String,
) -> Result<(), String> {
    use nami::clipboard::open_target::{self, OpenTarget};
    authorize(&window, &["clipboard", "translation"])?;
    if !platform::capabilities().clipboard_open_target {
        return Err("platform_unavailable".into());
    }
    let Some(target @ OpenTarget::Link(_)) = open_target::resolve("text", &url, None)? else {
        return Err("invalid_argument".into());
    };
    on_main(&app, move |_| platform::open_clipboard_target(target)).await?
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
    let (record, image_paths) = crate::clipboard_runtime::database(&app, move |database| {
        let record = database.clipboard_record(&record_id)?;
        let image_paths = if record.kind == "image" {
            database
                .clipboard_file_representation(&record_id)?
                .map(|value| value.paths)
        } else {
            None
        };
        Ok((record, image_paths))
    })
    .await?;
    let target =
        tauri::async_runtime::spawn_blocking(move || -> Result<Option<OpenTarget>, String> {
            let target = match (&record.kind[..], image_paths.as_deref()) {
                ("image", Some(paths)) => open_target::resolve_paths(paths, file_index)?,
                _ => open_target::resolve(&record.kind, &record.text, file_index)?,
            };
            if let Some(OpenTarget::File(path)) = &target {
                if (execute || record.kind != "files") && !path.exists() {
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
) -> Result<Option<crate::ClipboardTranslationContext>, String> {
    authorize(&window, &["translation"])?;
    let state = app.state::<AppState>();
    let context = state.clipboard_translation.lock().unwrap().take();
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
    let deleted_id = deleting.then(|| record_id.clone());
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
    if let Some(deleted_id) = deleted_id {
        let cleared = {
            let mut regions = state.windows.lock().unwrap();
            if regions.clipboard_translation_record_id.as_deref() == Some(deleted_id.as_str()) {
                regions.clipboard_translation_record_id = None;
                true
            } else {
                false
            }
        };
        if cleared {
            let _ = on_main(&app, |app| windows::emit_state(&app)).await;
        }
    }
    crate::clipboard_runtime::changed(&app)?;
    if deleting {
        crate::media::remove_file_thumbs(&app);
        if crate::clipboard_preferences::retry_image_deletions_locked(&app, root).await? {
            return Err("clipboard_cleanup_failed".into());
        }
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
    crate::clipboard_runtime::sync_pin_paste_keys(&app).await?;
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
) -> Result<tauri::ipc::Response, String> {
    authorize(&window, &["clipboard", "translation"])?;
    let max_pixels = max_pixels.unwrap_or(1024);
    let bytes = crate::media::load_image_preview(&app, record_id, max_pixels, None).await?;
    Ok(tauri::ipc::Response::new(bytes))
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
        let mut record: nami::model::TranslationOutput = serde_json::from_value(serde_json::json!({
            "id":"copy-test", "text":"source", "sourceLanguage":"auto", "targetLanguages":["en"],
            "translations":["primary"],
            "translation":{"requestedSource":"auto","engine":"edge","strategyVersion":1,"detections":[]}
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
            clipboard_visible: true,
            clipboard_pinned: true,
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
                clipboard_visible: false,
                ..window.clone()
            },
            WindowState {
                clipboard_pinned: false,
                ..window.clone()
            },
            WindowState {
                records_editor_open: true,
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

    #[test]
    fn apple_failure_never_sends_text_to_a_cloud_fallback() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut called = Vec::new();
        let batch = runtime
            .block_on(super::translate_with_google_fallback(
                "apple",
                "en",
                &["en".into(), "ja".into()],
                |engine, _| {
                    called.push(engine.to_owned());
                    async {
                        Ok(crate::translation::TranslationResult {
                            translations: vec!["Synthetic result".into(), String::new()],
                            errors: vec![
                                None,
                                Some("Apple Translate language pack is not downloaded.".into()),
                            ],
                            source_language: "en".into(),
                            detections: Vec::new(),
                        })
                    }
                },
            ))
            .unwrap();
        assert_eq!(called, ["apple"]);
        assert_eq!(batch.translations[0], "Synthetic result");
        assert!(batch.errors[1].is_some());
        assert_eq!(batch.engines, ["apple", "apple"]);
    }

    #[test]
    fn google_total_failure_falls_back_to_edge_automatically() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut called = Vec::new();
        let batch = runtime
            .block_on(super::translate_with_google_fallback(
                "google",
                "auto",
                &["en".into(), "zh-CN".into()],
                |engine, targets| {
                    called.push((engine.to_string(), targets.to_vec()));
                    let engine = engine.to_string();
                    let targets = targets.to_vec();
                    async move {
                        if engine == "google" {
                            Err("Google Translate is unreachable".into())
                        } else {
                            Ok(crate::translation::TranslationResult {
                                translations: targets
                                    .iter()
                                    .map(|target| format!("edge:{target}"))
                                    .collect(),
                                errors: vec![None; targets.len()],
                                source_language: "en".into(),
                                detections: Vec::new(),
                            })
                        }
                    }
                },
            ))
            .unwrap();
        assert_eq!(
            called,
            vec![
                ("google".into(), vec!["en".into(), "zh-CN".into()]),
                ("edge".into(), vec!["en".into(), "zh-CN".into()]),
            ]
        );
        assert_eq!(batch.translations, vec!["edge:en", "edge:zh-CN"]);
        assert_eq!(batch.errors, vec![None, None]);
        assert_eq!(batch.engines, vec!["edge", "edge"]);
        assert_eq!(batch.source_language, "en");
    }

    #[test]
    fn google_partial_failure_falls_back_failed_target_only_to_edge() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut called = Vec::new();
        let batch = runtime
            .block_on(super::translate_with_google_fallback(
                "google",
                "auto",
                &["en".into(), "zh-CN".into()],
                |engine, targets| {
                    called.push((engine.to_string(), targets.to_vec()));
                    let engine = engine.to_string();
                    let targets = targets.to_vec();
                    async move {
                        if engine == "google" {
                            Ok(crate::translation::TranslationResult {
                                translations: vec!["google:en".into(), String::new()],
                                errors: vec![None, Some("rate_limited".into())],
                                source_language: "ja".into(),
                                detections: Vec::new(),
                            })
                        } else {
                            Ok(crate::translation::TranslationResult {
                                translations: targets
                                    .iter()
                                    .map(|target| format!("edge:{target}"))
                                    .collect(),
                                errors: vec![None; targets.len()],
                                source_language: "ja".into(),
                                detections: Vec::new(),
                            })
                        }
                    }
                },
            ))
            .unwrap();
        assert_eq!(
            called,
            vec![
                ("google".into(), vec!["en".into(), "zh-CN".into()]),
                ("edge".into(), vec!["zh-CN".into()]),
            ]
        );
        assert_eq!(batch.translations, vec!["google:en", "edge:zh-CN"]);
        assert_eq!(batch.errors, vec![None, None]);
        assert_eq!(batch.engines, vec!["google", "edge"]);
        assert_eq!(batch.source_language, "ja");
    }

    #[test]
    fn google_fallback_skips_edge_when_target_is_unsupported_by_edge() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut called = Vec::new();
        let batch = runtime
            .block_on(super::translate_with_google_fallback(
                "google",
                "auto",
                &["ab".into()],
                |engine, targets| {
                    called.push((engine.to_string(), targets.to_vec()));
                    async move { Err("google_down".into()) }
                },
            ))
            .unwrap();
        assert_eq!(called, vec![("google".into(), vec!["ab".into()])]);
        assert_eq!(batch.errors, vec![Some("google_down".into())]);
        assert_eq!(batch.engines, vec!["google"]);
    }

    #[test]
    fn google_fallback_skips_unsupported_target_and_falls_back_supported_target_to_edge() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut called = Vec::new();
        let batch = runtime
            .block_on(super::translate_with_google_fallback(
                "google",
                "auto",
                &["zh-CN".into(), "ab".into()],
                |engine, targets| {
                    called.push((engine.to_string(), targets.to_vec()));
                    let engine = engine.to_string();
                    let targets = targets.to_vec();
                    async move {
                        if engine == "google" {
                            Err("google_down".into())
                        } else {
                            Ok(crate::translation::TranslationResult {
                                translations: targets
                                    .iter()
                                    .map(|target| format!("edge:{target}"))
                                    .collect(),
                                errors: vec![None; targets.len()],
                                source_language: "en".into(),
                                detections: Vec::new(),
                            })
                        }
                    }
                },
            ))
            .unwrap();
        assert_eq!(
            called,
            vec![
                ("google".into(), vec!["zh-CN".into(), "ab".into()]),
                ("edge".into(), vec!["zh-CN".into()]),
            ]
        );
        assert_eq!(batch.translations, vec!["edge:zh-CN", ""]);
        assert_eq!(batch.errors, vec![None, Some("google_down".into())]);
        assert_eq!(batch.engines, vec!["edge", "google"]);
    }

    #[test]
    fn edge_failure_does_not_auto_fallback_to_google() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut called = Vec::new();
        let batch = runtime
            .block_on(super::translate_with_google_fallback(
                "edge",
                "auto",
                &["en".into(), "zh-CN".into()],
                |engine, targets| {
                    called.push((engine.to_string(), targets.to_vec()));
                    async move { Err("edge_down".into()) }
                },
            ))
            .unwrap();
        assert_eq!(
            called,
            vec![("edge".into(), vec!["en".into(), "zh-CN".into()])]
        );
        assert_eq!(
            batch.errors,
            vec![Some("edge_down".into()), Some("edge_down".into())]
        );
        assert_eq!(batch.engines, vec!["edge", "edge"]);
    }
}
