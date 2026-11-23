use crate::{model::HistoryRecord, AppState};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

const HISTORY_VIEWS: [&str; 3] = ["translation", "history", "clipboard"];

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct VisualSelection {
    revision: u64,
    sample_id: Option<String>,
    retranslate: bool,
    clear_detail: bool,
    visual_only: bool,
    record: Option<HistoryRecord>,
}

fn history_consumer_labels() -> &'static [&'static str] {
    if crate::platform::capabilities().native_history {
        &["history", "clipboard"]
    } else {
        &HISTORY_VIEWS
    }
}

fn history_consumer_views(app: &AppHandle) -> Vec<tauri::Webview> {
    history_consumer_labels()
        .iter()
        .filter_map(|label| app.get_webview(label))
        .collect()
}

fn record_storage_failure(
    error: String,
    stored_error: &std::sync::Mutex<Option<String>>,
    mut notify: impl FnMut(&str, &str),
) -> String {
    if matches!(
        error.as_str(),
        "storage_corrupt" | "storage_schema" | "storage_version"
    ) {
        *stored_error.lock().unwrap() = Some(error.clone());
        for label in HISTORY_VIEWS.into_iter().chain(std::iter::once("settings")) {
            notify(label, &error);
        }
    }
    error
}

fn storage_failure(app: &AppHandle, error: String) -> String {
    let state = app.state::<AppState>();
    record_storage_failure(error, &state.storage_error, |label, error| {
        if let Some(view) = app.get_webview(label) {
            let _ = view.emit("storage_error", error);
        }
    })
}

pub(crate) fn select_native_history(app: &AppHandle, id: Option<&str>) -> Result<u64, String> {
    let state = app.state::<AppState>();
    let mut history = state.history.lock().unwrap();
    history.select(id)?;
    Ok(history.revision())
}

pub(crate) fn history_record(app: &AppHandle, id: &str) -> Option<HistoryRecord> {
    app.state::<AppState>().history.lock().unwrap().record(id)
}

#[cfg(all(
    feature = "ui-tests",
    target_os = "macos",
    not(feature = "portable-ui")
))]
pub(crate) fn history_add(app: &AppHandle, record: HistoryRecord) -> Result<(), String> {
    app.state::<AppState>()
        .history
        .lock()
        .unwrap()
        .add(record)?;
    app.state::<AppState>()
        .history_page
        .lock()
        .unwrap()
        .cache
        .clear();
    publish_history(app)
}

pub(crate) fn publish_history_snapshot(
    app: &AppHandle,
    snapshot: &crate::history::HistoryWireSnapshot,
) -> Result<(), String> {
    let views = history_consumer_views(app);
    for view in views {
        view.emit("history_changed", snapshot)
            .map_err(|_| "event_emit")?;
    }
    Ok(())
}

pub(crate) fn publish_history(app: &AppHandle) -> Result<(), String> {
    let views = history_consumer_views(app);
    if views.is_empty() {
        return Ok(());
    }
    let snapshot = app
        .state::<AppState>()
        .history
        .lock()
        .unwrap()
        .wire_snapshot();
    for view in views {
        view.emit("history_changed", &snapshot)
            .map_err(|_| "event_emit")?;
    }
    Ok(())
}

pub(crate) fn publish_visual_selection(
    app: &AppHandle,
    revision: u64,
    sample_id: Option<String>,
) -> Result<(), String> {
    let views = history_consumer_views(app);
    if views.is_empty() {
        return Ok(());
    }
    let selection = VisualSelection {
        revision,
        sample_id,
        retranslate: false,
        clear_detail: false,
        visual_only: true,
        record: None,
    };
    for view in views {
        view.emit("history_sample_selected", &selection)
            .map_err(|_| "event_emit")?;
    }
    Ok(())
}

pub(crate) async fn update_record(
    app: &AppHandle,
    id: String,
    action: String,
    title: Option<String>,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    let state = app.state::<AppState>();
    let _write = state.history_writes.lock().await;
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let worker = app.clone();
    let worker_id = id.clone();
    let (previous, next, replacement) = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        let (previous, next) = {
            let history = state.history.lock().unwrap();
            match (action.as_str(), title.as_deref()) {
                ("note", Some(title)) => history.prepare_title(&worker_id, title),
                ("note", None) | (_, Some(_)) => Err("invalid_argument".into()),
                _ => history.prepare_update(&worker_id, &action),
            }?
        };
        let query = state.history_page.lock().unwrap().query.clone();
        let reload_count = {
            let history = state.history.lock().unwrap();
            history
                .requires_update_query(&previous, next.as_ref(), &query)
                .then(|| history.len() + 1)
        };
        let mut databases = state.database.lock().unwrap();
        let replacement = if let Some(database) = databases.as_mut() {
            database.update(&previous, next.as_ref())?;
            reload_count
                .map(|count| database.page_prefix(&query, count))
                .transpose()?
        } else {
            None
        };
        Ok::<_, String>((previous, next, replacement))
    })
    .await
    .map_err(|_| "storage_worker")?
    .map_err(|error| storage_failure(app, error))?;
    crate::commands::on_main(app, move |app| {
        let query = app
            .state::<AppState>()
            .history_page
            .lock()
            .unwrap()
            .query
            .clone();
        let state = app.state::<AppState>();
        state
            .history_page
            .lock()
            .unwrap()
            .cache
            .update_record(&previous, next.as_ref());
        let snapshot = if let Some(page) = replacement {
            let cursor = page.cursor.clone();
            let (loaded, snapshot) = {
                let mut history = state.history.lock().unwrap();
                history.apply_updated_page(page, &id, next);
                (history.loaded_page(cursor.clone()), history.wire_snapshot())
            };
            let mut paging = state.history_page.lock().unwrap();
            paging.cursor = cursor;
            paging.cache.insert(query, loaded);
            snapshot
        } else {
            let mut history = state.history.lock().unwrap();
            history.apply_update(&previous, next, &query)?;
            history.wire_snapshot()
        };
        publish_history_snapshot(&app, &snapshot)?;
        Ok(snapshot)
    })
    .await?
}

fn cleanup_page(
    state: &AppState,
    database: &crate::database::HistoryDatabase,
    removed: &[String],
) -> Result<Option<crate::database::HistoryPage>, String> {
    if removed.is_empty() {
        return Ok(None);
    }
    let paging = state.history_page.lock().unwrap();
    database
        .page_prefix(&paging.query, paging.cache.loaded_count(&paging.query))
        .map(Some)
}

fn apply_cleanup_page(app: &AppHandle, page: crate::database::HistoryPage, removed: &[String]) {
    let state = app.state::<AppState>();
    {
        let mut paging = state.history_page.lock().unwrap();
        let query = paging.query.clone();
        paging.cursor = page.cursor.clone();
        paging.cache.clear();
        paging.cache.insert(query, page.clone());
    }
    let removed_detail = state
        .history
        .lock()
        .unwrap()
        .apply_cleanup_page(page, removed);
    if let (Some(id), Some(view)) = (removed_detail, app.get_webview("translation")) {
        let _ = view.emit("history_detail_removed", id);
    }
}

pub(crate) async fn cleanup_translations(
    app: &AppHandle,
    policy: Option<nami::clipboard::HistoryPolicy>,
    categories: u8,
    token: String,
) -> Result<usize, String> {
    let state = app.state::<AppState>();
    let _write = state.history_writes.lock().await;
    let worker = app.clone();
    let (removed, page) = crate::clipboard_runtime::database(app, move |database| {
        let removed = database.translation_cleanup(policy, categories, &token)?;
        let page = cleanup_page(&worker.state::<AppState>(), database, &removed)?;
        Ok((removed, page))
    })
    .await
    .map_err(|error| storage_failure(app, error))?;
    let count = removed.len();
    if let Some(page) = page {
        crate::commands::on_main(app, move |app| {
            apply_cleanup_page(&app, page, &removed);
            publish_history(&app)
        })
        .await??;
    }
    Ok(count)
}

#[derive(Clone)]
pub(crate) enum TranslationFocus {
    History(String),
    Clipboard(u64),
}

pub(crate) async fn commit_translation(
    app: &AppHandle,
    record: HistoryRecord,
    engine: String,
    revision: u64,
    focus_origin: Option<TranslationFocus>,
) -> Result<HistoryRecord, String> {
    let state = app.state::<AppState>();
    let _write = state.history_writes.lock().await;
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let worker = app.clone();
    let saved = record.clone();
    let (removed, replacement) = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        let current = || {
            state
                .translation_revision
                .load(std::sync::atomic::Ordering::SeqCst)
                == revision
        };
        if !current() {
            return Err("cancelled".into());
        }
        if let Some(database) = state.database.lock().unwrap().as_mut() {
            let removed = database.insert_retained(&saved, &engine, current)?;
            let replacement = cleanup_page(&state, database, &removed)?;
            return Ok((removed, replacement));
        }
        Ok::<_, String>((vec![], None))
    })
    .await
    .map_err(|_| "storage_worker")?
    .map_err(|error| storage_failure(app, error))?;
    // A committed result remains history even if a newer request starts before UI delivery.
    crate::commands::on_main(app, move |app| {
        let state = app.state::<AppState>();
        let current = state
            .translation_revision
            .load(std::sync::atomic::Ordering::SeqCst)
            == revision;
        let (can_refocus, browsing, session) = {
            let window = state.windows.lock().unwrap();
            (
                focus_origin.is_some()
                    && current
                    && window.history_visible
                    && window.translation_visible
                    && window.record_view == crate::model::RecordView::Translation
                    && !window.records_editor_open,
                window.records_browsing,
                window.records_session,
            )
        };
        let query = {
            let mut page = state.history_page.lock().unwrap();
            page.cache.insert_record(&record);
            page.query.clone()
        };
        let refocus = {
            let mut history = state.history.lock().unwrap();
            let refocus = can_refocus
                && match &focus_origin {
                    Some(TranslationFocus::History(id)) => {
                        history.selected_id() == Some(id.as_str())
                    }
                    Some(TranslationFocus::Clipboard(started)) => {
                        session == *started
                            && !browsing
                            && app
                                .get_window("translation")
                                .is_some_and(|window| window.is_focused().unwrap_or(false))
                    }
                    None => false,
                };
            history.add_completed(record.clone(), current && (!browsing || refocus), &query)?;
            refocus
        };
        if let Some(page) = replacement {
            apply_cleanup_page(&app, page, &removed);
        }
        publish_history(&app)?;
        if refocus {
            let view = ["clipboard", "history", "translation"]
                .into_iter()
                .find_map(|label| app.get_webview(label));
            if let Some(view) = view {
                let _ = view.set_focus();
                view.emit("history_refocus", &record.id)
                    .map_err(|_| "event_emit")?;
            }
        }
        Ok(record)
    })
    .await?
}

#[derive(Clone, Default)]
pub(crate) struct PageState {
    pub query: crate::database::HistoryQuery,
    pub cursor: Option<crate::database::HistoryCursor>,
    pub cache: crate::history::HistoryPageCache,
}

pub(crate) async fn query_history(
    app: &AppHandle,
    query: String,
    favorites_only: bool,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    let state = app.state::<AppState>();
    let revision = state
        .history_query_revision
        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
        + 1;
    load_page(
        app,
        Some(crate::database::HistoryQuery {
            text: query,
            favorites_only,
        }),
        revision,
    )
    .await
}

pub(crate) async fn load_more(
    app: &AppHandle,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    let state = app.state::<AppState>();
    if state
        .history_loading_more
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(state.history.lock().unwrap().wire_snapshot());
    }
    let revision = state
        .history_query_revision
        .load(std::sync::atomic::Ordering::SeqCst);
    let result = load_page(app, None, revision).await;
    state
        .history_loading_more
        .store(false, std::sync::atomic::Ordering::SeqCst);
    result
}

async fn load_page(
    app: &AppHandle,
    query: Option<crate::database::HistoryQuery>,
    revision: u64,
) -> Result<crate::history::HistoryWireSnapshot, String> {
    let state = app.state::<AppState>();
    let _write = state.history_writes.lock().await;
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let current = || {
        state
            .history_query_revision
            .load(std::sync::atomic::Ordering::SeqCst)
            == revision
    };
    if !current() || (query.is_none() && !state.history.lock().unwrap().has_more()) {
        return Ok(state.history.lock().unwrap().wire_snapshot());
    }
    let reset = query.is_some();
    let request = if let Some(query) = query {
        PageState {
            query,
            cursor: None,
            cache: Default::default(),
        }
    } else {
        let paging = state.history_page.lock().unwrap();
        PageState {
            query: paging.query.clone(),
            cursor: paging.cursor.clone(),
            cache: Default::default(),
        }
    };
    let cached = reset
        .then(|| state.history_page.lock().unwrap().cache.get(&request.query))
        .flatten();
    let reload_count = if reset {
        state
            .history_page
            .lock()
            .unwrap()
            .cache
            .loaded_count(&request.query)
    } else {
        0
    };
    let worker = app.clone();
    let work = request.clone();
    let page = if let Some(page) = cached {
        page
    } else {
        tauri::async_runtime::spawn_blocking(move || {
            let state = worker.state::<AppState>();
            let database = state.database.lock().unwrap();
            let database = database.as_ref().ok_or("storage_unavailable")?;
            if reset {
                database.page_prefix(&work.query, reload_count)
            } else {
                database.page(
                    &work.query,
                    work.cursor.as_ref(),
                    crate::database::PAGE_SIZE,
                )
            }
        })
        .await
        .map_err(|_| "storage_worker")?
        .map_err(|error| storage_failure(app, error))?
    };
    crate::commands::on_main(app, move |app| {
        let state = app.state::<AppState>();
        if state
            .history_query_revision
            .load(std::sync::atomic::Ordering::SeqCst)
            != revision
        {
            return Ok(state.history.lock().unwrap().wire_snapshot());
        }
        let cursor = page.cursor.clone();
        let detail_visible = state.windows.lock().unwrap().translation_visible;
        let (loaded, snapshot) = {
            let mut history = state.history.lock().unwrap();
            history.apply_page(page, reset);
            if detail_visible {
                history.restore_visual_selection();
            }
            (history.loaded_page(cursor.clone()), history.wire_snapshot())
        };
        {
            let mut paging = state.history_page.lock().unwrap();
            paging.cache.insert(request.query.clone(), loaded);
            paging.query = request.query;
            paging.cursor = cursor;
        }
        publish_history_snapshot(&app, &snapshot)?;
        Ok(snapshot)
    })
    .await?
}

#[cfg(test)]
mod event_routing_tests {
    use super::*;

    #[test]
    fn fatal_storage_error_reaches_records_only_host_and_blocks_writes() {
        for error in ["storage_corrupt", "storage_schema", "storage_version"] {
            let stored = std::sync::Mutex::new(None);
            let mut received = Vec::new();
            let result = record_storage_failure(error.into(), &stored, |label, payload| {
                if label == "clipboard" {
                    received.push(payload.to_owned());
                }
            });
            assert_eq!(result, error);
            assert_eq!(stored.lock().unwrap().as_deref(), Some(error));
            assert_eq!(received, vec![error]);
        }
    }

    #[test]
    fn transient_storage_error_does_not_latch_or_broadcast() {
        let stored = std::sync::Mutex::new(None);
        let result = record_storage_failure("storage_busy".into(), &stored, |_, _| {
            panic!("transient error must remain an operation error");
        });
        assert_eq!(result, "storage_busy");
        assert!(stored.lock().unwrap().is_none());
    }
}
