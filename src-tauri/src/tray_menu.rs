use crate::{clipboard_preferences, commands::on_main, platform, AppState};
use nami::shortcuts::{KeyBinding, ShortcutAction};
use std::sync::atomic::Ordering;
use std::time::Duration;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    AppHandle, Emitter, Manager,
};

pub(crate) const TRAY_ID: &str = "nami-tray";
pub(crate) const DISK_USAGE_REFRESH_INTERVAL: Duration = Duration::from_secs(5 * 60);

#[derive(Clone)]
pub(crate) struct TrayMenu {
    clipboard: MenuItem<tauri::Wry>,
    translation: MenuItem<tauri::Wry>,
    selection: MenuItem<tauri::Wry>,
    settings: MenuItem<tauri::Wry>,
    recording: Option<MenuItem<tauri::Wry>>,
    disk_usage: MenuItem<tauri::Wry>,
    disk_usage_index: usize,
    quit: MenuItem<tauri::Wry>,
}

pub(crate) fn is_zh_locale(locale: &str) -> bool {
    match locale {
        "zh-CN" => true,
        "en" => false,
        _ => platform::system_prefers_zh(),
    }
}

pub(crate) fn recording_label(enabled: bool, is_zh: bool) -> &'static str {
    match (enabled, is_zh) {
        (true, true) => "暂停监听",
        (false, true) => "恢复监听",
        (true, false) => "Pause Recording",
        (false, false) => "Resume Recording",
    }
}

pub(crate) fn format_storage_size(bytes: u64) -> String {
    if bytes < 1000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    for unit in ["KB", "MB", "GB", "TB"] {
        value /= 1000.0;
        if value < 1000.0 || unit == "TB" {
            return format!("{value:.1} {unit}");
        }
    }
    format!("{bytes} B")
}

pub(crate) fn disk_usage_title(is_zh: bool) -> &'static str {
    if is_zh {
        "磁盘占用"
    } else {
        "Disk Usage"
    }
}

pub(crate) fn disk_usage_value(bytes: Option<u64>) -> String {
    bytes
        .map(format_storage_size)
        .unwrap_or_else(|| "—".to_string())
}

pub(crate) fn disk_usage_label(bytes: Option<u64>, is_zh: bool) -> String {
    format!("{}  {}", disk_usage_title(is_zh), disk_usage_value(bytes))
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(crate) fn tray_menu_min_width(is_zh: bool) -> f64 {
    if is_zh {
        200.0
    } else {
        240.0
    }
}

pub(crate) fn cleanup(app: &AppHandle) {
    #[cfg(target_os = "macos")]
    {
        if let Some(tray) = app.tray_by_id(TRAY_ID) {
            let _ = tray.with_inner_tray_icon(|inner| {
                let ptr = inner
                    .ns_status_item()
                    .map(|status_item| {
                        objc2::rc::Retained::as_ptr(&status_item) as *mut std::ffi::c_void
                    })
                    .unwrap_or(std::ptr::null_mut());
                platform::cleanup_tray_disk_usage_observer(ptr);
            });
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = app;
    }
}

fn sync_disk_usage_item(
    app: &AppHandle,
    disk_usage: &MenuItem<tauri::Wry>,
    item_index: usize,
    bytes: Option<u64>,
    is_zh: bool,
) {
    #[cfg(target_os = "macos")]
    {
        let title = disk_usage_title(is_zh);
        let value = disk_usage_value(bytes);
        let fallback = disk_usage_label(bytes, is_zh);
        let min_width = tray_menu_min_width(is_zh);
        let app_handle = app.clone();
        let disk_usage = disk_usage.clone();
        let apply = move || {
            let _ = disk_usage.set_text(title);
            let applied = app_handle
                .tray_by_id(TRAY_ID)
                .and_then(|tray| {
                    tray.with_inner_tray_icon(move |inner| {
                        inner.ns_status_item().is_some_and(|status_item| {
                            platform::configure_tray_disk_usage(
                                objc2::rc::Retained::as_ptr(&status_item) as *mut std::ffi::c_void,
                                item_index,
                                &value,
                                min_width,
                            )
                        })
                    })
                    .ok()
                })
                .unwrap_or(false);
            if !applied {
                let _ = disk_usage.set_text(fallback);
            }
        };
        if platform::is_main_thread() {
            apply();
        } else {
            let _ = app.run_on_main_thread(apply);
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, item_index);
        let _ = disk_usage.set_text(disk_usage_label(bytes, is_zh));
    }
}

pub(crate) fn binding_accelerator(binding: &KeyBinding) -> Option<String> {
    if binding.code.is_empty() || is_modifier_only_code(&binding.code) {
        return None;
    }
    let mut parts = Vec::with_capacity(binding.modifiers.len() + 1);
    for (name, token) in [
        ("control", "Control"),
        ("alt", "Alt"),
        ("shift", "Shift"),
        ("meta", "Super"),
    ] {
        if binding.modifiers.iter().any(|item| item == name) {
            parts.push(token);
        }
    }
    parts.push(binding.code.as_str());
    let candidate = parts.join("+");
    candidate
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .ok()
        .map(|_| candidate)
}

fn is_modifier_only_code(code: &str) -> bool {
    matches!(
        code,
        "MetaLeft"
            | "MetaRight"
            | "ControlLeft"
            | "ControlRight"
            | "AltLeft"
            | "AltRight"
            | "ShiftLeft"
            | "ShiftRight"
    )
}

fn quit_accelerator() -> &'static str {
    if cfg!(target_os = "macos") {
        "Super+KeyQ"
    } else {
        "Control+KeyQ"
    }
}

struct TraySnapshot {
    is_zh: bool,
    clipboard_acc: Option<String>,
    translation_acc: Option<String>,
    selection_acc: Option<String>,
    selection_enabled: bool,
    settings_acc: Option<String>,
    recording_enabled: bool,
    storage_available: bool,
    disk_bytes: Option<u64>,
}

fn snapshot(state: &AppState) -> TraySnapshot {
    let is_zh = is_zh_locale(&state.locale.lock().unwrap());
    let clipboard_acc = {
        let shortcut = state.clipboard_shortcut.lock().unwrap();
        shortcut
            .enabled
            .then_some(shortcut.binding.as_ref())
            .flatten()
            .and_then(binding_accelerator)
    };
    let (translation_acc, selection_acc, settings_acc) = {
        let prefs = state.translation_preferences.lock().unwrap();
        let enabled_binding = |action: ShortcutAction| -> Option<String> {
            let enabled = match action {
                ShortcutAction::Selection => prefs.selection_enabled,
                _ => !prefs.disabled_shortcuts.contains(&action),
            };
            if !enabled {
                return None;
            }
            prefs.shortcuts.get(action).and_then(binding_accelerator)
        };
        (
            enabled_binding(ShortcutAction::OpenTranslation),
            enabled_binding(ShortcutAction::Selection),
            enabled_binding(ShortcutAction::OpenSettings),
        )
    };
    let recording_enabled = state.clipboard_enabled.load(Ordering::SeqCst);
    let storage_available = state.storage_error.lock().unwrap().is_none();
    let disk_bytes = state
        .tray_storage_snapshot
        .lock()
        .unwrap()
        .map(|usage| usage.bytes);
    TraySnapshot {
        is_zh,
        clipboard_acc,
        translation_acc,
        selection_acc,
        selection_enabled: platform::capabilities().selection,
        settings_acc,
        recording_enabled,
        storage_available,
        disk_bytes,
    }
}

pub(crate) fn build(app: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    let state = app.state::<AppState>();
    let snap = snapshot(&state);
    let (clipboard_text, translation_text, selection_text, settings_text, quit_text) = if snap.is_zh
    {
        ("拷贝记录", "打开翻译", "选字翻译", "偏好设置", "退出 Nami")
    } else {
        (
            "Clipboard History",
            "Open Translation",
            "Autofill Input",
            "Settings",
            "Quit Nami",
        )
    };

    let clipboard = MenuItem::with_id(
        app,
        "clipboard",
        clipboard_text,
        true,
        snap.clipboard_acc.as_deref(),
    )?;
    let translation = MenuItem::with_id(
        app,
        "translation",
        translation_text,
        true,
        snap.translation_acc.as_deref(),
    )?;
    let selection = MenuItem::with_id(
        app,
        "selection",
        selection_text,
        snap.selection_enabled,
        snap.selection_acc.as_deref(),
    )?;
    let sep_primary = PredefinedMenuItem::separator(app)?;
    let settings = MenuItem::with_id(
        app,
        "settings",
        settings_text,
        true,
        snap.settings_acc.as_deref(),
    )?;
    let recording = if platform::capabilities().clipboard_write {
        let item = MenuItem::with_id(
            app,
            "clipboard_recording",
            recording_label(snap.recording_enabled, snap.is_zh),
            snap.storage_available,
            None::<&str>,
        )?;
        *state.clipboard_recording_menu.lock().unwrap() = Some(item.clone());
        Some(item)
    } else {
        None
    };
    let disk_usage = MenuItem::with_id(
        app,
        "disk_usage",
        if cfg!(target_os = "macos") {
            disk_usage_title(snap.is_zh).to_owned()
        } else {
            disk_usage_label(snap.disk_bytes, snap.is_zh)
        },
        true,
        None::<&str>,
    )?;
    let sep_quit = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", quit_text, true, Some(quit_accelerator()))?;

    let menu = Menu::with_items(
        app,
        &[
            &clipboard,
            &translation,
            &selection,
            &sep_primary,
            &settings,
        ],
    )?;
    let disk_usage_index = if recording.is_some() { 6 } else { 5 };
    if let Some(recording_item) = &recording {
        menu.append(recording_item)?;
    }
    menu.append_items(&[&disk_usage, &sep_quit, &quit])?;

    *state.tray_menu.lock().unwrap() = Some(TrayMenu {
        clipboard,
        translation,
        selection,
        settings,
        recording,
        disk_usage,
        disk_usage_index,
        quit,
    });
    Ok(menu)
}

pub(crate) fn sync(app: &AppHandle) {
    let state = app.state::<AppState>();
    let Some(menu) = state.tray_menu.lock().unwrap().clone() else {
        return;
    };
    let snap = snapshot(&state);
    let (clipboard_text, translation_text, selection_text, settings_text, quit_text) = if snap.is_zh
    {
        ("拷贝记录", "打开翻译", "选字翻译", "偏好设置", "退出 Nami")
    } else {
        (
            "Clipboard History",
            "Open Translation",
            "Autofill Input",
            "Settings",
            "Quit Nami",
        )
    };
    let _ = menu.clipboard.set_text(clipboard_text);
    let _ = menu
        .clipboard
        .set_accelerator(snap.clipboard_acc.as_deref());
    let _ = menu.translation.set_text(translation_text);
    let _ = menu
        .translation
        .set_accelerator(snap.translation_acc.as_deref());
    let _ = menu.selection.set_text(selection_text);
    let _ = menu.selection.set_enabled(snap.selection_enabled);
    let _ = menu
        .selection
        .set_accelerator(snap.selection_acc.as_deref());
    let _ = menu.settings.set_text(settings_text);
    let _ = menu.settings.set_accelerator(snap.settings_acc.as_deref());
    if let Some(recording) = &menu.recording {
        let _ = recording.set_text(recording_label(snap.recording_enabled, snap.is_zh));
        let _ = recording.set_enabled(snap.storage_available);
    }
    sync_disk_usage_item(
        app,
        &menu.disk_usage,
        menu.disk_usage_index,
        snap.disk_bytes,
        snap.is_zh,
    );
    let _ = menu.quit.set_text(quit_text);
    let _ = menu.quit.set_accelerator(Some(quit_accelerator()));
}

pub(crate) fn start_disk_usage_loop(app: &AppHandle) {
    if app
        .state::<AppState>()
        .storage_error
        .lock()
        .unwrap()
        .is_some()
    {
        return;
    }
    refresh_disk_usage(app, 0);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(DISK_USAGE_REFRESH_INTERVAL).await;
            let state = app.state::<AppState>();
            let current_rev = state.clipboard_revision.load(Ordering::SeqCst);
            if state.tray_disk_revision.load(Ordering::SeqCst) == current_rev
                && state.tray_storage_snapshot.lock().unwrap().is_some()
            {
                continue;
            }
            refresh_disk_usage(&app, 0);
        }
    });
}

pub(crate) fn refresh_disk_usage(app: &AppHandle, debounce_ms: u64) {
    let state = app.state::<AppState>();
    if state.storage_error.lock().unwrap().is_some() {
        return;
    }
    let generation = state
        .tray_disk_refresh_generation
        .fetch_add(1, Ordering::SeqCst)
        + 1;
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if debounce_ms > 0 {
            tokio::time::sleep(Duration::from_millis(debounce_ms)).await;
            if app
                .state::<AppState>()
                .tray_disk_refresh_generation
                .load(Ordering::SeqCst)
                != generation
            {
                return;
            }
        }
        let revision = app
            .state::<AppState>()
            .clipboard_revision
            .load(Ordering::SeqCst);
        let Ok(usage) = clipboard_preferences::storage_usage_snapshot(&app).await else {
            return;
        };
        let state = app.state::<AppState>();
        if state.tray_disk_refresh_generation.load(Ordering::SeqCst) != generation {
            return;
        }
        *state.tray_storage_snapshot.lock().unwrap() = Some(usage);
        state.tray_disk_revision.store(revision, Ordering::SeqCst);
        if let Some(view) = app.get_webview("settings") {
            let _ = view.emit("records_storage_changed", usage);
        }
        let sync_app = app.clone();
        let _ = on_main(&app, move |_| sync(&sync_app)).await;
    });
}

pub(crate) fn open_storage_settings(app: &AppHandle) -> Result<(), String> {
    let state = app.state::<AppState>();
    *state.pending_settings_page.lock().unwrap() = Some("storage".to_owned());
    crate::windows::show(app, "settings")?;
    if let Some(view) = app.get_webview("settings") {
        let _ = view.emit("settings_page_requested", "storage");
    }
    let current_rev = state.clipboard_revision.load(Ordering::SeqCst);
    if state.tray_disk_revision.load(Ordering::SeqCst) != current_rev
        || state.tray_storage_snapshot.lock().unwrap().is_none()
    {
        refresh_disk_usage(app, 0);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_supported_bindings_to_valid_menu_accelerators() {
        assert_eq!(
            binding_accelerator(&KeyBinding::new("Backquote", &["control"])).as_deref(),
            Some("Control+Backquote")
        );
        assert_eq!(
            binding_accelerator(&KeyBinding::new("KeyT", &["meta"])).as_deref(),
            Some("Super+KeyT")
        );
        assert_eq!(
            binding_accelerator(&KeyBinding::new("Comma", &["meta"])).as_deref(),
            Some("Super+Comma")
        );
        assert_eq!(
            binding_accelerator(&KeyBinding::new("KeyO", &["shift", "meta"])).as_deref(),
            Some("Shift+Super+KeyO")
        );
        assert_eq!(binding_accelerator(&KeyBinding::new("MetaLeft", &[])), None);
        assert!(quit_accelerator()
            .parse::<tauri_plugin_global_shortcut::Shortcut>()
            .is_ok());
    }

    #[test]
    fn formats_storage_sizes_and_localized_labels() {
        assert_eq!(format_storage_size(0), "0 B");
        assert_eq!(format_storage_size(512), "512 B");
        assert_eq!(format_storage_size(1_500), "1.5 KB");
        assert_eq!(format_storage_size(12_400_000), "12.4 MB");
        assert_eq!(format_storage_size(2_500_000_000), "2.5 GB");

        assert_eq!(disk_usage_title(true), "磁盘占用");
        assert_eq!(disk_usage_title(false), "Disk Usage");
        assert_eq!(disk_usage_value(None), "—");
        assert_eq!(disk_usage_value(Some(12_400_000)), "12.4 MB");
        assert_eq!(tray_menu_min_width(true), 200.0);
        assert_eq!(tray_menu_min_width(false), 240.0);
        assert_eq!(disk_usage_label(None, true), "磁盘占用  —");
        assert_eq!(
            disk_usage_label(Some(12_400_000), true),
            "磁盘占用  12.4 MB"
        );
        assert_eq!(
            disk_usage_label(Some(12_400_000), false),
            "Disk Usage  12.4 MB"
        );
        assert_eq!(recording_label(true, true), "暂停监听");
        assert_eq!(recording_label(false, true), "恢复监听");
        assert_eq!(recording_label(true, false), "Pause Recording");
        assert_eq!(recording_label(false, false), "Resume Recording");
    }
}
