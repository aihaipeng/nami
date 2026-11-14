use crate::{model::WindowState, platform, AppState};
use std::sync::atomic::Ordering;
use tauri::{
    AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder,
    Window, WindowEvent,
};

pub(crate) use crate::history_runtime::{
    history_add, history_record, publish_history, select_native_history,
};

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
    if let Some(window) = app.get_window(label) {
        return Ok(window);
    }
    let (width, height) = match label {
        "translation" => (920.0, 620.0),
        "clipboard" => (420.0, 620.0),
        "settings" => (840.0, 640.0),
        _ => return Err("invalid_argument".into()),
    };
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
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
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

pub fn show(app: &AppHandle, target: &str) -> Result<(), String> {
    if !["translation", "clipboard", "settings"].contains(&target) {
        return Err("invalid_argument".into());
    }
    if target == "clipboard" {
        app.state::<AppState>()
            .operation
            .fetch_add(1, Ordering::SeqCst);
        *app.state::<AppState>().target.lock().unwrap() = platform::capture_target();
    }
    let window = create(app, target)?;
    if target == "translation" {
        app.state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .show_translation();
        apply_translation(app)?;
    }
    window.show().map_err(|_| "window_show")?;
    window.set_focus().map_err(|_| "window_focus")?;
    emit_state(app)
}

pub fn reveal_without_focus(window: &Window) -> Result<(), String> {
    window.show().map_err(|_| "window_show".into())
}

pub fn hide(app: &AppHandle, region: &str) -> Result<(), String> {
    match region {
        "translation" | "history" => {
            {
                let state = app.state::<AppState>();
                let mut state = state.windows.lock().unwrap();
                if region == "translation" {
                    state.dismiss();
                } else {
                    state.history_visible = false;
                }
            }
            apply_translation(app)
        }
        "clipboard" | "settings" => {
            if region == "clipboard" {
                app.state::<AppState>()
                    .operation
                    .fetch_add(1, Ordering::SeqCst);
            }
            if let Some(window) = app.get_window(region) {
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
