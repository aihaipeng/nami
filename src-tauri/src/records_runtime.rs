use crate::{windows, AppState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

pub(crate) struct PendingNavigation {
    action: String,
    // None leaves the current paste target alone; Some(None) clears it on confirmation.
    paste_target: Option<Option<crate::platform::Target>>,
}

pub(crate) fn defer_navigation(app: &AppHandle, target: &str) -> Result<bool, String> {
    defer_navigation_with_target(app, target, None)
}

pub(crate) fn defer_navigation_with_target(
    app: &AppHandle,
    target: &str,
    paste_target: Option<Option<crate::platform::Target>>,
) -> Result<bool, String> {
    let state = app.state::<AppState>();
    if !state.records_dirty.load(Ordering::SeqCst) {
        return Ok(false);
    }
    *state.records_navigation.lock().unwrap() = Some(PendingNavigation {
        action: target.to_owned(),
        paste_target,
    });
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    view.emit("records_navigation_requested", ())
        .map_err(|_| "event_emit")?;
    windows::focus_records(app)?;
    Ok(true)
}

pub(crate) fn open_settings_window(app: &AppHandle) -> Result<(), String> {
    if defer_navigation(app, "settings")? {
        return Ok(());
    }
    windows::show(app, "settings")
}

pub(crate) fn dismiss_records_window(app: &AppHandle, region: &str) -> Result<(), String> {
    if defer_navigation(app, "close:records")? {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let close_side_translation = {
        let regions = state.windows.lock().unwrap();
        regions.clipboard_visible
            && regions.clipboard_translation_visible
            && !regions.records_editor_open
    };
    if close_side_translation {
        state.image_text_operation.fetch_add(1, Ordering::SeqCst);
        state.clipboard_side_translation.clear();
        state.clipboard_translation.lock().unwrap().take();
        return windows::close_clipboard_translation(app);
    }
    state.image_text_operation.fetch_add(1, Ordering::SeqCst);
    state.operation.fetch_add(1, Ordering::SeqCst);
    windows::hide(app, region)
}

pub(crate) fn resolve_navigation(app: &AppHandle, proceed: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pending = state.records_navigation.lock().unwrap().take();
    let action = pending.as_ref().map(|pending| pending.action.as_str());
    if !proceed {
        if action == Some("translation") {
            state.selection.lock().unwrap().take();
            state.selection_presentation.lock().unwrap().take();
            state.selection_target.lock().unwrap().take();
        }
        return Ok(());
    }
    state.records_dirty.store(false, Ordering::SeqCst);
    let paste_target = pending.as_ref().and_then(|pending| pending.paste_target);
    if let Some(target) = paste_target {
        *state.target.lock().unwrap() = target;
        state.operation.fetch_add(1, Ordering::SeqCst);
    }
    match action {
        Some("close:records") => dismiss_records_window(app, "translation"),
        Some(target) => {
            if target == "translation" && paste_target.is_none() {
                if state.selection.lock().unwrap().is_some() {
                    return crate::selection::dispatch(app);
                }
                let presentation = *state.selection_presentation.lock().unwrap();
                if let Some(id) = presentation {
                    return crate::selection::present(app, id);
                }
            }
            windows::resume_navigation(app, target)
        }
        None => Ok(()),
    }
}

pub(crate) fn protect_result(app: &tauri::AppHandle) -> Result<(), String> {
    let changed = {
        let state = app.state::<crate::AppState>();
        let mut windows = state.windows.lock().unwrap();
        let changed = windows.records_browsing;
        windows.records_browsing = false;
        changed
    };
    if changed {
        crate::windows::emit_state(app)?;
    }
    Ok(())
}
