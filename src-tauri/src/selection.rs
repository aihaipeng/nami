use crate::{platform, windows, AppState};
use serde::Serialize;
use std::sync::atomic::Ordering;
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

struct CaptureGuard(AppHandle);
impl Drop for CaptureGuard {
    fn drop(&mut self) {
        self.0
            .state::<AppState>()
            .selection_busy
            .store(false, Ordering::SeqCst);
    }
}

fn readable_selection(result: Result<String, &'static str>) -> Result<String, &'static str> {
    match result {
        Ok(text) if !text.trim().is_empty() => Ok(text),
        Err("permission_required") => Err("permission_required"),
        _ => Ok(String::new()),
    }
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
                let _ =
                    app.run_on_main_thread(move || crate::record_lifecycle(&owned, &phase, None));
                readable_selection(direct)
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

#[cfg(all(
    feature = "ui-tests",
    target_os = "macos",
    not(feature = "portable-ui")
))]
pub(crate) fn test_empty_summon(app: &AppHandle) -> Result<(), String> {
    if app.config().identifier != "dev.local.nami.ui-tests" {
        return Err("permission_denied".into());
    }
    *app.state::<AppState>().selection_origin.lock().unwrap() =
        Some(SelectionOrigin { target: None });
    show_selected(app)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unreadable_or_empty_selection_opens_blank_without_clipboard_work() {
        for result in [
            Ok(String::new()),
            Ok(" \n\t".into()),
            Err("selection_unavailable"),
            Err("selection_non_text"),
            Err("selection_secure"),
            Err("selection_empty"),
            Err("target_unavailable"),
        ] {
            assert_eq!(readable_selection(result), Ok(String::new()));
        }
    }

    #[test]
    fn direct_selected_text_and_permission_errors_keep_their_meaning() {
        let text = "  Synthetic selected text\n第二行";
        assert_eq!(readable_selection(Ok(text.into())), Ok(text.into()));
        assert_eq!(
            readable_selection(Err("permission_required")),
            Err("permission_required")
        );
    }
}
