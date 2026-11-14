use crate::{model::HistoryRecord, windows, AppState};
use tauri::{AppHandle, Emitter, Manager};

pub(crate) fn select_native_history(app: &AppHandle, id: Option<&str>) -> Result<(), String> {
    app.state::<AppState>().history.lock().unwrap().select(id)?;
    windows::refresh_history(app)?;
    publish_history(app)
}

pub(crate) fn history_record(app: &AppHandle, id: &str) -> Option<HistoryRecord> {
    app.state::<AppState>().history.lock().unwrap().record(id)
}

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
    let snapshot = app.state::<AppState>().history.lock().unwrap().snapshot();
    for label in ["translation", "history"] {
        if let Some(view) = app.get_webview(label) {
            view.emit("history_changed", &snapshot)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}
