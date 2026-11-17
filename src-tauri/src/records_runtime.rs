use crate::{windows, AppState};
use std::sync::atomic::Ordering;
use tauri::{AppHandle, Emitter, Manager};

pub(crate) fn defer_navigation(app: &AppHandle, target: &str) -> Result<bool, String> {
    let state = app.state::<AppState>();
    if !state.records_dirty.load(Ordering::SeqCst) {
        return Ok(false);
    }
    *state.records_navigation.lock().unwrap() = Some(target.to_owned());
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    view.emit("records_navigation_requested", ())
        .map_err(|_| "event_emit")?;
    windows::focus_records_editor(app)?;
    Ok(true)
}

pub(crate) fn resolve_navigation(app: &AppHandle, proceed: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pending = state.records_navigation.lock().unwrap().take();
    if !proceed {
        if pending.as_deref() == Some("translation") {
            state.selection.lock().unwrap().take();
            state.selection_presentation.lock().unwrap().take();
            state.selection_source.lock().unwrap().take();
        }
        return Ok(());
    }
    state.records_dirty.store(false, Ordering::SeqCst);
    match pending.as_deref() {
        Some("view:clipboard") => windows::set_record_view(app, "clipboard"),
        Some("view:translation") => windows::set_record_view(app, "translation"),
        Some(target) => {
            if target == "translation" {
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
