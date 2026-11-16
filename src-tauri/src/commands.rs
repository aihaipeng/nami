use crate::{
    model::{OperationResult, WindowState},
    platform, windows, AppState,
};
use serde::Serialize;
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager, Webview};

fn authorize(window: &Webview, labels: &[&str]) -> Result<(), String> {
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
    storage_error: Option<String>,
    clipboard_capture_enabled: bool,
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
        storage_error: state.storage_error.lock().unwrap().clone(),
        clipboard_capture_enabled: state.clipboard_enabled.load(Ordering::SeqCst),
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
        app.state::<AppState>()
            .image_text_operation
            .fetch_add(1, Ordering::SeqCst);
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
    app.state::<AppState>()
        .image_text_operation
        .fetch_add(1, Ordering::SeqCst);
    windows::set_record_view(&app, &view)
}

#[tauri::command]
pub fn records_details_toggle(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    windows::toggle_record_details(&app)
}

#[tauri::command]
pub fn records_editor_set(window: Webview, app: AppHandle, dirty: bool) -> Result<(), String> {
    authorize(&window, &["clipboard", "translation"])?;
    app.state::<AppState>()
        .records_dirty
        .store(dirty, Ordering::SeqCst);
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
        state.translation_visible = true;
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
    actions_visible: bool,
}

#[tauri::command]
pub fn translation_chrome_update(
    window: Webview,
    app: AppHandle,
    state: TranslationChromeState,
) -> Result<bool, String> {
    authorize(&window, &["translation"])?;
    if !state.source_width.is_finite()
        || !(32.0..=200.0).contains(&state.source_width)
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
        state.actions_visible,
    )?;
    Ok(true)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HistorySelection {
    revision: u64,
    sample_id: Option<String>,
    retranslate: bool,
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
) -> Result<(), String> {
    authorize(&window, &["translation", "history", "clipboard"])?;
    select_history_sample(&app, sample_id, retranslate)
}

pub(crate) fn select_history_sample(
    app: &AppHandle,
    sample_id: Option<String>,
    retranslate: bool,
) -> Result<(), String> {
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
    if !["copy", "paste"].contains(&action.as_str()) {
        return Err("invalid_argument".into());
    }
    let operation = on_main(&app, |app| {
        app.state::<AppState>()
            .operation
            .fetch_add(1, Ordering::SeqCst)
            + 1
    })
    .await?;
    let payload = crate::clipboard_runtime::payload(&app, record_id).await?;
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
        if action == "copy" {
            if !state.windows.lock().unwrap().history_pinned {
                let _ = windows::hide_after_clipboard_use(&app);
            }
            return Err(OperationResult::copied());
        }
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
        Ok((target, version))
    })
    .await?;
    let (target, version) = match initial {
        Ok(values) => values,
        Err(result) => {
            reveal_failure(&app, operation, &result).await?;
            return Ok(result);
        }
    };
    for _ in 0..30 {
        let result = on_main(&app, move |app| {
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
                return Some(OperationResult::failed(code, Some(version), current));
            }
            if !platform::target_focused(target) {
                return None;
            }
            Some(match platform::send_paste(target, version) {
                Ok(()) => OperationResult {
                    status: "dispatched",
                    copy_status: "current",
                    code: None,
                },
                Err(code) => {
                    OperationResult::failed(code, Some(version), platform::clipboard_version())
                }
            })
        })
        .await?;
        if let Some(result) = result {
            reveal_failure(&app, operation, &result).await?;
            return Ok(result);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let result = on_main(&app, move |_| {
        OperationResult::failed(
            "focus_timeout",
            Some(version),
            platform::clipboard_version(),
        )
    })
    .await?;
    reveal_failure(&app, operation, &result).await?;
    Ok(result)
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
    let context = app.state::<AppState>().selection.lock().unwrap().take();
    Ok(context)
}

#[tauri::command]
pub fn translation_cancel(window: Webview, app: AppHandle) -> Result<(), String> {
    authorize(&window, &["translation"])?;
    app.state::<AppState>()
        .translation_revision
        .fetch_add(1, Ordering::SeqCst);
    Ok(())
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
    *state.translation_engine.lock().unwrap() = engine.clone();
    state.translation_revision.fetch_add(1, Ordering::SeqCst);
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
    let revision = state.translation_revision.fetch_add(1, Ordering::SeqCst) + 1;
    let engine = state.translation_engine.lock().unwrap().clone();
    crate::translation::validate_options(&engine, &source, &targets)?;
    let previous = reuse
        .as_ref()
        .and_then(|reuse| state.history.lock().unwrap().record(&reuse.record_id));
    let retained = crate::translation::reusable_results(
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
    let captured = if from_selection {
        state.selection_source.lock().unwrap().clone()
    } else {
        None
    };
    let source_app =
        crate::translation::source_app_for_translation(&text, previous.as_ref(), captured);
    let translated = if missing.is_empty() {
        Vec::new()
    } else {
        let request = crate::translation::translate(&engine, &text, &source, &missing);
        tokio::pin!(request);
        let translated = loop {
            tokio::select! {
                result = &mut request => break result?,
                _ = tokio::time::sleep(std::time::Duration::from_millis(25)) => {
                    if state.translation_revision.load(Ordering::SeqCst) != revision { return Err("cancelled".into()); }
                }
            }
        };
        translated
    };
    let mut fresh = translated.into_iter();
    let translated = retained
        .into_iter()
        .map(|cached| cached.or_else(|| fresh.next()).ok_or("translation_result"))
        .collect::<Result<Vec<_>, _>>()?;
    let record = crate::model::HistoryRecord {
        id: uuid::Uuid::new_v4().to_string(),
        text,
        source_language: source,
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
    crate::clipboard_runtime::database(&app, move |database| database.clipboard_detail(&record_id))
        .await
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
    let image = crate::clipboard_runtime::database(&app, move |database| {
        let image = if action == "delete" {
            database.clipboard_record(&record_id)?.image_path
        } else {
            None
        };
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
        Ok(image)
    })
    .await?;
    crate::clipboard_runtime::changed(&app)?;
    if let Some(path) = image {
        tauri::async_runtime::spawn_blocking(move || nami::clipboard::remove_image(&root, &path))
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
    if !platform::capabilities().clipboard_write {
        return Err("platform_unavailable".into());
    }
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    crate::clipboard_runtime::database(&app, move |database| {
        database.clipboard_capture_set(enabled)
    })
    .await?;
    on_main(&app, move |app| {
        let state = app.state::<AppState>();
        state
            .clipboard_version
            .store(platform::clipboard_version(), Ordering::SeqCst);
        state.clipboard_enabled.store(enabled, Ordering::SeqCst);
        for label in ["clipboard", "settings"] {
            if let Some(view) = app.get_webview(label) {
                view.emit("clipboard_capture_changed", enabled)
                    .map_err(|_| "event_emit".to_string())?;
            }
        }
        Ok(())
    })
    .await?
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
    let payload = crate::clipboard_runtime::payload(&app, record_id).await?;
    let bytes = tauri::async_runtime::spawn_blocking(move || {
        let bytes = match payload {
            platform::ClipboardPayload::Image { bytes, .. } => bytes,
            platform::ClipboardPayload::Files(paths) if paths.len() == 1 => {
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
