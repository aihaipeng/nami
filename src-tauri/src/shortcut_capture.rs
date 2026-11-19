use block2::RcBlock;
use objc2::{rc::Retained, runtime::AnyObject};
use objc2_app_kit::{NSEvent, NSEventMask, NSEventModifierFlags, NSWindow};
use std::{cell::RefCell, ptr::NonNull};
use tauri::{AppHandle, Manager};

thread_local! {
    static MONITOR: RefCell<Option<Retained<AnyObject>>> = const { RefCell::new(None) };
}

// AppKit handles these menu equivalents before WKWebView can emit a keydown.
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
