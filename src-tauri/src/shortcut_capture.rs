use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSWindow};
use std::{cell::RefCell, ptr::NonNull};
use tauri::{AppHandle, Manager};

thread_local! {
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

// AppKit can consume these command keys before WKWebView emits a keydown.
pub(crate) fn set_active(app: &AppHandle, active: bool) {
    MONITOR.with(|slot| {
        let mut slot = slot.borrow_mut();
        if let Some(previous) = slot.take() {
            unsafe { NSEvent::removeMonitor(&previous) };
        }
        let Some(view) = app.get_webview("settings").filter(|_| active) else {
            return;
        };
        let Some(window) = app.get_window("settings") else {
            return;
        };
        let Ok(native_window) = window.ns_window() else {
            return;
        };
        let window_number = unsafe { (&*native_window.cast::<NSWindow>()).windowNumber() };
        let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
            let key = unsafe { event.as_ref() };
            if key.windowNumber() != window_number || !window.is_focused().unwrap_or(false) {
                return event.as_ptr();
            }
            let flags = key.modifierFlags();
            if !flags.contains(NSEventModifierFlags::Command)
                || flags.intersects(NSEventModifierFlags::Control | NSEventModifierFlags::Option)
            {
                return event.as_ptr();
            }
            let shift = flags.contains(NSEventModifierFlags::Shift);
            let letter = match (key.keyCode(), shift) {
                (0, false) => 'A',
                (6, _) => 'Z',
                (7, false) => 'X',
                (8, false) => 'C',
                (9, false) => 'V',
                (13, false) => 'W',
                _ => return event.as_ptr(),
            };
            if !key.isARepeat() {
                let script = format!("if(document.querySelector('[data-recording]'))document.activeElement?.dispatchEvent(new KeyboardEvent('keydown',{{key:'{}',code:'Key{letter}',metaKey:true,shiftKey:{shift},bubbles:true,cancelable:true}}));", letter.to_ascii_lowercase());
                if view.eval(script).is_err() {
                    return event.as_ptr();
                }
            }
            std::ptr::null_mut()
        });
        *slot = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
        };
    });
}

fn route_attached_close(app: &AppHandle) -> bool {
    let can_route = {
        let state = app.state::<crate::AppState>();
        let regions = state.windows.lock().unwrap();
        regions.clipboard_visible
            && regions.clipboard_translation_visible
            && !regions.records_editor_open
    };
    // Only the right WebView knows whether a dropdown needs to close before the pane.
    can_route && !crate::windows::clipboard_has_focus(app) && app.get_webview("translation").is_some_and(|view| {
        view.eval(r#"{
            const active = document.activeElement;
            const target = active?.closest('.clipboard-right-panes, .language-popover')
                ? active : document.querySelector('.clipboard-translate-pane');
            target?.dispatchEvent(new KeyboardEvent('keydown', { key: 'w', code: 'KeyW', metaKey: true, bubbles: true, cancelable: true }));
        }"#).is_ok()
    })
}

// Borderless records windows do not reliably deliver these command keys to WebKit.
pub(crate) fn install_window_shortcuts(app: &AppHandle) {
    thread_local! {
        static WINDOW_MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
    }
    let app = app.clone();
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        let key = unsafe { event.as_ref() };
        let flags = key.modifierFlags();
        if !flags.contains(NSEventModifierFlags::Command)
            || flags.intersects(
                NSEventModifierFlags::Control
                    | NSEventModifierFlags::Option
                    | NSEventModifierFlags::Shift,
            )
            || !matches!(key.keyCode(), 13 | 43)
        {
            return event.as_ptr();
        }
        let Some(window) = ["translation", "clipboard", "settings"]
            .into_iter()
            .filter_map(|label| app.get_window(label))
            .find(|window| {
                window.ns_window().ok().is_some_and(|native| unsafe {
                    let window = &*native.cast::<NSWindow>();
                    window.isKeyWindow() && window.windowNumber() == key.windowNumber()
                })
            })
        else {
            return event.as_ptr();
        };
        let result = if key.keyCode() == 13 {
            if key.isARepeat() {
                return std::ptr::null_mut();
            }
            if window.label() == "settings" {
                if MONITOR.with(|slot| slot.borrow().is_some()) {
                    return event.as_ptr();
                }
                crate::windows::hide(&app, "settings")
            } else if window.label() == "translation" && route_attached_close(&app) {
                Ok(())
            } else {
                crate::records_runtime::dismiss_records_window(&app, window.label())
            }
        } else {
            if window.label() == "settings" {
                return event.as_ptr();
            }
            let state = app.state::<crate::AppState>();
            let preferences = state.translation_preferences.lock().unwrap();
            let action = nami::shortcuts::ShortcutAction::OpenSettings;
            if preferences.disabled_shortcuts.contains(&action)
                || !preferences
                    .shortcuts
                    .get(action)
                    .is_some_and(|binding| binding.code == "Comma" && binding.modifiers == ["meta"])
            {
                return event.as_ptr();
            }
            drop(preferences);
            if key.isARepeat() {
                return std::ptr::null_mut();
            }
            crate::records_runtime::open_settings_window(&app)
        };
        if let Err(error) = result {
            eprintln!("Nami window shortcut: {error}");
        }
        std::ptr::null_mut()
    });
    WINDOW_MONITOR.with(|slot| {
        *slot.borrow_mut() = unsafe {
            NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler)
        };
    });
}
