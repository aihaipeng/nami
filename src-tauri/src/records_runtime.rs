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
    windows::focus_records_editor(app)?;
    Ok(true)
}

pub(crate) fn toggle_translation_history(app: &AppHandle) -> Result<(), String> {
    if defer_navigation(app, "toggle:history")? {
        return Ok(());
    }
    let state = app.state::<AppState>();
    let was_hidden = app
        .get_window("translation")
        .map(|window| window.is_visible())
        .transpose()
        .map_err(|_| "window_geometry")?
        != Some(true);
    let (history, translation) = {
        let mut regions = state.windows.lock().unwrap();
        if was_hidden {
            regions.dismiss();
        }
        if regions.history_visible && regions.record_view == nami::model::RecordView::Clipboard {
            state.operation.fetch_add(1, Ordering::SeqCst);
        }
        regions.toggle_history();
        if regions.history_visible {
            regions.records_focus += 1;
        }
        (regions.history_visible, regions.translation_visible)
    };
    if was_hidden {
        *state.target.lock().unwrap() = crate::platform::capture_target();
        windows::prewarm_records(app)?;
    }
    windows::apply_translation(app)?;
    if history && crate::platform::capabilities().native_history {
        if was_hidden {
            app.get_window("translation")
                .ok_or("window_unavailable")?
                .show()
                .map_err(|_| "window_show")?;
        }
        windows::focus_records_editor(app)?;
    } else if translation {
        if let Some(view) = app.get_webview("translation") {
            view.set_focus().map_err(|_| "window_focus")?;
        }
    }
    Ok(())
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
    state.image_text_operation.fetch_add(1, Ordering::SeqCst);
    state.operation.fetch_add(1, Ordering::SeqCst);
    windows::hide(
        app,
        if region == "history" {
            "translation"
        } else {
            region
        },
    )
}

pub(crate) fn resolve_navigation(app: &AppHandle, proceed: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    let pending = state.records_navigation.lock().unwrap().take();
    let action = pending.as_ref().map(|pending| pending.action.as_str());
    if !proceed {
        if action == Some("translation") {
            state.selection.lock().unwrap().take();
            state.selection_presentation.lock().unwrap().take();
            state.selection_source.lock().unwrap().take();
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
        Some("view:clipboard") => windows::set_record_view(app, "clipboard"),
        Some("view:translation") => windows::set_record_view(app, "translation"),
        Some("close:records") => dismiss_records_window(app, "translation"),
        Some("toggle:history") => toggle_translation_history(app),
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
