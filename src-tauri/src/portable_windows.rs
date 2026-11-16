use crate::{
    model::{RecordView, WindowState},
    platform, AppState,
};
use std::sync::atomic::Ordering;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
    Window, WindowEvent,
};

pub(crate) use crate::history_runtime::{history_record, select_native_history};

// The shared React view consumes history_changed; this host owns no native list.
pub(crate) fn refresh_history(_: &AppHandle) -> Result<(), String> {
    Ok(())
}

pub fn update_translation_chrome(
    _: &AppHandle,
    _: String,
    _: f64,
    _: String,
    _: String,
    _: bool,
    _: bool,
) -> Result<(), String> {
    Err("platform_unavailable".into())
}

fn create(app: &AppHandle, label: &str) -> Result<Window, String> {
    if label == "clipboard" {
        return create(app, "translation");
    }
    if let Some(window) = app.get_window(label) {
        return Ok(window);
    }
    let (width, height) = match label {
        "translation" => (920.0, 620.0),
        "clipboard" => (420.0, 620.0),
        "settings" => (840.0, 640.0),
        _ => return Err("invalid_argument".into()),
    };
    let focused = !cfg!(feature = "ui-tests") || std::env::var_os("NAMI_UI_TEST_REPORT").is_none();
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(if label == "settings" {
            "Nami · Settings"
        } else {
            "Nami"
        })
        .inner_size(width, height)
        .min_inner_size(340.0, 440.0)
        .decorations(true)
        .transparent(false)
        .visible(false)
        .focused(focused)
        .center()
        .build()
        .map_err(|_| "window_create")?;
    let window = window.as_ref().window();
    let handle = app.clone();
    let label = label.to_owned();
    window.on_window_event(move |event| {
        let result = match event {
            WindowEvent::CloseRequested { api, .. } => {
                api.prevent_close();
                hide(&handle, &label)
            }
            WindowEvent::Resized(size) if label == "translation" => {
                if let Some(window) = handle.get_window("translation") {
                    let scale = window.scale_factor().unwrap_or(1.0);
                    let width = size.to_logical::<f64>(scale).width;
                    handle
                        .state::<AppState>()
                        .windows
                        .lock()
                        .unwrap()
                        .resize_visible_width(width);
                    emit_state(&handle)
                } else {
                    Ok(())
                }
            }
            WindowEvent::Focused(false) if label == "translation" => {
                handle.state::<AppState>().windows.lock().unwrap().blur();
                apply_translation(&handle)
            }
            WindowEvent::Focused(false) if label == "clipboard" => {
                if !handle
                    .state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .clipboard_pinned
                {
                    hide(&handle, &label)
                } else {
                    Ok(())
                }
            }
            _ => Ok(()),
        };
        if let Err(code) = result {
            let _ = handle.emit("layout_error", code);
        }
    });
    Ok(window)
}

pub fn emit_state(app: &AppHandle) -> Result<(), String> {
    let (state, ended) = {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        let ended = regions.sync_records_session();
        if ended {
            regions.record_view = RecordView::Clipboard;
        }
        (regions.clone(), ended)
    };
    if ended {
        crate::clipboard_runtime::visibility(app, false)?;
        app.state::<AppState>()
            .records_dirty
            .store(false, Ordering::SeqCst);
        app.state::<AppState>()
            .records_navigation
            .lock()
            .unwrap()
            .take();
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            let _ = crate::history_runtime::query_history(&app, String::new(), false).await;
        });
    }
    if !state.translation_visible {
        app.state::<AppState>()
            .history
            .lock()
            .unwrap()
            .clear_visual_selection();
        crate::history_runtime::publish_history(app)?;
    }
    for label in ["translation", "history", "clipboard", "settings"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("window_state", &state)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

fn visible_width(state: &WindowState) -> f64 {
    (if state.history_visible {
        state.history_width
    } else {
        0.0
    }) + if state.translation_visible {
        state.translation_width
    } else {
        0.0
    }
}

fn layout(app: &AppHandle, anchor_right: bool) -> Result<(), String> {
    let (state, anchor_right) = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        let anchor_right = keep_right_anchor(&state, anchor_right);
        state.history_extent = if state.history_visible {
            state.history_width
        } else {
            0.0
        };
        state.translation_extent = if state.translation_visible {
            state.translation_width
        } else {
            0.0
        };
        (state.clone(), anchor_right)
    };
    let window = app.get_window("translation").ok_or("window_unavailable")?;
    if !state.translation_visible && !state.history_visible {
        window.hide().map_err(|_| "window_hide")?;
        return emit_state(app);
    }
    let scale = window.scale_factor().map_err(|_| "window_geometry")?;
    let size = window
        .inner_size()
        .map_err(|_| "window_geometry")?
        .to_logical::<f64>(scale);
    let width = visible_width(&state);
    let minimum = if state.translation_visible {
        state.translation_min_width()
            + if state.history_visible {
                state.history_width
            } else {
                0.0
            }
    } else {
        280.0
    };
    window
        .set_min_size(Some(LogicalSize::new(minimum, 440.0)))
        .map_err(|_| "window_geometry")?;
    window
        .set_max_size(if state.translation_visible {
            None
        } else {
            Some(LogicalSize::new(720.0, 100_000.0))
        })
        .map_err(|_| "window_geometry")?;
    if (size.width - width).abs() > 0.5 {
        let position = window
            .outer_position()
            .map_err(|_| "window_geometry")?
            .to_logical::<f64>(scale);
        window
            .set_size(LogicalSize::new(width, size.height))
            .map_err(|_| "window_geometry")?;
        if anchor_right {
            window
                .set_position(LogicalPosition::new(
                    position.x + size.width - width,
                    position.y,
                ))
                .map_err(|_| "window_geometry")?;
        }
    }
    window
        .set_always_on_top(
            (state.translation_visible && state.translation_pinned)
                || (state.history_visible && state.history_pinned),
        )
        .map_err(|_| "window_pin")?;
    window.show().map_err(|_| "window_show")?;
    emit_state(app)
}

fn keep_right_anchor(state: &WindowState, requested: bool) -> bool {
    // Changing the right card keeps the history's left edge; changing history
    // keeps the translation's right edge. Seam resizing explicitly keeps left.
    requested && ((state.translation_extent > 0.0) == state.translation_visible)
}

pub fn apply_translation(app: &AppHandle) -> Result<(), String> {
    layout(app, true)
}

pub fn resize_history(app: &AppHandle, width: f64, initialize: bool) -> Result<(), String> {
    if !width.is_finite() {
        return Err("invalid_argument".into());
    }
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if initialize && state.history_width_initialized {
            return Ok(());
        }
        state.history_width = width.clamp(280.0, 720.0);
        state.history_width_initialized = true;
    }
    platform::save_history_width(width.clamp(280.0, 720.0));
    layout(app, false)
}

pub fn update_translation_level(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_window("translation") {
        let state = app.state::<AppState>().windows.lock().unwrap().clone();
        window
            .set_always_on_top(
                (state.translation_visible && state.translation_pinned)
                    || (state.history_visible && state.history_pinned),
            )
            .map_err(|_| "window_pin")?;
    }
    emit_state(app)
}

pub fn set_record_view(app: &AppHandle, view: &str) -> Result<(), String> {
    let view = match view {
        "clipboard" => RecordView::Clipboard,
        "translation" => RecordView::Translation,
        _ => return Err("invalid_argument".into()),
    };
    if app.state::<AppState>().windows.lock().unwrap().record_view == view {
        return Ok(());
    }
    if crate::records_runtime::defer_navigation(
        app,
        if view == RecordView::Clipboard {
            "view:clipboard"
        } else {
            "view:translation"
        },
    )? {
        return Ok(());
    }
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .set_record_view(view);
    apply_translation(app)
}

pub fn toggle_record_details(app: &AppHandle) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        if regions.record_view != RecordView::Translation {
            return Err("details_unavailable".into());
        }
        regions.translation_visible = !regions.translation_visible;
        if regions.translation_visible {
            state.history.lock().unwrap().restore_visual_selection();
        }
    }
    crate::history_runtime::publish_history(app)?;
    apply_translation(app)
}

pub fn show(app: &AppHandle, target: &str) -> Result<(), String> {
    show_entry(app, target, true)
}

pub fn resume_navigation(app: &AppHandle, target: &str) -> Result<(), String> {
    show_entry(app, target, false)
}

fn show_entry(app: &AppHandle, target: &str, capture: bool) -> Result<(), String> {
    if !["translation", "clipboard", "settings"].contains(&target) {
        return Err("invalid_argument".into());
    }
    if capture && target != "settings" {
        app.state::<AppState>()
            .operation
            .fetch_add(1, Ordering::SeqCst);
        *app.state::<AppState>().target.lock().unwrap() = platform::capture_target();
    }
    if target != "settings" && crate::records_runtime::defer_navigation(app, target)? {
        return Ok(());
    }
    let window = create(app, target)?;
    if target != "settings" {
        {
            let state = app.state::<AppState>();
            let mut regions = state.windows.lock().unwrap();
            if target == "translation" {
                regions.summon_translation();
            } else {
                regions.set_record_view(RecordView::Clipboard);
                regions.records_focus += 1;
            }
        }
        apply_translation(app)?;
    }
    window.show().map_err(|_| "window_show")?;
    #[cfg(feature = "ui-tests")]
    if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
        return emit_state(app);
    }
    window.set_focus().map_err(|_| "window_focus")?;
    if let Some(view) = app.get_webview(if target == "clipboard" {
        "translation"
    } else {
        target
    }) {
        view.set_focus().map_err(|_| "window_focus")?;
    }
    emit_state(app)
}

pub fn focus_records_editor(app: &AppHandle) -> Result<(), String> {
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    view.window().set_focus().map_err(|_| "window_focus")?;
    view.set_focus().map_err(|_| "window_focus".into())
}

pub fn reveal_clipboard_failure(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .set_record_view(RecordView::Clipboard);
    apply_translation(app)?;
    if let Some(window) = app.get_window("translation") {
        reveal_without_focus(&window)?;
    }
    Ok(())
}

pub fn hide_after_clipboard_use(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>().windows.lock().unwrap().dismiss();
    apply_translation(app)
}

pub fn reveal_without_focus(window: &Window) -> Result<(), String> {
    window.show().map_err(|_| "window_show".into())
}

pub fn hide(app: &AppHandle, region: &str) -> Result<(), String> {
    match region {
        "translation" | "history" | "clipboard" => {
            if region == "clipboard" {
                app.state::<AppState>()
                    .operation
                    .fetch_add(1, Ordering::SeqCst);
            }
            {
                let state = app.state::<AppState>();
                let mut regions = state.windows.lock().unwrap();
                if region == "history" {
                    regions.history_visible = false;
                } else {
                    regions.dismiss();
                }
            }
            apply_translation(app)
        }
        "settings" => {
            if let Some(window) = app.get_window("settings") {
                window.hide().map_err(|_| "window_hide")?;
            }
            Ok(())
        }
        _ => Err("invalid_argument".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn shared_host_width_only_includes_visible_regions() {
        let mut state = WindowState::default();
        assert_eq!(visible_width(&state), 920.0);
        state.history_visible = true;
        assert_eq!(visible_width(&state), 1280.0);
        state.translation_visible = false;
        assert_eq!(visible_width(&state), 360.0);
        state.history_visible = false;
        assert_eq!(visible_width(&state), 0.0);
    }

    #[test]
    fn translation_open_close_keeps_history_left_while_history_toggle_keeps_right() {
        let mut state = WindowState {
            history_visible: true,
            history_extent: 360.0,
            ..WindowState::default()
        };
        assert!(keep_right_anchor(&state, true));
        state.translation_visible = false;
        assert!(!keep_right_anchor(&state, true));
        state.translation_extent = 0.0;
        state.translation_visible = true;
        assert!(!keep_right_anchor(&state, true));
        state.translation_extent = state.translation_width;
        state.history_visible = false;
        assert!(keep_right_anchor(&state, true));
        assert!(!keep_right_anchor(&state, false));
    }
}
