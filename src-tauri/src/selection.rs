use crate::{platform, windows, AppState};
use serde::Serialize;
use std::sync::{atomic::Ordering, Arc};
use std::time::Duration;
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
    let target = platform::capture_selection_target();
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

const AX_STABILIZE_DELAY: Duration = Duration::from_millis(150);
const COPY_POLL_INTERVAL: Duration = Duration::from_millis(10);
const COPY_ATTEMPTS: usize = 22;
const COPY_SETTLE_INTERVAL: Duration = Duration::from_millis(10);
const COPY_SETTLE_ATTEMPTS: usize = 20;
const COPY_FINISH_ATTEMPTS: usize = 3;
const CAPTURE_ATTEMPTS: usize = 2;

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

fn record_phase(app: &AppHandle, phase: &'static str) {
    let owned = app.clone();
    let _ = app.run_on_main_thread(move || crate::record_lifecycle(&owned, phase, None));
}

async fn selection_ax<T: Send + 'static>(
    app: &AppHandle,
    revision: u64,
    operation: impl FnOnce() -> T + Send + 'static,
) -> Result<T, &'static str> {
    crate::commands::on_main(app, move |app| {
        selection_is_current(&app.state::<AppState>(), revision)
            .then_some(())
            .ok_or("translation_cancelled")
    })
    .await
    .map_err(|_| "main_thread_unavailable")??;
    tauri::async_runtime::spawn_blocking(operation)
        .await
        .map_err(|_| "selection_unavailable")
}

fn selection_is_current(state: &AppState, revision: u64) -> bool {
    state.selection_revision.load(Ordering::SeqCst) == revision
}

/// Reads the focused selection directly. When the first pass is empty, wakes
/// the application accessibility tree once and re-reads the focused element
/// from the application root.
async fn ax_capture(
    app: &AppHandle,
    target: Option<platform::Target>,
    revision: u64,
) -> Result<String, &'static str> {
    let direct = match selection_ax(app, revision, move || platform::selected_text(target)).await {
        Ok(Ok(text)) => text,
        // A missing focus or unavailable application is equivalent to an
        // empty direct read. Chromium-family applications may still expose the
        // selection after the application accessibility switch is enabled.
        Ok(Err("selection_application_unavailable" | "selection_focus_unavailable")) => {
            String::new()
        }
        Ok(Err(code)) => return Err(code),
        Err(code) => return Err(code),
    };
    if !direct.trim().is_empty() {
        record_phase(app, "selection_ax_text");
        return Ok(direct);
    }

    let Some(target) = target else {
        record_phase(app, "selection_ax_empty");
        return Ok(String::new());
    };
    match selection_ax(app, revision, move || {
        platform::enable_selection_accessibility(target)
    })
    .await??
    {
        platform::ManualAccessibility::Enabled => record_phase(app, "selection_ax_activated"),
        platform::ManualAccessibility::AlreadyEnabled
        | platform::ManualAccessibility::Unsupported => {
            record_phase(app, "selection_ax_empty");
            return Ok(String::new());
        }
    }
    if !selection_ax(app, revision, move || platform::wait_for_ax_ready(target)).await? {
        record_phase(app, "selection_ax_not_ready");
        return Ok(String::new());
    }
    tokio::time::sleep(AX_STABILIZE_DELAY).await;
    let text = selection_ax(app, revision, move || platform::selected_text(Some(target))).await??;
    record_phase(
        app,
        if text.trim().is_empty() {
            "selection_ax_empty"
        } else {
            "selection_ax_text"
        },
    );
    Ok(text)
}

fn copy_observed(
    initial_version: i64,
    initial_text: Option<&str>,
    initial_has_items: bool,
    payload: &platform::SelectionRead,
) -> bool {
    payload.version != initial_version
        || payload
            .text
            .as_deref()
            .is_some_and(|text| initial_text != Some(text))
        || (!initial_has_items && payload.has_items)
}

async fn wait_for_copy(
    app: &AppHandle,
    revision: u64,
    initial_version: i64,
    initial_text: Option<String>,
    initial_has_items: bool,
) -> Result<i64, &'static str> {
    for _ in 0..COPY_ATTEMPTS {
        tokio::time::sleep(COPY_POLL_INTERVAL).await;
        let payload = selection_payload(app, revision).await?;
        if copy_observed(
            initial_version,
            initial_text.as_deref(),
            initial_has_items,
            &payload,
        ) {
            return Ok(payload.version);
        }
    }
    record_phase(app, "selection_copy_no_change");
    Err("selection_copy_no_change")
}

async fn selection_payload(
    app: &AppHandle,
    revision: u64,
) -> Result<platform::SelectionRead, &'static str> {
    crate::commands::on_main(app, move |app| {
        selection_is_current(&app.state::<AppState>(), revision)
            .then(platform::read_selection_payload)
            .ok_or("translation_cancelled")
    })
    .await
    .map_err(|_| "main_thread_unavailable")?
}

/// `changeCount` advances when a source clears the pasteboard, before it writes
/// the copied value. Wait for a text payload (or a definitive non-text payload)
/// before reading and restoring, otherwise a fast poll can mistake the gap for
/// an empty selection and restore while the source is still writing.
async fn wait_for_selection_payload(
    app: &AppHandle,
    revision: u64,
) -> Result<(String, i64), &'static str> {
    let mut settled: Option<(i64, String)> = None;
    for _ in 0..COPY_SETTLE_ATTEMPTS {
        let payload = selection_payload(app, revision).await?;
        if let Some(text) = payload.text {
            if settled
                .as_ref()
                .is_some_and(|(version, expected)| *version == payload.version && expected == &text)
            {
                return Ok((text, payload.version));
            }
            settled = Some((payload.version, text));
        } else {
            settled = None;
        }
        tokio::time::sleep(COPY_SETTLE_INTERVAL).await;
    }
    let payload = selection_payload(app, revision).await?;
    Ok((payload.text.unwrap_or_default(), payload.version))
}

/// Snapshots the pasteboard, activates the captured application, sends the
/// layout-aware copy shortcut, waits for the copied payload to settle, then
/// restores the snapshot.
async fn copy_capture(
    app: &AppHandle,
    target: Option<platform::Target>,
    revision: u64,
) -> Result<String, &'static str> {
    let state = app.state::<AppState>();
    let _write = state.clipboard_writes.lock().await;
    if !selection_is_current(&state, revision) {
        return Err("translation_cancelled");
    }
    let _guard = CopyGuard::new(app);
    let saved = Arc::new(
        crate::commands::on_main(app, move |app| {
            if !selection_is_current(&app.state::<AppState>(), revision) {
                return Err("translation_cancelled");
            }
            let saved = platform::begin_selection_copy(target)?;
            crate::record_lifecycle(&app, "selection_copy_sent", None);
            Ok(saved)
        })
        .await
        .map_err(|_| "main_thread_unavailable")??,
    );
    wait_for_copy(
        app,
        revision,
        saved.version,
        saved.initial_text.clone(),
        saved.initial_has_items,
    )
    .await?;
    let (mut text, mut observed_version) = wait_for_selection_payload(app, revision).await?;
    for _ in 0..COPY_FINISH_ATTEMPTS {
        let saved = Arc::clone(&saved);
        let finish_text = text.clone();
        let result = crate::commands::on_main(app, move |app| {
            if !selection_is_current(&app.state::<AppState>(), revision) {
                return Err("translation_cancelled");
            }
            let (text, restored) =
                platform::finish_selection_copy(&saved, finish_text, observed_version)?;
            app.state::<AppState>()
                .clipboard_version
                .store(restored, Ordering::SeqCst);
            Ok(text)
        })
        .await
        .map_err(|_| "main_thread_unavailable")?;
        match result {
            Ok(text) => return Ok(text),
            Err("selection_copy_changed") => {
                let (next_text, next_version) = wait_for_selection_payload(app, revision).await?;
                text = next_text;
                observed_version = next_version;
            }
            Err(code) => return Err(code),
        }
    }
    Err("selection_copy_changed")
}

async fn capture_text(
    app: &AppHandle,
    target: Option<platform::Target>,
    revision: u64,
) -> Result<String, &'static str> {
    let mut last_error = "selection_unavailable";
    for attempt in 0..CAPTURE_ATTEMPTS {
        if let Ok(text) = ax_capture(app, target, revision).await {
            if !text.trim().is_empty() {
                return Ok(text);
            }
        }
        match copy_capture(app, target, revision).await {
            Ok(text) => return Ok(text),
            Err(code) => last_error = code,
        }
        if attempt + 1 < CAPTURE_ATTEMPTS {
            record_phase(app, "selection_retry");
        }
    }
    Err(last_error)
}

pub fn show_blank(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    state.selection_origin.lock().unwrap().take();
    state.translation_revision.fetch_add(1, Ordering::SeqCst);
    let revision = state.selection_revision.fetch_add(1, Ordering::SeqCst) + 1;
    queue_input(
        app,
        revision,
        platform::capture_target(),
        String::new(),
        None,
    )
}

pub(crate) fn queue_input(
    app: &AppHandle,
    revision: u64,
    target: Option<platform::Target>,
    text: String,
    error: Option<String>,
) -> Result<(), String> {
    let state = app.state::<AppState>();
    let source_app = if text.is_empty() {
        None
    } else {
        target.and_then(platform::target_app_info)
    };
    *state.selection_source.lock().unwrap() = source_app.clone();
    *state.selection.lock().unwrap() = Some(SelectionContext {
        text,
        source_app,
        error,
        presentation_id: Some(revision),
    });
    *state.selection_presentation.lock().unwrap() = Some(revision);
    *state.selection_target.lock().unwrap() = target;
    dispatch(app)
}

pub fn show_selected(app: &AppHandle, open_when_empty: bool) -> Result<(), String> {
    let state = app.state::<AppState>();
    if state.selection_busy.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let revision = state.selection_revision.fetch_add(1, Ordering::SeqCst) + 1;
    *state.selection.lock().unwrap() = None;
    *state.selection_presentation.lock().unwrap() = None;
    let origin = state
        .selection_origin
        .lock()
        .unwrap()
        .take()
        .unwrap_or_else(|| {
            let target = platform::capture_selection_target();
            SelectionOrigin { target }
        });
    let SelectionOrigin { target } = origin;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let guard = CaptureGuard(app.clone());
        let result = if !platform::accessibility() {
            Err("permission_required")
        } else {
            capture_text(&app, target, revision).await
        };
        let owned = app.clone();
        let _ = app.run_on_main_thread(move || {
            let _guard = guard;
            let state = owned.state::<AppState>();
            if !selection_is_current(&state, revision) {
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
                // A completed capture with no new clipboard version means there
                // was no selection; deliver an empty context instead of an error.
                Err("selection_copy_no_change" | "selection_empty" | "selection_secure") => {
                    (String::new(), None)
                }
                Err("target_unavailable") if target.is_none() => (String::new(), None),
                Err(code) => (String::new(), Some(code.to_string())),
            };
            if text.is_empty() && !open_when_empty && error.is_none() {
                return;
            }
            state.translation_revision.fetch_add(1, Ordering::SeqCst);
            if let Err(code) = queue_input(&owned, revision, target, text, error) {
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
    *state.target.lock().unwrap() = state.selection_target.lock().unwrap().take();
    state.operation.fetch_add(1, Ordering::SeqCst);
    windows::resume_navigation(app, "translation")
}

#[cfg(all(
    feature = "ui-tests",
    target_os = "macos",
    not(feature = "portable-ui")
))]
pub(crate) fn test_empty_summon(app: &AppHandle, open_when_empty: bool) -> Result<(), String> {
    // The UI handoff case must not depend on whichever real application happens
    // to be frontmost. Capture-empty and clipboard restore are covered by the
    // cross-app `empty` fixture; this helper exercises only empty delivery.
    if !matches!(
        app.config().identifier.as_str(),
        "dev.local.nami.ui-tests" | "dev.local.nami.ui-tests.denial"
    ) {
        return Err("permission_denied".into());
    }
    let state = app.state::<AppState>();
    if state.selection_busy.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let revision = state.selection_revision.fetch_add(1, Ordering::SeqCst) + 1;
    *state.selection_origin.lock().unwrap() = None;
    *state.selection.lock().unwrap() = None;
    *state.selection_presentation.lock().unwrap() = None;
    let owned = app.clone();
    tauri::async_runtime::spawn(async move {
        let guard = CaptureGuard(owned.clone());
        let handle = owned.clone();
        let _ = handle.run_on_main_thread(move || {
            let _guard = guard;
            let state = owned.state::<AppState>();
            if !selection_is_current(&state, revision) {
                return;
            }
            crate::record_lifecycle(&owned, "selection_result_empty", None);
            if !open_when_empty {
                return;
            }
            state.translation_revision.fetch_add(1, Ordering::SeqCst);
            if let Err(code) = queue_input(&owned, revision, None, String::new(), None) {
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
pub(crate) fn test_summon_from(
    app: &AppHandle,
    target: Option<platform::Target>,
    open_when_empty: bool,
) -> Result<(), String> {
    if !matches!(
        app.config().identifier.as_str(),
        "dev.local.nami.ui-tests" | "dev.local.nami.ui-tests.denial"
    ) {
        return Err("permission_denied".into());
    }
    *app.state::<AppState>().selection_origin.lock().unwrap() = Some(SelectionOrigin { target });
    show_selected(app, open_when_empty)
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

    #[test]
    fn a_late_write_without_a_version_change_is_observed() {
        let payload = platform::SelectionRead {
            version: 7,
            text: Some("copied".into()),
            has_items: true,
        };
        assert!(copy_observed(7, None, false, &payload));
    }

    #[test]
    fn a_version_change_is_observed_even_for_the_same_text() {
        let payload = platform::SelectionRead {
            version: 8,
            text: Some("same".into()),
            has_items: true,
        };
        assert!(copy_observed(7, Some("same"), true, &payload));
    }
}
