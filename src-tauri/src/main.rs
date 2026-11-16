mod clipboard_detail;
mod clipboard_runtime;
mod clipboard_text;
mod commands;
mod history_runtime;
#[cfg(all(feature = "ui-tests", feature = "portable-ui"))]
mod portable_ui_tests;
use nami::{
    database, history, model,
    shortcuts::{self, KeyBinding},
    translation,
};
mod platform;
mod selection;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod windows;
#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
#[path = "portable_windows.rs"]
mod windows;

use model::WindowState;
use std::io::Write;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, AtomicI64, AtomicU64},
    Mutex,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

mod records_runtime;

pub struct AppState {
    records_dirty: AtomicBool,
    records_navigation: Mutex<Option<String>>,
    clipboard_enabled: AtomicBool,
    clipboard_version: AtomicI64,
    clipboard_revision: AtomicU64,
    clipboard_writes: tokio::sync::Mutex<()>,
    image_text: tokio::sync::Mutex<clipboard_text::ImageTextCache>,
    image_text_operation: AtomicU64,
    clipboard_translation: Mutex<Option<selection::SelectionContext>>,
    history: Mutex<history::HistoryStore>,
    database: Mutex<Option<database::HistoryDatabase>>,
    storage_error: Mutex<Option<String>>,
    history_writes: tokio::sync::Mutex<()>,
    history_page: Mutex<history_runtime::PageState>,
    history_query_revision: AtomicU64,
    history_loading_more: AtomicBool,
    windows: Mutex<WindowState>,
    locale: Mutex<String>,
    shortcut_errors: Mutex<Vec<String>>,
    target: Mutex<Option<platform::Target>>,
    operation: AtomicU64,
    translation_revision: AtomicU64,
    translation_engine: Mutex<String>,
    translation_preferences: Mutex<translation::Preferences>,
    shortcuts_suspended: AtomicBool,
    registered_shortcuts: Mutex<Vec<Shortcut>>,
    selection: Mutex<Option<selection::SelectionContext>>,
    selection_source: Mutex<Option<serde_json::Value>>,
    selection_busy: AtomicBool,
    selection_origin: Mutex<Option<selection::SelectionOrigin>>,
}

impl Default for AppState {
    fn default() -> Self {
        let mut windows = WindowState::default();
        if let Some(width) = platform::history_width() {
            windows.history_width = width;
            windows.history_width_initialized = true;
        }
        Self {
            records_dirty: AtomicBool::new(false),
            records_navigation: Mutex::new(None),
            clipboard_enabled: AtomicBool::new(false),
            clipboard_version: AtomicI64::new(-1),
            clipboard_revision: AtomicU64::new(0),
            clipboard_writes: tokio::sync::Mutex::new(()),
            image_text: tokio::sync::Mutex::new(clipboard_text::ImageTextCache::default()),
            image_text_operation: AtomicU64::new(0),
            clipboard_translation: Mutex::new(None),
            history: Mutex::new(history::HistoryStore::new(model::initial_history_records())),
            database: Mutex::new(None),
            storage_error: Mutex::new(None),
            history_writes: tokio::sync::Mutex::new(()),
            history_page: Mutex::new(history_runtime::PageState::default()),
            history_query_revision: AtomicU64::new(0),
            history_loading_more: AtomicBool::new(false),
            windows: Mutex::new(windows),
            locale: Mutex::new("en".into()),
            shortcut_errors: Mutex::new(Vec::new()),
            target: Mutex::new(None),
            operation: AtomicU64::new(0),
            translation_revision: AtomicU64::new(0),
            translation_engine: Mutex::new("edge".into()),
            translation_preferences: Mutex::new(translation::Preferences::defaults(
                platform::capabilities().primary_modifier,
            )),
            shortcuts_suspended: AtomicBool::new(false),
            registered_shortcuts: Mutex::new(Vec::new()),
            selection: Mutex::new(None),
            selection_source: Mutex::new(None),
            selection_busy: AtomicBool::new(false),
            selection_origin: Mutex::new(None),
        }
    }
}

fn record_lifecycle(app: &tauri::AppHandle, event: &str, code: Option<i32>) {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let directory = app.path().app_log_dir()?;
        std::fs::create_dir_all(&directory)?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(directory.join("lifecycle.jsonl"))?;
        // Keep diagnostics bounded without removing earlier exit evidence.
        if file.metadata()?.len() < 65_536 {
            let record = serde_json::json!({
                "unixSeconds": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
                "pid": std::process::id(),
                "event": event,
                "code": code,
                "appleEvent": platform::current_apple_event(),
            });
            writeln!(file, "{record}")?;
        }
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("Nami lifecycle diagnostic: {error}");
    }
}

fn clipboard_shortcut() -> Shortcut {
    Shortcut::new(Some(Modifiers::CONTROL), Code::Backquote)
}

fn binding_shortcut(binding: &KeyBinding) -> Result<Shortcut, String> {
    let code = Code::from_str(&binding.code).map_err(|_| "shortcut_key".to_string())?;
    let mut modifiers = Modifiers::empty();
    for name in &binding.modifiers {
        modifiers |= match name.as_str() {
            "meta" => Modifiers::SUPER,
            "control" => Modifiers::CONTROL,
            "alt" => Modifiers::ALT,
            "shift" => Modifiers::SHIFT,
            _ => return Err("shortcut_modifier".into()),
        };
    }
    Ok(Shortcut::new(Some(modifiers), code))
}

fn same_shortcut(left: &Shortcut, right: &Shortcut) -> bool {
    left.key == right.key && left.mods == right.mods
}

pub(crate) fn sync_global_shortcuts(app: &tauri::AppHandle) {
    if !platform::capabilities().selection {
        return;
    }
    let state = app.state::<AppState>();
    let suspended = state
        .shortcuts_suspended
        .load(std::sync::atomic::Ordering::SeqCst);
    let selection = state
        .translation_preferences
        .lock()
        .unwrap()
        .shortcuts
        .selection
        .clone();
    let previous = state.registered_shortcuts.lock().unwrap().clone();
    let mut desired = Vec::new();
    if !suspended {
        desired.push(clipboard_shortcut());
        if let Some(binding) = selection {
            if let Ok(shortcut) = binding_shortcut(&binding) {
                if !desired
                    .iter()
                    .any(|current| same_shortcut(current, &shortcut))
                {
                    desired.push(shortcut);
                }
            }
        }
    }
    for shortcut in &previous {
        if !desired
            .iter()
            .any(|current| same_shortcut(current, shortcut))
        {
            let _ = app.global_shortcut().unregister(*shortcut);
        }
    }
    let mut failures = Vec::new();
    for shortcut in &desired {
        if previous
            .iter()
            .any(|current| same_shortcut(current, shortcut))
        {
            continue;
        }
        if app.global_shortcut().register(*shortcut).is_err() {
            failures.push(shortcut.to_string());
        }
    }
    {
        let mut errors = state.shortcut_errors.lock().unwrap();
        errors.retain(|error| {
            !previous
                .iter()
                .any(|shortcut| shortcut.to_string() == *error)
        });
        for failure in failures {
            if !errors.contains(&failure) {
                errors.push(failure);
            }
        }
    }
    *state.registered_shortcuts.lock().unwrap() = desired;
}

fn main() {
    let builder = tauri::Builder::default().manage(AppState::default());
    let builder = if platform::capabilities().selection {
        builder.plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    let key = shortcut.key;
                    let modifiers = shortcut.mods;
                    let pressed = event.state() == ShortcutState::Pressed;
                    let owned = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        let state = owned.state::<AppState>();
                        let selection = state
                            .translation_preferences
                            .lock()
                            .unwrap()
                            .shortcuts
                            .selection
                            .clone()
                            .and_then(|binding| binding_shortcut(&binding).ok());
                        let matches = |candidate: &Shortcut| {
                            candidate.key == key && candidate.mods == modifiers
                        };
                        let result = if matches(&clipboard_shortcut()) {
                            if pressed {
                                windows::show(&owned, "clipboard")
                            } else {
                                Ok(())
                            }
                        } else if selection.as_ref().is_some_and(matches) {
                            if pressed {
                                selection::prepare_selected(&owned);
                                Ok(())
                            } else {
                                selection::show_selected(&owned)
                            }
                        } else {
                            Ok(())
                        };
                        if let Err(code) = result {
                            state.shortcut_errors.lock().unwrap().push(code);
                        }
                    });
                })
                .build(),
        )
    } else {
        builder
    };
    builder
        .invoke_handler(tauri::generate_handler![
            commands::app_snapshot,
            commands::translation_run,
            commands::translation_cancel,
            commands::translation_engine_set,
            commands::translation_preferences_set,
            commands::translation_shortcut_set,
            commands::translation_shortcuts_restore,
            commands::translation_shortcut_capture,
            commands::selection_take,
            commands::window_show,
            commands::window_hide,
            commands::window_pin,
            commands::records_view_set,
            commands::records_details_toggle,
            commands::clipboard_details_select,
            commands::clipboard_note_open,
            commands::records_editor_set,
            commands::records_navigation_resolve,
            commands::history_visibility_set,
            commands::history_toggle,
            commands::history_width_set,
            commands::translation_chrome_update,
            commands::history_sample_select,
            commands::history_snapshot,
            commands::history_query,
            commands::history_load_more,
            commands::history_record_update,
            commands::translation_columns_set,
            commands::translation_sample_copy,
            commands::clipboard_record_use,
            commands::clipboard_query,
            commands::clipboard_record_detail,
            commands::clipboard_record_has_text,
            commands::clipboard_record_process,
            commands::clipboard_translation_take,
            commands::clipboard_record_update,
            commands::clipboard_capture_set,
            commands::clipboard_image,
            commands::clipboard_files,
            commands::locale_set,
            commands::permission_request,
        ])
        .setup(|app| {
            #[cfg(feature = "ui-tests")]
            if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
                let database = std::thread::spawn(|| -> Result<_, String> {
                    let mut database =
                        database::HistoryDatabase::open(std::path::Path::new(":memory:"))?;
                    for record in model::initial_history_records() {
                        database.insert(&record, "edge", || true)?;
                    }
                    Ok(database)
                })
                .join()
                .map_err(|_| std::io::Error::other("storage_worker"))?
                .map_err(std::io::Error::other)?;
                *app.state::<AppState>().database.lock().unwrap() = Some(database);
            }
            #[cfg(all(feature = "ui-tests", feature = "portable-ui"))]
            if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
                portable_ui_tests::start(app.handle()).map_err(std::io::Error::other)?;
                return Ok(());
            }
            #[cfg(all(
                feature = "ui-tests",
                target_os = "macos",
                not(feature = "portable-ui")
            ))]
            if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
                windows::start_ui_regression(app.handle()).map_err(std::io::Error::other)?;
                return Ok(());
            }
            let directory = app.path().app_data_dir()?;
            let loaded = std::thread::spawn(move || -> Result<_, String> {
                std::fs::create_dir_all(&directory).map_err(|_| "storage_unavailable")?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
                        .map_err(|_| "storage_unavailable")?;
                }
                let mut database =
                    database::HistoryDatabase::open(&directory.join("nami.sqlite3"))?;
                let page = database.page(
                    &database::HistoryQuery::default(),
                    None,
                    database::PAGE_SIZE,
                )?;
                let capture = database.clipboard_capture_enabled()?;
                let preferences = translation::Preferences::from_store(
                    |key| database.preference(key).ok().flatten(),
                    platform::capabilities().primary_modifier,
                );
                if database
                    .preference("translation_proxy_url")
                    .ok()
                    .flatten()
                    .as_deref()
                    .is_some_and(translation::proxy_contains_credentials)
                {
                    database.preference_set("translation_proxy_url", "")?;
                    database.preference_set("translation_proxy_enabled", "false")?;
                }
                Ok((database, page, capture, preferences))
            })
            .join()
            .map_err(|_| "storage_worker".to_owned())
            .and_then(|loaded| loaded);
            match loaded {
                Ok((database, page, capture, preferences)) => {
                    *app.state::<AppState>().translation_engine.lock().unwrap() =
                        preferences.engine.clone();
                    *app.state::<AppState>()
                        .translation_preferences
                        .lock()
                        .unwrap() = preferences;
                    app.state::<AppState>().clipboard_enabled.store(
                        capture && platform::capabilities().clipboard_write,
                        std::sync::atomic::Ordering::SeqCst,
                    );
                    *app.state::<AppState>().database.lock().unwrap() = Some(database);
                    {
                        let state = app.state::<AppState>();
                        let mut paging = state.history_page.lock().unwrap();
                        paging.cursor = page.cursor.clone();
                        paging
                            .cache
                            .insert(database::HistoryQuery::default(), page.clone());
                    }
                    app.state::<AppState>()
                        .history
                        .lock()
                        .unwrap()
                        .apply_page(page, true);
                }
                Err(error) => *app.state::<AppState>().storage_error.lock().unwrap() = Some(error),
            }
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let translation =
                MenuItem::with_id(app, "translation", "Translate", true, None::<&str>)?;
            let clipboard = MenuItem::with_id(app, "clipboard", "Clipboard", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Nami", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&translation, &clipboard, &settings, &quit])?;
            let tray = TrayIconBuilder::new()
                .tooltip("Nami")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == "quit" {
                        record_lifecycle(app, "tray_quit", Some(0));
                        app.exit(0)
                    } else if let Err(code) = windows::show(app, id) {
                        app.state::<AppState>()
                            .shortcut_errors
                            .lock()
                            .unwrap()
                            .push(code);
                    }
                });
            #[cfg(target_os = "macos")]
            let tray = tray
                .icon(tauri::include_image!("icons/tray-icon.png"))
                .icon_as_template(true);
            #[cfg(not(target_os = "macos"))]
            let tray = tray
                .title("N")
                .icon(app.default_window_icon().ok_or("missing app icon")?.clone());
            tray.build(app)?;
            if let Err(error) = windows::prewarm_records(app.handle()) {
                eprintln!("Nami records prewarm: {error}");
            }
            sync_global_shortcuts(app.handle());
            clipboard_runtime::start(app.handle());
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Nami")
        .run(|app, event| match &event {
            tauri::RunEvent::Ready => record_lifecycle(app, "ready", None),
            tauri::RunEvent::ExitRequested { code, .. } => {
                record_lifecycle(app, "exit_requested", *code)
            }
            tauri::RunEvent::Exit => record_lifecycle(app, "exit", None),
            _ => {}
        });
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn clipboard_history_uses_control_backquote() {
        let clipboard = clipboard_shortcut();
        assert_eq!(clipboard.key, Code::Backquote);
        assert_eq!(clipboard.mods, Modifiers::CONTROL);
        let selection = binding_shortcut(&KeyBinding::new("KeyT", &["meta"])).unwrap();
        assert_eq!(selection, Shortcut::new(Some(Modifiers::SUPER), Code::KeyT));
    }
}
