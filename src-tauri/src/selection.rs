use crate::{platform, windows, AppState};
use serde::Serialize;
use std::sync::atomic::{AtomicI64, Ordering};
use tauri::{AppHandle, Emitter, Manager};

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectionContext {
    pub text: String,
    pub source_app: Option<serde_json::Value>,
    pub error: Option<String>,
}

pub struct SelectionOrigin {
    target: Option<platform::Target>,
}

pub fn prepare_selected(app: &AppHandle) {
    let state = app.state::<AppState>();
    if state.selection_busy.load(Ordering::SeqCst) {
        return;
    }
    let target = platform::capture_target();
    crate::record_lifecycle(
        app,
        if target.is_some() {
            "selection_pressed_target"
        } else {
            "selection_pressed_no_target"
        },
        None,
    );
    *state.selection_origin.lock().unwrap() = Some(SelectionOrigin { target });
}

async fn on_main<T: Send + 'static>(
    app: &AppHandle,
    work: impl FnOnce() -> T + Send + 'static,
) -> Result<T, &'static str> {
    let (send, receive) = tokio::sync::oneshot::channel();
    app.run_on_main_thread(move || {
        let _ = send.send(work());
    })
    .map_err(|_| "selection_unavailable")?;
    receive.await.map_err(|_| "selection_unavailable")
}

struct CaptureGuard(AppHandle);
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.0
            .state::<AppState>()
            .selection_busy
            .store(false, Ordering::SeqCst);
    }
}

fn finish_owned_copy<T>(
    handled: &AtomicI64,
    current_version: impl FnOnce() -> i64,
    finish: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let result = finish()?;
    handled.store(current_version(), Ordering::SeqCst);
    Ok(result)
}

async fn restore_copy(
    app: &AppHandle,
    saved: platform::CopySnapshot,
    expected: i64,
    accept: bool,
) -> Result<String, &'static str> {
    let owned = app.clone();
    on_main(app, move || {
        finish_owned_copy(
            &owned.state::<AppState>().clipboard_version,
            platform::clipboard_version,
            || platform::finish_selection_copy(&saved, expected, accept),
        )
    })
    .await?
}

async fn copy_fallback(app: &AppHandle, target: platform::Target) -> Result<String, &'static str> {
    let saved = on_main(app, move || platform::begin_selection_copy(target)).await??;
    let owned = app.clone();
    on_main(app, move || {
        crate::record_lifecycle(&owned, "selection_copy_sent", None)
    })
    .await?;
    let mut observed = saved.version;
    let mut stable = 0;
    for _ in 0..60 {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let (version, focused) = on_main(app, move || {
            (
                platform::clipboard_version(),
                platform::target_focused(target),
            )
        })
        .await?;
        if !focused {
            if observed != saved.version && observed == version {
                restore_copy(app, saved, observed, false).await?;
            }
            // A changed clipboard after focus moved cannot safely be attributed to our copy.
            return Err("target_unavailable");
        }
        if version == saved.version {
            continue;
        }
        if observed != saved.version && version != observed {
            return Err("clipboard_changed");
        }
        if observed == version {
            stable += 1;
        } else {
            observed = version;
            stable = 0;
        }
        if stable >= 2 {
            return restore_copy(app, saved, version, true).await;
        }
    }
    if observed != saved.version {
        restore_copy(app, saved, observed, false).await?;
    }
    // No change means no selection: never reuse the old clipboard text.
    let owned = app.clone();
    on_main(app, move || {
        crate::record_lifecycle(&owned, "selection_copy_no_change", None)
    })
    .await?;
    Ok(String::new())
}

pub fn show_selected(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.selection_busy.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let revision = state.translation_revision.fetch_add(1, Ordering::SeqCst) + 1;
    *state.selection.lock().unwrap() = None;
    let origin = state
        .selection_origin
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| {
            let target = platform::capture_target();
            SelectionOrigin { target }
        });
    let SelectionOrigin { target } = origin;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let guard = CaptureGuard(app.clone());
        let result = match target {
            None => Ok(String::new()),
            Some(target) => {
                let direct =
                    tauri::async_runtime::spawn_blocking(move || platform::selected_text(target))
                        .await
                        .unwrap_or(Err("selection_unavailable"));
                let phase = match &direct {
                    Ok(_) => "selection_ax_text".to_string(),
                    Err(code) => format!("selection_ax_{code}"),
                };
                let owned = app.clone();
                let _ = on_main(&app, move || crate::record_lifecycle(&owned, &phase, None)).await;
                match direct {
                    Ok(text) => Ok(text),
                    Err("selection_secure" | "selection_non_text") => Ok(String::new()),
                    Err("permission_required") => Err("permission_required"),
                    Err(_) => copy_fallback(&app, target).await,
                }
            }
        };
        let owned = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _guard = guard;
            let state = owned.state::<AppState>();
            if state.translation_revision.load(Ordering::SeqCst) != revision
                || target.is_some_and(|target| !platform::target_focused(target))
            {
                crate::record_lifecycle(&owned, "selection_discarded", None);
                return;
            }
            let phase = match &result {
                Ok(text) if text.is_empty() => "selection_result_empty".to_string(),
                Ok(_) => "selection_result_text".to_string(),
                Err(code) => format!("selection_error_{code}"),
            };
            crate::record_lifecycle(&owned, &phase, None);
            let (text, error) = match result {
                Ok(text) => (text, None),
                Err("selection_empty" | "selection_secure") => (String::new(), None),
                Err(code) => (String::new(), Some(code.to_string())),
            };
            // An ordinary summon has no source record: do not export an app icon for it.
            let source_app = if text.is_empty() {
                None
            } else {
                target.and_then(platform::target_app_info)
            };
            *state.selection_source.lock().unwrap() = if text.is_empty() {
                None
            } else {
                source_app.clone()
            };
            *state.selection.lock().unwrap() = Some(SelectionContext {
                text,
                source_app,
                error,
            });
            let result = windows::show(&owned, "translation").and_then(|_| {
                if state.records_navigation.lock().unwrap().is_none() {
                    if let Some(view) = owned.get_webview("translation") {
                        view.emit("selection_ready", ()).map_err(|_| "event_emit")?;
                    }
                }
                Ok(())
            });
            if let Err(code) = result {
                state.shortcut_errors.lock().unwrap().push(code);
            }
        });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_successful_owned_restoration_suppresses_clipboard_capture() {
        let handled = AtomicI64::new(7);
        assert_eq!(
            finish_owned_copy(
                &handled,
                || panic!("external version must not be suppressed"),
                || Err::<(), _>("clipboard_changed")
            ),
            Err("clipboard_changed")
        );
        assert_eq!(handled.load(Ordering::SeqCst), 7);
        assert_eq!(
            finish_owned_copy(&handled, || 10, || Ok("synthetic restored text")),
            Ok("synthetic restored text")
        );
        assert_eq!(handled.load(Ordering::SeqCst), 10);
    }
}
