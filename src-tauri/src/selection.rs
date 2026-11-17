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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub presentation_id: Option<u64>,
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

const COPY_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(10);
const COPY_ATTEMPTS: usize = 120;

struct CopyGuard(AppHandle);
impl CopyGuard {
    fn new(app: &AppHandle) -> Self {
        app.state::<AppState>()
            .selection_copy_busy
            .store(true, Ordering::SeqCst);
        Self(app.clone())
    }
}
impl Drop for CopyGuard {
    fn drop(&mut self) {
        self.0
            .state::<AppState>()
            .selection_copy_busy
            .store(false, Ordering::SeqCst);
    }
}

async fn readable_selection(
    result: Result<String, &'static str>,
    copy: impl std::future::Future<Output = Result<String, &'static str>>,
) -> Result<String, &'static str> {
    match result {
        Ok(text) => Ok(if text.trim().is_empty() {
            String::new()
        } else {
            text
        }),
        Err("permission_required") => Err("permission_required"),
        Err(
            "selection_secure" | "selection_non_text" | "selection_empty" | "target_unavailable",
        ) => Ok(String::new()),
        Err(_) => copy.await,
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
    crate::commands::on_main(app, move |app| {
        finish_owned_copy(
            &app.state::<AppState>().clipboard_version,
            platform::clipboard_version,
            || platform::finish_selection_copy(&saved, expected, accept),
        )
    })
    .await
    .map_err(|_| "main_thread_unavailable")?
}

async fn copy_fallback(
    app: &AppHandle,
    target: platform::Target,
    revision: u64,
) -> Result<String, &'static str> {
    let _guard = CopyGuard::new(app);
    let saved = crate::commands::on_main(app, move |app| {
        if !selection_is_current(&app.state::<AppState>(), revision) {
            return Err("translation_cancelled");
        }
        platform::begin_selection_copy(target)
    })
    .await
    .map_err(|_| "main_thread_unavailable")??;
    let owned = app.clone();
    crate::commands::on_main(app, move |_| {
        crate::record_lifecycle(&owned, "selection_copy_sent", None)
    })
    .await
    .map_err(|_| "main_thread_unavailable")?;
    let (observed, accept_text) = wait_for_copy(saved.version, || async move {
        crate::commands::on_main(app, move |_| {
            (
                platform::clipboard_version(),
                platform::target_focused(target),
            )
        })
        .await
        .map_err(|_| "main_thread_unavailable")
    })
    .await?;
    if observed != saved.version {
        return restore_copy(app, saved, observed, accept_text).await;
    }
    // No change means no selection: never reuse the old clipboard text.
    let owned = app.clone();
    crate::commands::on_main(app, move |_| {
        crate::record_lifecycle(&owned, "selection_copy_no_change", None)
    })
    .await
    .map_err(|_| "main_thread_unavailable")?;
    Ok(String::new())
}

async fn wait_for_copy<F: std::future::Future<Output = Result<(i64, bool), &'static str>>>(
    initial: i64,
    mut sample: impl FnMut() -> F,
) -> Result<(i64, bool), &'static str> {
    let mut observed = initial;
    let mut stable = 0;
    let mut poll = tokio::time::interval_at(
        tokio::time::Instant::now() + COPY_POLL_INTERVAL,
        COPY_POLL_INTERVAL,
    );
    for attempt in 0..COPY_ATTEMPTS + 2 {
        poll.tick().await;
        let (version, focused) = sample().await?;
        if !focused {
            // Only restore a version already observed while the source had focus.
            return Ok((
                if observed != initial && observed == version {
                    observed
                } else {
                    initial
                },
                false,
            ));
        }
        if version == initial {
            if attempt + 1 >= COPY_ATTEMPTS {
                break;
            }
            continue;
        }
        if observed != initial && version != observed {
            return Err("clipboard_changed");
        }
        if observed == version {
            stable += 1;
        } else {
            observed = version;
            stable = 0;
        }
        if stable >= 2 {
            return Ok((observed, true));
        }
    }
    Ok((observed, false))
}

fn selection_is_current(state: &AppState, revision: u64) -> bool {
    state.selection_revision.load(Ordering::SeqCst) == revision
}

pub fn show_selected(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.selection_busy.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    state.translation_revision.fetch_add(1, Ordering::SeqCst);
    let revision = state.selection_revision.fetch_add(1, Ordering::SeqCst) + 1;
    *state.selection.lock().unwrap() = None;
    *state.selection_presentation.lock().unwrap() = None;
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
                readable_selection(direct, copy_fallback(&app, target, revision)).await
            }
        };
        let owned = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _guard = guard;
            let state = owned.state::<AppState>();
            if !selection_is_current(&state, revision)
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
                presentation_id: Some(revision),
            });
            *state.selection_presentation.lock().unwrap() = Some(revision);
            *state.target.lock().unwrap() = target;
            state.operation.fetch_add(1, Ordering::SeqCst);
            let result = dispatch(&owned);
            if let Err(code) = result {
                state.shortcut_errors.lock().unwrap().push(code);
            }
        });
    });
    Ok(())
}

pub(crate) fn dispatch(app: &AppHandle) -> Result<(), String> {
    if crate::records_runtime::defer_navigation(app, "translation")? {
        return Ok(());
    }
    windows::prewarm_records(app)?;
    app.get_webview("translation")
        .ok_or("window_unavailable")?
        .emit("selection_ready", ())
        .map_err(|_| "event_emit".into())
}

pub(crate) fn present(app: &AppHandle, presentation_id: u64) -> Result<(), String> {
    let state = app.state::<AppState>();
    if *state.selection_presentation.lock().unwrap() != Some(presentation_id) {
        return Ok(());
    }
    if crate::records_runtime::defer_navigation(app, "translation")? {
        return Ok(());
    }
    state.selection_presentation.lock().unwrap().take();
    windows::resume_navigation(app, "translation")
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
    fn an_older_context_cannot_cancel_the_next_selection_capture() {
        let state = AppState::default();
        let old = state.selection_revision.fetch_add(1, Ordering::SeqCst) + 1;
        let current = state.selection_revision.fetch_add(1, Ordering::SeqCst) + 1;
        // A late context may still cancel its translation while the next AX read is running.
        state.translation_revision.fetch_add(1, Ordering::SeqCst);
        assert!(selection_is_current(&state, current));
        assert!(!selection_is_current(&state, old));
    }

    #[tokio::test]
    async fn empty_secure_nontext_and_direct_text_never_start_copy() {
        for result in [
            Ok(String::new()),
            Ok(" \n\t".into()),
            Err("selection_non_text"),
            Err("selection_secure"),
            Err("selection_empty"),
            Err("target_unavailable"),
        ] {
            assert_eq!(
                readable_selection(result, async {
                    panic!("must not copy for a blank summon or protected selection")
                })
                .await,
                Ok(String::new())
            );
        }
        let text = "  Synthetic selected text\n第二行";
        assert_eq!(
            readable_selection(Ok(text.into()), async { panic!("AX text must not copy") }).await,
            Ok(text.into())
        );
        assert_eq!(
            readable_selection(Err("permission_required"), async {
                panic!("permission failure must not copy")
            })
            .await,
            Err("permission_required")
        );
    }

    #[tokio::test]
    async fn explicit_empty_selection_summons_without_waiting_for_copy() {
        let started = std::time::Instant::now();
        let result = readable_selection(Ok(String::new()), async {
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
            panic!("blank summon must never start the copy transaction");
        })
        .await;
        assert_eq!(result, Ok(String::new()));
        assert!(started.elapsed() < std::time::Duration::from_millis(50));
    }

    #[tokio::test]
    async fn delayed_copy_and_copy_at_the_deadline_are_still_accepted() {
        let mut polls = 0;
        assert_eq!(
            wait_for_copy(7, || {
                polls += 1;
                std::future::ready(Ok((if polls < 15 { 7 } else { 8 }, true)))
            })
            .await,
            Ok((8, true))
        );
        let mut polls = 0;
        assert_eq!(
            wait_for_copy(7, || {
                polls += 1;
                std::future::ready(Ok((if polls < COPY_ATTEMPTS { 7 } else { 8 }, true)))
            })
            .await,
            Ok((8, true))
        );
    }

    #[tokio::test]
    async fn ax_focus_failure_uses_copy_and_preserves_its_errors() {
        assert_eq!(
            readable_selection(Err("selection_focus_unavailable"), async {
                Ok("Synthetic fallback text".into())
            })
            .await,
            Ok("Synthetic fallback text".into())
        );
        for failure in [
            "clipboard_changed",
            "clipboard_restore_failed",
            "translation_cancelled",
        ] {
            assert_eq!(
                readable_selection(Err("selection_unavailable"), async { Err(failure) }).await,
                Err(failure)
            );
        }
        assert_eq!(
            readable_selection(Err("selection_focus_unavailable"), async {
                Ok(String::new())
            })
            .await,
            Ok(String::new())
        );
    }

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

    #[tokio::test]
    async fn blank_summon_stops_polling_without_accepting_the_old_clipboard() {
        let started = std::time::Instant::now();
        let mut polls = 0;
        assert_eq!(
            wait_for_copy(7, || {
                polls += 1;
                std::future::ready(Ok((7, true)))
            })
            .await,
            Ok((7, false))
        );
        assert_eq!(polls, COPY_ATTEMPTS);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        eprintln!("Synthetic blank-copy wait: {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn copied_version_must_stabilize_and_a_new_copy_is_never_restored() {
        assert_eq!(
            wait_for_copy(7, || std::future::ready(Ok((8, true)))).await,
            Ok((8, true))
        );
        let mut versions = [8, 9].into_iter();
        assert_eq!(
            wait_for_copy(7, || std::future::ready(Ok((
                versions.next().unwrap(),
                true
            ))))
            .await,
            Err("clipboard_changed")
        );
        assert_eq!(
            wait_for_copy(7, || std::future::ready(Ok((8, false)))).await,
            Ok((7, false))
        );
        let mut samples = [(8, true), (8, false)].into_iter();
        assert_eq!(
            wait_for_copy(7, || std::future::ready(Ok(samples.next().unwrap()))).await,
            Ok((8, false))
        );
    }
}
