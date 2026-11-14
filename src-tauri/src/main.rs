mod commands;
mod history_runtime;
#[cfg(all(feature = "ui-tests", feature = "portable-ui"))]
mod portable_ui_tests;
use nami::{history, model, translation};
mod platform;
mod selection;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod windows;
#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
#[path = "portable_windows.rs"]
mod windows;

use model::WindowState;
use std::io::Write;
use std::sync::{
    atomic::{AtomicBool, AtomicU64},
    Mutex,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

pub struct AppState {
    history: Mutex<history::HistoryStore>,
    windows: Mutex<WindowState>,
    locale: Mutex<String>,
    shortcut_errors: Mutex<Vec<String>>,
    target: Mutex<Option<platform::Target>>,
    operation: AtomicU64,
    translation_revision: AtomicU64,
    translation_engine: Mutex<String>,
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
            history: Mutex::new(history::HistoryStore::new(model::initial_history_records())),
            windows: Mutex::new(windows),
            locale: Mutex::new("en".into()),
            shortcut_errors: Mutex::new(Vec::new()),
            target: Mutex::new(None),
            operation: AtomicU64::new(0),
            translation_revision: AtomicU64::new(0),
            translation_engine: Mutex::new("edge".into()),
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

fn main() {
    let builder = tauri::Builder::default().manage(AppState::default());
    let builder = if platform::capabilities().selection {
        builder.plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if shortcut.key == Code::KeyT || event.state() == ShortcutState::Pressed {
                        let owned = app.clone();
                        let is_translation = shortcut.key == Code::KeyT;
                        let _ = app.run_on_main_thread(move || {
                            let result = if is_translation {
                                if event.state() == ShortcutState::Pressed {
                                    selection::prepare_selected(&owned);
                                    Ok(())
                                } else {
                                    selection::show_selected(&owned)
                                }
                            } else {
                                windows::show(&owned, "clipboard")
                            };
                            if let Err(code) = result {
                                owned
                                    .state::<AppState>()
                                    .shortcut_errors
                                    .lock()
                                    .unwrap()
                                    .push(code);
                            }
                        });
                    }
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
            commands::selection_take,
            commands::window_show,
            commands::window_hide,
            commands::window_pin,
            commands::history_visibility_set,
            commands::history_toggle,
            commands::history_width_set,
            commands::translation_chrome_update,
            commands::history_sample_select,
            commands::history_snapshot,
            commands::history_record_update,
            commands::translation_columns_set,
            commands::translation_sample_copy,
            commands::clipboard_record_use,
            commands::locale_set,
            commands::permission_request,
        ])
        .setup(|app| {
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
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let translation =
                MenuItem::with_id(app, "translation", "Translate", true, None::<&str>)?;
            let clipboard = MenuItem::with_id(app, "clipboard", "Clipboard", true, None::<&str>)?;
            let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit Nami", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&translation, &clipboard, &settings, &quit])?;
            let tray = TrayIconBuilder::new()
                .title("N")
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
            #[cfg(not(target_os = "macos"))]
            let tray = tray.icon(app.default_window_icon().ok_or("missing app icon")?.clone());
            tray.build(app)?;
            if platform::capabilities().selection {
                for shortcut in [
                    Shortcut::new(Some(Modifiers::SUPER), Code::KeyT),
                    Shortcut::new(Some(Modifiers::CONTROL | Modifiers::ALT), Code::KeyV),
                ] {
                    if app.global_shortcut().register(shortcut).is_err() {
                        app.state::<AppState>()
                            .shortcut_errors
                            .lock()
                            .unwrap()
                            .push(shortcut.to_string());
                    }
                }
            }
            windows::show(app.handle(), "translation").map_err(std::io::Error::other)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build Nami")
        .run(|app, event| {
            match &event {
                tauri::RunEvent::Ready => record_lifecycle(app, "ready", None),
                tauri::RunEvent::ExitRequested { code, .. } => {
                    record_lifecycle(app, "exit_requested", *code)
                }
                tauri::RunEvent::Exit => record_lifecycle(app, "exit", None),
                _ => {}
            }
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Err(code) = windows::show(app, "translation") {
                    app.state::<AppState>()
                        .shortcut_errors
                        .lock()
                        .unwrap()
                        .push(code);
                }
            }
        });
}
