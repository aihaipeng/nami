use crate::{
    model::{OperationResult, WindowState},
    platform, windows, AppState,
};
use serde::Serialize;
use std::{sync::atomic::Ordering, time::Duration};
use tauri::{AppHandle, Emitter, Manager, Webview};

const IMAGE_PASTE_SETTLE: Duration = Duration::from_millis(200);
const PASTE_KEY_GAP: Duration = Duration::from_millis(15);

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
    Posted,
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
    accessibility: bool,
    shortcut_errors: Vec<String>,
    translation_engine: String,
    translation_preferences: crate::translation::Preferences,
    storage_error: Option<String>,
    clipboard_capture_enabled: bool,
    clipboard_recording_types: nami::clipboard::RecordingTypes,
    clipboard_binding: Option<crate::shortcuts::KeyBinding>,
    window: WindowState,
}

#[tauri::command]
pub fn app_snapshot(window: Webview, app: AppHandle) -> Result<Snapshot, String> {
    authorize(
        &window,
        &["translation", "history", "clipboard", "settings"],
    )?;
    let state = app.state::<AppState>();
    let snapshot = Snapshot {
        capabilities: platform::capabilities(),
        locale: state.locale.lock().unwrap().clone(),
        accessibility: platform::accessibility(),
        shortcut_errors: state.shortcut_errors.lock().unwrap().clone(),
        translation_engine: state.translation_engine.lock().unwrap().clone(),
        translation_preferences: state.translation_preferences.lock().unwrap().clone(),
        storage_error: state.storage_error.lock().unwrap().clone(),
        clipboard_capture_enabled: state.clipboard_enabled.load(Ordering::SeqCst),
        clipboard_recording_types: *state.clipboard_recording_types.lock().unwrap(),
        clipboard_binding: state.clipboard_binding.lock().unwrap().clone(),
        window: state.windows.lock().unwrap().clone(),
    };
    Ok(snapshot)
}

#[tauri::command]
pub fn window_show(window: Webview, app: AppHandle, target: String) -> Result<(), String> {
    authorize(
        &window,
        &["translation", "history", "clipboard", "settings"],
    )?;
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
pub fn translation_sample_copy(
    window: Webview,
    app: AppHandle,
    sample_id: String,
    part: String,
) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    let record = windows::history_record(&app, &sample_id).ok_or("invalid_argument")?;
    let text = match part.as_str() {
        "source" => Some(record.text),
        "primary" => record.translations.first().cloned(),
        "secondary" => record.translations.get(1).cloned(),
        _ => None,
    }
    .ok_or("invalid_argument")?;
    platform::write_text(&text)
        .map(|version| {
            app.state::<AppState>()
                .clipboard_version
                .store(version, Ordering::SeqCst);
        })
        .map_err(|(code, _)| code.into())
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
pub fn records_details_toggle(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    windows::toggle_record_details(&app)
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
    authorize(&window, &["translation", "history"])?;
    {
        let state = app.state::<AppState>();
        let mut windows = state.windows.lock().unwrap();
        windows.toggle_history();
        if windows.history_visible {
            windows.records_focus += 1;
        }
    }
    windows::apply_translation(&app)?;
    if platform::capabilities().native_history
        && app
            .state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .history_visible
    {
        if let Some(view) = app.get_webview("clipboard") {
            view.set_focus().map_err(|_| "window_focus")?;
        }
    }
    Ok(())
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
    translate_label: String,
    actions_label: String,
    can_translate: bool,
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
        || [
            &state.source_label,
            &state.translate_label,
            &state.actions_label,
        ]
        .iter()
        .any(|label| {
            label.is_empty() || label.chars().count() > 100 || label.contains(['\n', '\r'])
        })
    {
        return Err("invalid_argument".into());
    }
    if !platform::capabilities().native_chrome {
        return Ok(false);
    }
    windows::update_translation_chrome(
        &app,
        state.source_label,
        state.source_width,
        state.translate_label,
        state.actions_label,
        state.can_translate,
    )?;
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
) -> Result<crate::history::HistorySnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    let state = app.state::<AppState>();
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let snapshot = state.history.lock().unwrap().snapshot();
    Ok(snapshot)
}

#[tauri::command]
pub async fn history_query(
    window: Webview,
    app: AppHandle,
    query: String,
    favorites_only: bool,
) -> Result<crate::history::HistorySnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    crate::history_runtime::query_history(&app, query, favorites_only).await
}

#[tauri::command]
pub async fn history_load_more(
    window: Webview,
    app: AppHandle,
) -> Result<crate::history::HistorySnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    crate::history_runtime::load_more(&app).await
}

#[tauri::command]
pub async fn history_record_update(
    window: Webview,
    app: AppHandle,
    record_id: String,
    action: String,
) -> Result<crate::history::HistorySnapshot, String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    update_history_record(&app, &record_id, &action).await
}

pub(crate) async fn update_history_record(
    app: &AppHandle,
    id: &str,
    action: &str,
) -> Result<crate::history::HistorySnapshot, String> {
    crate::history_runtime::update_record(app, id.to_owned(), action.to_owned()).await
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
                if retranslate {
                    view.set_focus().map_err(|_| "window_focus")?;
                }
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
    let action = match action.as_str() {
        "copy" => crate::clipboard_preferences::Activation::Copy,
        "paste" => crate::clipboard_preferences::Activation::Paste,
        "activate" => {
            app.state::<AppState>()
                .clipboard_options
                .lock()
                .unwrap()
                .activation
        }
        _ => return Err("invalid_argument".into()),
    };
    let operation = on_main(&app, |app| {
        app.state::<AppState>()
            .operation
            .fetch_add(1, Ordering::SeqCst)
            + 1
    })
    .await?;
    let promote_id = record_id.clone();
    let payload = crate::clipboard_runtime::payload(&app, record_id, true).await?;
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
        if matches!(action, crate::clipboard_preferences::Activation::Copy) {
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
        Ok((target, version, written_at))
    })
    .await?;
    let (target, version, written_at) = match initial {
        Ok(values) => {
            // Clicking a row writes the clipboard before it pastes. That write is the same copy.
            promote_used_record(&app, promote_id.clone()).await?;
            values
        }
        Err(result) => {
            if result.copy_status == "current" {
                promote_used_record(&app, promote_id).await?;
            }
            reveal_failure(&app, operation, &result).await?;
            return Ok(result);
        }
    };
    let mut keys_down = false;
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
                Ok(()) => Ok(PasteAttempt::Posted),
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
            Some(Ok(PasteAttempt::Posted)) => {
                keys_down = true;
                break;
            }
        }
    }
    if !keys_down {
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
    }
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
    reveal_failure(&app, operation, &result).await?;
    Ok(result)
}

async fn promote_used_record(app: &AppHandle, id: String) -> Result<(), String> {
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    crate::clipboard_runtime::database(app, move |database| database.clipboard_promote(&id))
        .await?;
    crate::clipboard_runtime::changed(app)?;
    drop(_write);
    Ok(())
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
    app.state::<AppState>()
        .translation_revision
        .fetch_add(1, Ordering::SeqCst);
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
    for label in ["translation", "settings"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("translation_preferences_changed", preferences)
                .map_err(|_| "event_emit")?;
            view.emit("translation_engine_changed", &preferences.engine)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
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
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let mut preferences = state.translation_preferences.lock().unwrap().clone();
    let clipboard = state.clipboard_binding.lock().unwrap().clone();
    preferences.set_shortcut(&action, binding, clipboard.as_ref())?;
    commit_preferences(&app, preferences, false).await
}

#[tauri::command]
pub async fn translation_shortcuts_restore(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let mut preferences = state.translation_preferences.lock().unwrap().clone();
    preferences.shortcuts =
        crate::shortcuts::TranslationShortcuts::defaults(platform::capabilities().primary_modifier);
    if let Some(binding) = state.clipboard_binding.lock().unwrap().clone() {
        if crate::shortcuts::validate_clipboard_binding(binding, &preferences.shortcuts).is_err() {
            return Err("shortcut_clipboard".into());
        }
    }
    commit_preferences(&app, preferences, false).await
}

#[tauri::command]
pub async fn translation_shortcut_capture(
    window: Webview,
    app: AppHandle,
    active: bool,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    on_main(&app, move |app| {
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

#[tauri::command]
pub async fn translation_run(
    window: Webview,
    app: AppHandle,
    text: String,
    source: String,
    targets: Vec<String>,
    from_selection: bool,
    reuse: Option<crate::translation::TranslationReuse>,
) -> Result<crate::model::HistoryRecord, String> {
    authorize(&window, &["translation"])?;
    let captured = if from_selection {
        app.state::<AppState>()
            .selection_source
            .lock()
            .unwrap()
            .clone()
    } else {
        None
    };
    run_translation(app, text, source, targets, captured, reuse, None).await
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
    reuse: Option<crate::translation::TranslationReuse>,
    expected_revision: Option<u64>,
) -> Result<crate::model::HistoryRecord, String> {
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
    if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
        return Err("translation_network_disabled_in_tests".into());
    }
    let state = app.state::<AppState>();
    let revision = reserve_translation(&state.translation_revision, expected_revision)?;
    let preferences = state.translation_preferences.lock().unwrap().clone();
    let engine = preferences.engine.clone();
    let proxy_url = preferences.proxy_url.clone();
    let proxy_enabled = preferences.proxy_enabled;
    crate::translation::validate_options(&engine, &source, &targets)?;
    let previous = reuse
        .as_ref()
        .and_then(|reuse| state.history.lock().unwrap().record(&reuse.record_id));
    let retained = crate::translation::reusable_results(
        &engine,
        &text,
        &source,
        &targets,
        previous.as_ref(),
        reuse.as_ref().and_then(|reuse| reuse.target_index),
    );
    let missing: Vec<String> = targets
        .iter()
        .zip(&retained)
        .filter(|(_, result)| result.is_none())
        .map(|(target, _)| target.clone())
        .collect();
    let source_app =
        crate::translation::source_app_for_translation(&text, previous.as_ref(), captured);
    let translated_result = if missing.is_empty() {
        crate::translation::TranslationResult {
            translations: Vec::new(),
            source_language: source.clone(),
            detections: Vec::new(),
        }
    } else {
        let request = crate::translation::translate(
            &engine,
            &text,
            &source,
            &missing,
            &proxy_url,
            proxy_enabled,
        );
        tokio::pin!(request);
        loop {
            tokio::select! {
                result = &mut request => break result?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                    if state.translation_revision.load(Ordering::SeqCst) != revision { return Err("cancelled".into()); }
                }
            }
        }
    };
    let detections = if retained.first().is_some_and(Option::is_some) {
        previous
            .as_ref()
            .and_then(|record| record.translation.as_ref())
            .map(|metadata| metadata.detections.clone())
            .unwrap_or_default()
    } else {
        translated_result.detections
    };
    let resolved_source = if retained.first().is_some_and(Option::is_some) {
        previous
            .as_ref()
            .map(|record| record.source_language.clone())
            .unwrap_or_else(|| source.clone())
    } else {
        translated_result.source_language
    };
    let mut fresh = translated_result.translations.into_iter();
    let translated = retained
        .into_iter()
        .map(|cached| cached.or_else(|| fresh.next()).ok_or("translation_result"))
        .collect::<Result<Vec<_>, _>>()?;
    let record = crate::model::HistoryRecord {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        source_language: resolved_source,
        translation: Some(crate::model::TranslationMetadata {
            requested_source: source,
            engine: engine.clone(),
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
    crate::history_runtime::commit_translation(&app, record, engine, revision).await
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
    crate::clipboard_runtime::database(&app, move |database| {
        let page = database.clipboard_page_filtered(
            &query,
            favorites_only,
            kind.as_deref(),
            cursor.as_ref(),
            crate::database::PAGE_SIZE,
        )?;
        let alternate = if cursor.is_none() {
            Some(database.clipboard_page_filtered(
                &query,
                !favorites_only,
                kind.as_deref(),
                None,
                crate::database::PAGE_SIZE,
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
    let (detail, measure_files) = crate::clipboard_runtime::database(&app, move |database| {
        let detail = database.clipboard_detail(&record_id)?;
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
    let images = crate::clipboard_runtime::database(&app, move |database| {
        let mut images = Vec::new();
        if action == "delete" {
            images.extend(database.clipboard_record(&record_id)?.image_path);
            images.extend(
                database
                    .clipboard_file_representation(&record_id)?
                    .map(|representation| representation.image_path),
            );
            images.sort();
            images.dedup();
        }
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
        let mut unreferenced = Vec::new();
        for path in images {
            if !database.clipboard_image_referenced(&path)? {
                unreferenced.push(path);
            }
        }
        Ok(unreferenced)
    })
    .await?;
    crate::clipboard_runtime::changed(&app)?;
    if !images.is_empty() {
        tauri::async_runtime::spawn_blocking(move || {
            for path in images {
                nami::clipboard::remove_image(&root, &path)?;
            }
            Ok::<_, String>(())
        })
        .await
        .map_err(|_| "storage_worker")?
        .map_err(|_| "clipboard_cleanup_failed")?;
    }
    Ok(())
}

#[tauri::command]
pub async fn clipboard_capture_set(
    window: Webview,
    app: AppHandle,
    enabled: bool,
) -> Result<(), String> {
    authorize(&window, &["settings"])?;
    crate::clipboard_preferences::set_recording(&app, Some(enabled)).await
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
) -> Result<(), String> {
    use tauri_plugin_global_shortcut::GlobalShortcutExt;
    authorize(&window, &["settings"])?;
    if !platform::capabilities().selection {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    let _write = state.shortcut_writes.lock().await;
    let binding = {
        let preferences = state.translation_preferences.lock().unwrap();
        binding
            .map(|binding| {
                crate::shortcuts::validate_clipboard_binding(binding, &preferences.shortcuts)
            })
            .transpose()?
    };
    let next = binding.as_ref().map(crate::binding_shortcut).transpose()?;
    let value = serde_json::to_string(&binding).map_err(|_| "storage_record")?;
    // Reserve the new key before saving it, without releasing the current binding.
    let staged = on_main(&app, move |app| {
        let Some(next) = next else {
            return Ok(None);
        };
        if app.global_shortcut().is_registered(next) {
            return Ok::<_, String>(None);
        }
        app.global_shortcut()
            .register(next)
            .map_err(|_| "shortcut_registration")?;
        Ok(Some(next))
    })
    .await??;
    let saved = crate::clipboard_runtime::database(&app, move |database| {
        database.preference_set("clipboard_shortcut", &value)
    })
    .await;
    let result = binding.clone();
    on_main(&app, move |app| {
        let state = app.state::<AppState>();
        if let Err(error) = saved {
            if let Some(staged) = staged {
                let _ = app.global_shortcut().unregister(staged);
            }
            return Err(error);
        }
        if let Some(staged) = staged {
            state.registered_shortcuts.lock().unwrap().push(staged);
        }
        *state.clipboard_binding.lock().unwrap() = binding;
        crate::sync_global_shortcuts(&app);
        Ok(())
    })
    .await??;
    let _ = window.emit_to(
        "settings",
        "clipboard_binding_changed",
        serde_json::json!({
            "binding": result,
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
    let payload = crate::clipboard_runtime::payload(&app, record_id, false).await?;
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
    use super::{image_history_paste, paste_release_result, remaining_image_settle, PASTE_KEY_GAP};
    use crate::platform::ClipboardPayload;
    use std::time::Duration;

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
    }

    #[test]
    fn paste_release_keeps_a_changed_clipboard_ahead_of_a_lost_focus() {
        assert_eq!(paste_release_result(true, true), Ok(()));
        assert_eq!(paste_release_result(true, false), Err("clipboard_changed"));
        assert_eq!(paste_release_result(false, false), Err("clipboard_changed"));
        assert_eq!(paste_release_result(false, true), Err("target_unavailable"));
    }
}
