mod backup_runtime;
mod clipboard_detail;
mod clipboard_preferences;
mod clipboard_runtime;
mod clipboard_text;
mod commands;
mod media;
#[cfg(all(feature = "ui-tests", feature = "portable-ui"))]
mod portable_ui_tests;
#[cfg(target_os = "macos")]
mod shortcut_capture;
use nami::{
    clipboard::RecordingTypes,
    database, model,
    shortcuts::{self, KeyBinding},
    translation,
};
mod platform;
mod selection;
mod storage_location;
mod tray_menu;
#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
mod windows;
#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
#[path = "portable_windows.rs"]
mod windows;

use model::{TranslationOutput, TranslationScope, WindowState};
use serde::Serialize;
use std::io::Write;
use std::str::FromStr;
use std::sync::{
    atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
    Mutex,
};
use tauri::{menu::MenuItem, tray::TrayIconBuilder, Emitter, Manager};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

mod records_runtime;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardTranslationContext {
    pub record_id: String,
    pub text: String,
    pub source_app: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Default)]
pub(crate) struct TranslationSlot {
    pub(crate) revision: AtomicU64,
    pub(crate) pending: Mutex<Option<commands::PendingTranslation>>,
    pub(crate) completed: Mutex<Option<TranslationOutput>>,
}

impl TranslationSlot {
    pub(crate) fn revision(&self) -> u64 {
        self.revision.load(Ordering::SeqCst)
    }

    pub(crate) fn bump_revision(&self) -> u64 {
        let next = self.revision.fetch_add(1, Ordering::SeqCst) + 1;
        self.pending.lock().unwrap().take();
        next
    }

    pub(crate) fn clear(&self) {
        self.bump_revision();
        self.completed.lock().unwrap().take();
    }
}

pub struct AppState {
    records_dirty: AtomicBool,
    records_navigation: Mutex<Option<records_runtime::PendingNavigation>>,
    clipboard_enabled: AtomicBool,
    clipboard_options: Mutex<clipboard_preferences::ClipboardOptions>,
    clipboard_recording_menu: Mutex<Option<MenuItem<tauri::Wry>>>,
    tray_menu: Mutex<Option<tray_menu::TrayMenu>>,
    tray_storage_snapshot: Mutex<Option<clipboard_preferences::StorageUsageSnapshot>>,
    tray_disk_revision: AtomicU64,
    tray_disk_refresh_generation: AtomicU64,
    pending_settings_page: Mutex<Option<String>>,
    clipboard_recording_types: Mutex<RecordingTypes>,
    clipboard_shortcut: Mutex<ClipboardShortcut>,
    clipboard_version: AtomicI64,
    clipboard_revision: AtomicU64,
    cache_cleanup_revision: AtomicU64,
    cache_cleanup_dirty: AtomicBool,
    clipboard_dragging: AtomicBool,
    clipboard_writes: tokio::sync::Mutex<()>,
    // The lock admits one Vision job before loading its image, for QR, OCR and indexing.
    vision_work: std::sync::Arc<tokio::sync::Mutex<clipboard_text::ImageTextCache>>,
    image_text_operation: AtomicU64,
    image_index_jobs: Mutex<clipboard_text::IndexJobs>,
    clipboard_translation: Mutex<Option<ClipboardTranslationContext>>,
    database: Mutex<Option<database::HistoryDatabase>>,
    data_dir: Mutex<Option<std::path::PathBuf>>,
    cleanup_schedule_notify: tokio::sync::Notify,
    storage_error: Mutex<Option<String>>,
    windows: Mutex<WindowState>,
    window_position_mode: Mutex<model::WindowPositionMode>,
    pending_window_anchor: Mutex<Option<model::PlacementAnchor>>,
    locale: Mutex<String>,
    shortcut_errors: Mutex<Vec<String>>,
    target: Mutex<Option<platform::Target>>,
    operation: AtomicU64,
    standalone_translation: TranslationSlot,
    clipboard_side_translation: TranslationSlot,
    translation_engine: Mutex<String>,
    translation_preferences: Mutex<translation::Preferences>,
    shortcuts_suspended: AtomicBool,
    clipboard_translate_hotkey: AtomicBool,
    shortcut_writes: tokio::sync::Mutex<()>,
    registered_shortcuts: Mutex<Vec<Shortcut>>,
    selection: Mutex<Option<selection::SelectionContext>>,
    selection_presentation: Mutex<Option<u64>>,
    selection_target: Mutex<Option<platform::Target>>,
    selection_revision: AtomicU64,
    selection_busy: AtomicBool,
    selection_copy_busy: AtomicBool,
    selection_origin: Mutex<Option<selection::SelectionOrigin>>,
    pending_backup_import: Mutex<Option<backup_runtime::PendingBackupImport>>,
}

impl AppState {
    pub(crate) fn translation_slot(&self, scope: TranslationScope) -> &TranslationSlot {
        match scope {
            TranslationScope::Standalone => &self.standalone_translation,
            TranslationScope::Clipboard => &self.clipboard_side_translation,
        }
    }
}

impl Default for AppState {
    fn default() -> Self {
        let mut windows = WindowState::default();
        if let Some(width) = platform::history_width() {
            windows.clipboard_width = width;
            windows.clipboard_width_initialized = true;
        }
        Self {
            records_dirty: AtomicBool::new(false),
            records_navigation: Mutex::new(None),
            clipboard_enabled: AtomicBool::new(false),
            clipboard_options: Mutex::new(clipboard_preferences::ClipboardOptions::default()),
            clipboard_recording_menu: Mutex::new(None),
            tray_menu: Mutex::new(None),
            tray_storage_snapshot: Mutex::new(None),
            tray_disk_revision: AtomicU64::new(u64::MAX),
            tray_disk_refresh_generation: AtomicU64::new(0),
            pending_settings_page: Mutex::new(None),
            clipboard_recording_types: Mutex::new(RecordingTypes::default()),
            clipboard_shortcut: Mutex::new(ClipboardShortcut {
                binding: Some(shortcuts::clipboard_binding()),
                enabled: true,
            }),
            clipboard_version: AtomicI64::new(-1),
            clipboard_revision: AtomicU64::new(0),
            cache_cleanup_revision: AtomicU64::new(u64::MAX),
            cache_cleanup_dirty: AtomicBool::new(true),
            clipboard_dragging: AtomicBool::new(false),
            clipboard_writes: tokio::sync::Mutex::new(()),
            vision_work: Default::default(),
            image_text_operation: AtomicU64::new(0),
            image_index_jobs: Mutex::default(),
            clipboard_translation: Mutex::new(None),
            database: Mutex::new(None),
            data_dir: Mutex::new(None),
            cleanup_schedule_notify: tokio::sync::Notify::new(),
            storage_error: Mutex::new(None),
            windows: Mutex::new(windows),
            window_position_mode: Mutex::new(model::WindowPositionMode::default()),
            pending_window_anchor: Mutex::new(None),
            locale: Mutex::new("en".into()),
            shortcut_errors: Mutex::new(Vec::new()),
            target: Mutex::new(None),
            operation: AtomicU64::new(0),
            standalone_translation: TranslationSlot::default(),
            clipboard_side_translation: TranslationSlot::default(),
            translation_engine: Mutex::new("edge".into()),
            translation_preferences: Mutex::new(translation::Preferences::defaults(
                platform::capabilities().primary_modifier,
            )),
            shortcuts_suspended: AtomicBool::new(false),
            clipboard_translate_hotkey: AtomicBool::new(false),
            shortcut_writes: tokio::sync::Mutex::new(()),
            registered_shortcuts: Mutex::new(Vec::new()),
            selection: Mutex::new(None),
            selection_presentation: Mutex::new(None),
            selection_target: Mutex::new(None),
            selection_revision: AtomicU64::new(0),
            selection_busy: AtomicBool::new(false),
            selection_copy_busy: AtomicBool::new(false),
            selection_origin: Mutex::new(None),
            pending_backup_import: Mutex::new(None),
        }
    }
}

fn record_lifecycle(app: &tauri::AppHandle, event: &str, code: Option<i32>) {
    let result = (|| -> Result<(), Box<dyn std::error::Error>> {
        let directory = app.path().app_log_dir()?;
        std::fs::create_dir_all(&directory)?;
        let path = directory.join("lifecycle.jsonl");
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        // Keep diagnostics bounded while preserving the most recent history.
        if file.metadata()?.len() >= 65_536 {
            let rotated = directory.join("lifecycle.jsonl.1");
            let _ = std::fs::remove_file(&rotated);
            std::fs::rename(&path, &rotated)?;
            file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
        }
        let record = serde_json::json!({
            "unixSeconds": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs(),
            "pid": std::process::id(),
            "event": event,
            "code": code,
            "appleEvent": platform::current_apple_event(),
        });
        writeln!(file, "{record}")?;
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("Nami lifecycle diagnostic: {error}");
    }
}

#[derive(Clone)]
pub(crate) struct ClipboardShortcut {
    pub(crate) binding: Option<KeyBinding>,
    pub(crate) enabled: bool,
}

fn clipboard_shortcut(state: &AppState) -> Option<Shortcut> {
    let shortcut = state.clipboard_shortcut.lock().unwrap();
    if !shortcut.enabled {
        return None;
    }
    shortcut
        .binding
        .as_ref()
        .and_then(|binding| binding_shortcut(binding).ok())
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

pub(crate) fn parse_ui_locale(stored: Option<&str>) -> String {
    match stored {
        Some(locale @ ("system" | "zh-CN" | "en")) => locale.to_owned(),
        _ => "en".to_owned(),
    }
}

pub(crate) fn load_clipboard_binding(
    stored: Option<&str>,
    translation: &shortcuts::TranslationShortcuts,
) -> Option<KeyBinding> {
    let binding = stored
        .and_then(|value| serde_json::from_str::<Option<KeyBinding>>(value).ok())
        .unwrap_or_else(|| Some(shortcuts::clipboard_binding()));
    binding.and_then(|binding| shortcuts::validate_clipboard_binding(binding, translation).ok())
}

fn same_shortcut(left: &Shortcut, right: &Shortcut) -> bool {
    left.key == right.key && left.mods == right.mods
}

pub(crate) fn sync_global_shortcuts(app: &tauri::AppHandle) {
    tray_menu::sync(app);
    if !platform::capabilities().selection {
        return;
    }
    let state = app.state::<AppState>();
    let suspended = state
        .shortcuts_suspended
        .load(std::sync::atomic::Ordering::SeqCst);
    use shortcuts::ShortcutAction::{OpenTranslation, Selection};
    let previous = state.registered_shortcuts.lock().unwrap().clone();
    let mut desired = Vec::new();
    if !suspended {
        desired.extend(clipboard_shortcut(&state));
        let preferences = state.translation_preferences.lock().unwrap();
        for shortcut in [Selection, OpenTranslation]
            .into_iter()
            .filter_map(|action| preferences.global_binding(action))
            .filter_map(|binding| binding_shortcut(binding).ok())
        {
            if !desired
                .iter()
                .any(|current| same_shortcut(current, &shortcut))
            {
                desired.push(shortcut);
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
        if app.global_shortcut().is_registered(*shortcut) {
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
    if let Some(code) = platform::maybe_run_vision_worker() {
        std::process::exit(code);
    }
    let builder = tauri::Builder::default()
        .manage(AppState::default())
        .register_asynchronous_uri_scheme_protocol(
            media::MEDIA_SCHEME,
            media::handle_uri_scheme_request,
        );
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
                        use std::sync::atomic::Ordering::SeqCst;
                        let contextual = cfg!(target_os = "macos")
                            && key == Code::KeyT
                            && modifiers == Modifiers::SUPER;
                        if contextual
                            && (if pressed {
                                state.clipboard_translate_hotkey.load(SeqCst)
                            } else {
                                state.clipboard_translate_hotkey.swap(false, SeqCst)
                            })
                        {
                            return;
                        }
                        if state
                            .shortcuts_suspended
                            .load(std::sync::atomic::Ordering::SeqCst)
                        {
                            return;
                        }
                        use shortcuts::ShortcutAction::{OpenTranslation, Selection};
                        let matches = |candidate: &Shortcut| {
                            candidate.key == key && candidate.mods == modifiers
                        };
                        let (selection, open) = {
                            let prefs = state.translation_preferences.lock().unwrap();
                            let matched = |action| {
                                prefs
                                    .global_binding(action)
                                    .and_then(|binding| binding_shortcut(binding).ok())
                                    .as_ref()
                                    .is_some_and(matches)
                            };
                            (matched(Selection), matched(OpenTranslation))
                        };
                        if contextual
                            && pressed
                            && (selection || open)
                            && state
                                .clipboard_options
                                .lock()
                                .unwrap()
                                .record_shortcuts
                                .uses_translate_launch_key()
                            && windows::clipboard_has_focus(&owned)
                        {
                            // Own the release too, even if translating moves focus to another pane.
                            state.clipboard_translate_hotkey.store(true, SeqCst);
                            if let Some(view) = owned.get_webview("clipboard") {
                                let _ = view.emit("clipboard_translate_requested", ());
                            }
                            return;
                        }
                        let result = if clipboard_shortcut(&state).as_ref().is_some_and(matches) {
                            if pressed {
                                windows::show(&owned, "clipboard")
                            } else {
                                Ok(())
                            }
                        } else if selection {
                            if pressed {
                                selection::prepare_selected(&owned);
                                Ok(())
                            } else {
                                selection::show_selected(&owned, open)
                            }
                        } else if open && pressed {
                            selection::show_blank(&owned)
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
            commands::translation_copy,
            commands::translation_engine_set,
            commands::translation_language_settings_open,
            commands::translation_preferences_set,
            commands::translation_shortcut_set,
            commands::translation_shortcuts_restore,
            commands::translation_shortcut_capture,
            commands::selection_take,
            commands::selection_present,
            commands::window_show,
            commands::window_hide,
            commands::window_pin,
            commands::records_details_toggle,
            commands::clipboard_details_select,
            commands::clipboard_note_open,
            commands::records_editor_set,
            commands::records_navigation_resolve,
            commands::clipboard_width_set,
            commands::clipboard_detail_width_set,
            commands::clipboard_translation_close,
            commands::clipboard_translation_clear_record,
            commands::translation_chrome_update,
            commands::translation_columns_set,
            commands::clipboard_record_use,
            commands::clipboard_query,
            commands::clipboard_record_detail,
            commands::clipboard_image_qr_codes,
            commands::clipboard_open_url,
            commands::clipboard_open_target,
            commands::clipboard_record_has_text,
            commands::clipboard_record_drag,
            commands::clipboard_record_process,
            commands::clipboard_translation_take,
            commands::clipboard_record_update,
            commands::clipboard_recording_type_set,
            clipboard_preferences::clipboard_capture_set,
            clipboard_preferences::clipboard_options_get,
            clipboard_preferences::clipboard_excluded_app_pick,
            clipboard_preferences::clipboard_excluded_app_remove,
            clipboard_preferences::clipboard_ignored_type_add,
            clipboard_preferences::clipboard_ignored_type_update,
            clipboard_preferences::clipboard_ignored_type_remove,
            clipboard_preferences::clipboard_ignore_secrets_set,
            clipboard_preferences::clipboard_ignored_regex_add,
            clipboard_preferences::clipboard_ignored_regex_remove,
            clipboard_preferences::clipboard_behavior_set,
            clipboard_preferences::clipboard_search_set,
            clipboard_preferences::records_open_focus_set,
            clipboard_preferences::records_preview_lines_set,
            clipboard_preferences::clipboard_record_shortcut_set,
            clipboard_preferences::records_storage_info,
            clipboard_preferences::records_cleanup_preview,
            clipboard_preferences::records_cleanup_apply,
            storage_location::storage_location_info,
            storage_location::storage_open_data_dir,
            storage_location::storage_open_log_dir,
            storage_location::storage_data_dir_pick,
            storage_location::storage_data_dir_reset,
            commands::clipboard_shortcut_set,
            commands::clipboard_image,
            commands::clipboard_files,
            commands::locale_set,
            commands::window_position_mode_set,
            commands::launch_at_login_set,
            backup_runtime::backup_export,
            backup_runtime::backup_import_pick,
            backup_runtime::backup_import_unlock,
            backup_runtime::backup_import_apply,
            commands::permission_request,
        ])
        .setup(|app| {
            #[cfg(feature = "ui-tests")]
            if app.config().identifier == "dev.local.nami.ui-tests.denial"
                && !(std::env::var_os("NAMI_UI_TEST_REPORT").is_some()
                    && std::env::var("NAMI_UI_TEST_CASE").as_deref() == Ok("cross-app")
                    && std::env::var("NAMI_UI_TEST_CROSS_APP").as_deref() == Ok("denied-fresh"))
            {
                return Err(std::io::Error::other(
                    "isolated denial host requires the cross-app denial smoke",
                )
                .into());
            }
            #[cfg(target_os = "macos")]
            shortcut_capture::install_window_shortcuts(app.handle());
            #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
            platform::install_frontmost_tracker();
            #[cfg(feature = "ui-tests")]
            if std::env::var_os("NAMI_UI_TEST_REPORT").is_some() {
                let database = std::thread::spawn(|| -> Result<_, String> {
                    database::HistoryDatabase::open(std::path::Path::new(":memory:"))
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
            let default_directory = app.path().app_data_dir()?;
            let directory = storage_location::resolve_startup_data_dir(&default_directory);
            *app.state::<AppState>().data_dir.lock().unwrap() = Some(directory.clone());
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
                let ui_locale = parse_ui_locale(database.preference("ui_locale")?.as_deref());
                let window_position_mode = model::WindowPositionMode::parse(
                    database.preference("window_position_mode")?.as_deref(),
                );
                let capture = database.clipboard_capture_enabled()?;
                let mut preferences = translation::Preferences::from_store(
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
                let recording_types = database
                    .preference("clipboard_recording_types")?
                    .and_then(|value| serde_json::from_str::<RecordingTypes>(&value).ok())
                    .unwrap_or_default();
                let mut clipboard_options =
                    clipboard_preferences::ClipboardOptions::load(&database)?;
                let clipboard_shortcut = ClipboardShortcut {
                    binding: load_clipboard_binding(
                        database.preference("clipboard_shortcut")?.as_deref(),
                        &preferences.shortcuts,
                    ),
                    enabled: database
                        .preference("clipboard_shortcut_enabled")?
                        .as_deref()
                        != Some("false"),
                };
                clipboard_options
                    .record_shortcuts
                    .upgrade_translate_binding(
                        &preferences.shortcuts,
                        clipboard_shortcut.binding.as_ref(),
                    );
                shortcuts::upgrade_legacy_pin_defaults(
                    &mut clipboard_options.record_shortcuts,
                    &mut preferences.shortcuts,
                    clipboard_shortcut.binding.as_ref(),
                );
                Ok((
                    database,
                    ui_locale,
                    window_position_mode,
                    capture,
                    preferences,
                    recording_types,
                    clipboard_shortcut,
                    clipboard_options,
                ))
            })
            .join()
            .map_err(|_| "storage_worker".to_owned())
            .and_then(|loaded| loaded);
            match loaded {
                Ok((
                    database,
                    ui_locale,
                    window_position_mode,
                    capture,
                    preferences,
                    recording_types,
                    clipboard_shortcut,
                    clipboard_options,
                )) => {
                    *app.state::<AppState>().locale.lock().unwrap() = ui_locale;
                    *app.state::<AppState>().window_position_mode.lock().unwrap() =
                        window_position_mode;
                    *app.state::<AppState>().clipboard_options.lock().unwrap() = clipboard_options;
                    *app.state::<AppState>()
                        .clipboard_recording_types
                        .lock()
                        .unwrap() = recording_types;
                    *app.state::<AppState>().clipboard_shortcut.lock().unwrap() =
                        clipboard_shortcut;
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
                    let app = app.handle().clone();
                    tauri::async_runtime::spawn(async move {
                        let state = app.state::<AppState>();
                        let _write = state.clipboard_writes.lock().await;
                        let result = match clipboard_runtime::image_root(&app) {
                            Ok(root) => {
                                clipboard_preferences::retry_image_deletions_locked(&app, root)
                                    .await
                            }
                            Err(error) => Err(error),
                        };
                        match result {
                            Ok(true) => {
                                eprintln!("Nami clipboard maintenance: clipboard_cleanup_failed")
                            }
                            Err(error) => eprintln!("Nami clipboard maintenance: {error}"),
                            Ok(false) => (),
                        }
                    });
                }
                Err(error) => *app.state::<AppState>().storage_error.lock().unwrap() = Some(error),
            }
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);
            let menu = tray_menu::build(app.handle())?;
            let tray = TrayIconBuilder::with_id(tray_menu::TRAY_ID)
                .tooltip("Nami")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    let id = event.id().as_ref();
                    if id == "quit" {
                        record_lifecycle(app, "tray_quit", Some(0));
                        app.exit(0)
                    } else if id == "clipboard_recording" {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) =
                                clipboard_preferences::set_recording(&app, None).await
                            {
                                eprintln!("Nami clipboard recording: {error}");
                            }
                        });
                    } else if id == "translation" {
                        if let Err(code) = selection::show_blank(app) {
                            app.state::<AppState>()
                                .shortcut_errors
                                .lock()
                                .unwrap()
                                .push(code);
                        }
                    } else if id == "selection" {
                        selection::prepare_selected(app);
                        if let Err(code) = selection::show_selected(app, false) {
                            app.state::<AppState>()
                                .shortcut_errors
                                .lock()
                                .unwrap()
                                .push(code);
                        }
                    } else if id == "disk_usage" {
                        if let Err(code) = tray_menu::open_storage_settings(app) {
                            app.state::<AppState>()
                                .shortcut_errors
                                .lock()
                                .unwrap()
                                .push(code);
                        }
                    } else if id == "settings" {
                        *app.state::<AppState>()
                            .pending_settings_page
                            .lock()
                            .unwrap() = None;
                        if let Err(code) = windows::show(app, id) {
                            app.state::<AppState>()
                                .shortcut_errors
                                .lock()
                                .unwrap()
                                .push(code);
                        }
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
            tray_menu::sync(app.handle());
            tray_menu::start_disk_usage_loop(app.handle());
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
            tauri::RunEvent::ExitRequested { code, api, .. } => {
                if code.is_none() {
                    api.prevent_exit();
                } else {
                    record_lifecycle(app, "exit_requested", *code);
                }
            }
            tauri::RunEvent::Exit => {
                tray_menu::cleanup(app);
                platform::shutdown_vision_worker();
                record_lifecycle(app, "exit", None);
            }
            _ => {}
        });
}

#[cfg(test)]
mod shortcut_tests {
    use super::*;

    #[test]
    fn clipboard_shortcut_persists_default_rebind_and_explicit_clear() {
        let directory = std::env::temp_dir().join(format!(
            "nami-clipboard-preferences-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");
        let translations = shortcuts::TranslationShortcuts::defaults("meta");
        let binding = Some(KeyBinding::new("KeyJ", &["control", "shift"]));
        let types = RecordingTypes {
            image: false,
            ..Default::default()
        };
        let mut database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            load_clipboard_binding(
                database
                    .preference("clipboard_shortcut")
                    .unwrap()
                    .as_deref(),
                &translations
            ),
            Some(shortcuts::clipboard_binding())
        );
        database
            .preference_set(
                "clipboard_recording_types",
                &serde_json::to_string(&types).unwrap(),
            )
            .unwrap();
        database
            .preference_set(
                "clipboard_shortcut",
                &serde_json::to_string(&binding).unwrap(),
            )
            .unwrap();
        drop(database);
        let mut database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            load_clipboard_binding(
                database
                    .preference("clipboard_shortcut")
                    .unwrap()
                    .as_deref(),
                &translations
            ),
            binding
        );
        assert_eq!(
            serde_json::from_str::<RecordingTypes>(
                &database
                    .preference("clipboard_recording_types")
                    .unwrap()
                    .unwrap()
            )
            .unwrap(),
            types
        );
        database
            .preference_set("clipboard_shortcut", "null")
            .unwrap();
        drop(database);
        let database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            load_clipboard_binding(
                database
                    .preference("clipboard_shortcut")
                    .unwrap()
                    .as_deref(),
                &translations
            ),
            None
        );
        assert!(!database.clipboard_capture_enabled().unwrap());
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn clipboard_history_uses_control_backquote() {
        let clipboard = binding_shortcut(&shortcuts::clipboard_binding()).unwrap();
        assert_eq!(clipboard.key, Code::Backquote);
        assert_eq!(clipboard.mods, Modifiers::CONTROL);
        let selection = binding_shortcut(&KeyBinding::new("KeyT", &["meta"])).unwrap();
        assert_eq!(selection, Shortcut::new(Some(Modifiers::SUPER), Code::KeyT));
    }

    #[test]
    fn ui_locale_persists_across_reopen_and_defaults_safely() {
        let directory =
            std::env::temp_dir().join(format!("nami-ui-locale-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");

        let mut database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(database.preference("ui_locale").unwrap(), None);
        assert_eq!(parse_ui_locale(None), "en");
        assert_eq!(parse_ui_locale(Some("invalid")), "en");

        database.preference_set("ui_locale", "zh-CN").unwrap();
        drop(database);

        let mut database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            parse_ui_locale(database.preference("ui_locale").unwrap().as_deref()),
            "zh-CN"
        );
        assert!(!database.clipboard_capture_enabled().unwrap());

        database.preference_set("ui_locale", "system").unwrap();
        drop(database);

        let database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            parse_ui_locale(database.preference("ui_locale").unwrap().as_deref()),
            "system"
        );
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn window_position_mode_persists_across_reopen() {
        let directory =
            std::env::temp_dir().join(format!("nami-window-pos-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");

        let mut database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            model::WindowPositionMode::parse(
                database
                    .preference("window_position_mode")
                    .unwrap()
                    .as_deref()
            ),
            model::WindowPositionMode::LastPosition
        );
        database
            .preference_set(
                "window_position_mode",
                model::WindowPositionMode::Cursor.as_str(),
            )
            .unwrap();
        drop(database);

        let database = database::HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            model::WindowPositionMode::parse(
                database
                    .preference("window_position_mode")
                    .unwrap()
                    .as_deref()
            ),
            model::WindowPositionMode::Cursor
        );
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }
}
