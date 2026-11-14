use crate::{model::HistoryRecord, windows, AppState};
use tauri::{AppHandle, Emitter, Manager};

fn storage_failure(app: &AppHandle, error: String) -> String {
    if matches!(
        error.as_str(),
        "storage_corrupt" | "storage_schema" | "storage_version"
    ) {
        *app.state::<AppState>().storage_error.lock().unwrap() = Some(error.clone());
        for label in ["translation", "history", "settings"] {
            if let Some(view) = app.get_webview(label) {
                let _ = view.emit("storage_error", &error);
            }
        }
    }
    error
}

pub(crate) fn select_native_history(app: &AppHandle, id: Option<&str>) -> Result<(), String> {
    app.state::<AppState>().history.lock().unwrap().select(id)?;
    windows::refresh_history(app)?;
    publish_history(app)
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
    windows::refresh_history(app)?;
    publish_history(app)
}

pub(crate) fn publish_history(app: &AppHandle) -> Result<(), String> {
    let views: Vec<_> = ["translation", "history"]
        .into_iter()
        .filter(|label| *label != "translation" || !crate::platform::capabilities().native_history)
        .filter_map(|label| app.get_webview(label))
        .collect();
    if views.is_empty() {
        return Ok(());
    }
    let snapshot = app.state::<AppState>().history.lock().unwrap().snapshot();
    for view in views {
        view.emit("history_changed", &snapshot)
            .map_err(|_| "event_emit")?;
    }
    Ok(())
}

pub(crate) async fn update_record(
    app: &AppHandle,
    id: String,
    action: String,
) -> Result<crate::history::HistorySnapshot, String> {
    let state = app.state::<AppState>();
    let _write = state.history_writes.lock().await;
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let worker = app.clone();
    let worker_id = id.clone();
    let worker_action = action.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        let (previous, next) = state
            .history
            .lock()
            .unwrap()
            .prepare_update(&worker_id, &worker_action)?;
        if let Some(database) = state.database.lock().unwrap().as_mut() {
            database.update(&previous, next.as_ref())?;
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "storage_worker")?
    .map_err(|error| storage_failure(app, error))?;
    crate::commands::on_main(app, move |app| {
        app.state::<AppState>()
            .history
            .lock()
            .unwrap()
            .update(&id, &action)?;
        windows::refresh_history(&app)?;
        publish_history(&app)?;
        Ok(app.state::<AppState>().history.lock().unwrap().snapshot())
    })
    .await?
}

pub(crate) async fn commit_translation(
    app: &AppHandle,
    record: HistoryRecord,
    engine: String,
    revision: u64,
) -> Result<HistoryRecord, String> {
    let state = app.state::<AppState>();
    let _write = state.history_writes.lock().await;
    if let Some(error) = state.storage_error.lock().unwrap().clone() {
        return Err(error);
    }
    let worker = app.clone();
    let saved = record.clone();
    tauri::async_runtime::spawn_blocking(move || {
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
            database.insert(&saved, &engine, current)?;
        }
        Ok::<_, String>(())
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
        let query = state.history_page.lock().unwrap().query.clone();
        state
            .history
            .lock()
            .unwrap()
            .add_completed(record.clone(), current, &query)?;
        windows::refresh_history(&app)?;
        publish_history(&app)?;
        Ok(record)
    })
    .await?
}

#[derive(Clone, Default)]
pub(crate) struct PageState {
    pub query: crate::database::HistoryQuery,
    pub cursor: Option<crate::database::HistoryCursor>,
}

pub(crate) async fn query_history(
    app: &AppHandle,
    query: String,
    favorites_only: bool,
) -> Result<crate::history::HistorySnapshot, String> {
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

pub(crate) async fn load_more(app: &AppHandle) -> Result<crate::history::HistorySnapshot, String> {
    let state = app.state::<AppState>();
    if state
        .history_loading_more
        .swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        return Ok(state.history.lock().unwrap().snapshot());
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
) -> Result<crate::history::HistorySnapshot, String> {
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
        return Ok(state.history.lock().unwrap().snapshot());
    }
    let reset = query.is_some();
    let request = if let Some(query) = query {
        PageState {
            query,
            cursor: None,
        }
    } else {
        state.history_page.lock().unwrap().clone()
    };
    let worker = app.clone();
    let work = request.clone();
    let page = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        let database = state.database.lock().unwrap();
        database.as_ref().ok_or("storage_unavailable")?.page(
            &work.query,
            work.cursor.as_ref(),
            crate::database::PAGE_SIZE,
        )
    })
    .await
    .map_err(|_| "storage_worker")?
    .map_err(|error| storage_failure(app, error))?;
    crate::commands::on_main(app, move |app| {
        let state = app.state::<AppState>();
        if state
            .history_query_revision
            .load(std::sync::atomic::Ordering::SeqCst)
            != revision
        {
            return Ok(state.history.lock().unwrap().snapshot());
        }
        *state.history_page.lock().unwrap() = PageState {
            query: request.query,
            cursor: page.cursor.clone(),
        };
        state.history.lock().unwrap().apply_page(page, reset);
        windows::refresh_history(&app)?;
        publish_history(&app)?;
        let snapshot = state.history.lock().unwrap().snapshot();
        Ok(snapshot)
    })
    .await?
}
