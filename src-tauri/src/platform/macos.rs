use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{ns_string, NSString};
use std::ffi::c_void;

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pid: i32,
    started: f64,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementGetTypeID() -> usize;
    fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        parameter: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const c_void;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSourceCreate(state: i32) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventPostToPid(pid: i32, event: *const c_void);
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    static kCFBooleanTrue: *const c_void;
    fn CFDictionaryCreate(
        allocator: *const c_void,
        keys: *const *const c_void,
        values: *const *const c_void,
        count: isize,
        key_callbacks: *const c_void,
        value_callbacks: *const c_void,
    ) -> *const c_void;
    fn CFRelease(value: *const c_void);
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: *const c_void) -> isize;
    fn CFStringGetCharacters(value: *const c_void, range: CFRange, buffer: *mut u16);
}

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

struct AXValue(*const c_void);

impl Drop for AXValue {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

unsafe fn ax_attribute(element: *const c_void, name: &NSString) -> Option<AXValue> {
    let mut value = std::ptr::null();
    let status =
        AXUIElementCopyAttributeValue(element, (name as *const NSString).cast(), &mut value);
    if status == 0 && !value.is_null() {
        Some(AXValue(value))
    } else {
        if !value.is_null() {
            CFRelease(value);
        }
        None
    }
}

unsafe fn ax_string(value: &AXValue) -> Option<String> {
    if CFGetTypeID(value.0) != CFStringGetTypeID() {
        return None;
    }
    let length = CFStringGetLength(value.0);
    let mut buffer = vec![0u16; length as usize];
    CFStringGetCharacters(
        value.0,
        CFRange {
            location: 0,
            length,
        },
        buffer.as_mut_ptr(),
    );
    String::from_utf16(&buffer).ok()
}

fn selection_result(text: String) -> Result<String, &'static str> {
    if text.trim().is_empty() {
        Err("selection_empty")
    } else {
        Ok(text)
    }
}

// Every AppKit call in this module runs on Tauri's main thread.
pub fn current_apple_event() -> Option<serde_json::Value> {
    unsafe {
        let manager: *mut AnyObject =
            msg_send![class!(NSAppleEventManager), sharedAppleEventManager];
        let event: Option<Retained<AnyObject>> = msg_send![manager, currentAppleEvent];
        let event = event?;
        let event_class: u32 = msg_send![&*event, eventClass];
        let event_id: u32 = msg_send![&*event, eventID];
        let sender: Option<Retained<AnyObject>> =
            msg_send![&*event, attributeDescriptorForKeyword: u32::from_be_bytes(*b"spid")];
        let sender_pid: Option<i32> = sender.map(|sender| msg_send![&*sender, int32Value]);
        let sender_bundle = sender_pid.and_then(|pid| {
            let application: Option<Retained<AnyObject>> = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
            let bundle: Option<Retained<NSString>> = msg_send![&*application?, bundleIdentifier];
            bundle.map(|bundle| bundle.to_string())
        });
        Some(serde_json::json!({
            "class": event_class,
            "id": event_id,
            "senderPid": sender_pid,
            "senderBundle": sender_bundle,
        }))
    }
}

#[cfg(not(feature = "portable-ui"))]
pub fn reduce_motion() -> bool {
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        msg_send![workspace, accessibilityDisplayShouldReduceMotion]
    }
}

pub fn history_width() -> Option<f64> {
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let width: f64 =
            msg_send![defaults, doubleForKey: ns_string!("translation.window.historyWidth")];
        (width.is_finite() && (280.0..=720.0).contains(&width)).then_some(width)
    }
}

pub fn save_history_width(width: f64) {
    unsafe {
        let defaults: *mut AnyObject = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let _: () = msg_send![defaults, setDouble: width, forKey: ns_string!("translation.window.historyWidth")];
    }
}

unsafe fn frontmost() -> Option<Retained<AnyObject>> {
    let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
    msg_send![workspace, frontmostApplication]
}

unsafe fn identity(application: &AnyObject) -> Option<Target> {
    let pid: i32 = msg_send![application, processIdentifier];
    let date: Option<Retained<AnyObject>> = msg_send![application, launchDate];
    let started: f64 = msg_send![date.as_ref()?, timeIntervalSince1970];
    Some(Target { pid, started })
}

pub fn capture_target() -> Option<Target> {
    unsafe {
        let application = frontmost()?;
        let target = identity(&application)?;
        (target.pid != std::process::id() as i32).then_some(target)
    }
}

unsafe fn running(target: Target) -> Option<Retained<AnyObject>> {
    let application: Option<Retained<AnyObject>> = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: target.pid];
    let application = application?;
    let terminated: bool = msg_send![&*application, isTerminated];
    let actual = identity(&application)?;
    (!terminated && actual.started == target.started).then_some(application)
}

pub fn activate(target: Target) -> bool {
    unsafe {
        let Some(application) = running(target) else {
            return false;
        };
        msg_send![&*application, activateWithOptions: 2usize]
    }
}

pub fn target_alive(target: Target) -> bool {
    unsafe { running(target).is_some() }
}

pub fn target_focused(target: Target) -> bool {
    unsafe {
        let Some(actual) = frontmost().and_then(|app| identity(&app)) else {
            return false;
        };
        actual.pid == target.pid && actual.started == target.started && target_alive(target)
    }
}

pub fn accessibility() -> bool {
    unsafe { AXIsProcessTrusted() }
}

// AX reads may block; callers capture and revalidate the foreground target on the main thread.
pub fn selected_text(target: Target) -> Result<String, &'static str> {
    if !accessibility() {
        return Err("permission_required");
    }
    if secure_input() {
        return Err("selection_secure");
    }
    unsafe {
        let application = AXUIElementCreateApplication(target.pid);
        if application.is_null() {
            return Err("selection_unavailable");
        }
        let application = AXValue(application);
        if AXUIElementSetMessagingTimeout(application.0, 0.25) != 0 {
            return Err("selection_unavailable");
        }
        let focused = ax_attribute(application.0, ns_string!("AXFocusedUIElement"))
            .ok_or("selection_unavailable")?;
        if CFGetTypeID(focused.0) != AXUIElementGetTypeID() {
            return Err("selection_unavailable");
        }
        let mut focused_pid = 0;
        if AXUIElementGetPid(focused.0, &mut focused_pid) != 0 || focused_pid != target.pid {
            return Err("target_unavailable");
        }
        if AXUIElementSetMessagingTimeout(focused.0, 0.25) != 0 {
            return Err("selection_unavailable");
        }
        let subrole =
            ax_attribute(focused.0, ns_string!("AXSubrole")).and_then(|value| ax_string(&value));
        if subrole.as_deref() == Some("AXSecureTextField") {
            return Err("selection_secure");
        }
        let role =
            ax_attribute(focused.0, ns_string!("AXRole")).and_then(|value| ax_string(&value));
        if role.as_deref() == Some("AXImage") {
            return Err("selection_non_text");
        }
        let text = ax_attribute(focused.0, ns_string!("AXSelectedText"))
            .and_then(|value| ax_string(&value))
            .filter(|text| !text.trim().is_empty())
            .or_else(|| {
                let range = ax_attribute(focused.0, ns_string!("AXSelectedTextRange"))?;
                let mut value = std::ptr::null();
                let status = AXUIElementCopyParameterizedAttributeValue(
                    focused.0,
                    (ns_string!("AXStringForRange") as *const NSString).cast(),
                    range.0,
                    &mut value,
                );
                if value.is_null() {
                    return None;
                }
                let value = AXValue(value);
                (status == 0).then(|| ax_string(&value)).flatten()
            })
            .ok_or("selection_empty")?;
        selection_result(text)
    }
}

pub fn target_app_info(target: Target) -> Option<serde_json::Value> {
    unsafe {
        let application = running(target)?;
        let bundle: Option<Retained<NSString>> = msg_send![&*application, bundleIdentifier];
        let bundle = bundle?.to_string();
        let name: Option<Retained<NSString>> = msg_send![&*application, localizedName];
        let name = name
            .map(|name| name.to_string())
            .unwrap_or_else(|| bundle.clone());
        let url: Option<Retained<AnyObject>> = msg_send![&*application, bundleURL];
        let path: Option<Retained<NSString>> = url.and_then(|url| msg_send![&*url, path]);
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let icon: Option<Retained<AnyObject>> =
            path.and_then(|path| msg_send![workspace, iconForFile: &*path]);
        let icon_data_url = icon.and_then(|icon| {
            let tiff: Option<Retained<AnyObject>> = msg_send![&*icon, TIFFRepresentation];
            let bitmap: Option<Retained<AnyObject>> =
                msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff?];
            let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let png: Option<Retained<AnyObject>> =
                msg_send![&*bitmap?, representationUsingType: 4usize, properties: properties];
            let encoded: Retained<NSString> =
                msg_send![&*png?, base64EncodedStringWithOptions: 0usize];
            Some(format!("data:image/png;base64,{encoded}"))
        });
        Some(serde_json::json!({
            "bundleId": bundle,
            "name": name,
            "iconDataUrl": icon_data_url,
        }))
    }
}

pub fn secure_input() -> bool {
    unsafe { IsSecureEventInputEnabled() }
}

pub fn send_copy(target: Target) -> Result<(), &'static str> {
    if !accessibility() {
        return Err("permission_required");
    }
    if secure_input() {
        return Err("selection_secure");
    }
    if !target_focused(target) {
        return Err("target_unavailable");
    }
    unsafe {
        let source = CGEventSourceCreate(-1);
        if source.is_null() {
            return Err("event_creation");
        }
        let keys = [
            (55, true, 1 << 20),
            (8, true, 1 << 20),
            (8, false, 1 << 20),
            (55, false, 0),
        ];
        let events: Vec<_> = keys
            .iter()
            .map(|(key, down, _)| CGEventCreateKeyboardEvent(source, *key, *down))
            .collect();
        CFRelease(source);
        if events.iter().any(|event| event.is_null()) {
            for event in events.into_iter().filter(|event| !event.is_null()) {
                CFRelease(event);
            }
            return Err("event_creation");
        }
        for (event, (_, _, flags)) in events.iter().zip(keys) {
            CGEventSetFlags(*event, flags);
            CGEventPostToPid(target.pid, *event);
            CFRelease(*event);
        }
    }
    Ok(())
}

pub fn request_accessibility() -> bool {
    unsafe {
        let key = kAXTrustedCheckOptionPrompt;
        let value = kCFBooleanTrue;
        let options = CFDictionaryCreate(
            std::ptr::null(),
            &key,
            &value,
            1,
            std::ptr::null(),
            std::ptr::null(),
        );
        if options.is_null() {
            return false;
        }
        let trusted = AXIsProcessTrustedWithOptions(options);
        CFRelease(options);
        trusted
    }
}

pub fn clipboard_version() -> i64 {
    unsafe {
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let count: isize = msg_send![board, changeCount];
        count as i64
    }
}

pub fn write_text(text: &str) -> Result<i64, (&'static str, Option<i64>)> {
    unsafe {
        let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
        let text = NSString::from_str(text);
        let owned_version: isize = msg_send![board, clearContents];
        let success: bool =
            msg_send![board, setString: &*text, forType: ns_string!("public.utf8-plain-text")];
        if !success {
            Err(("clipboard_write", None))
        } else if clipboard_version() != owned_version as i64 {
            Err(("clipboard_changed", Some(owned_version as i64)))
        } else {
            Ok(owned_version as i64)
        }
    }
}

pub fn send_paste(target: Target, expected_version: i64) -> Result<(), &'static str> {
    if !accessibility() {
        return Err("permission_required");
    }
    if !target_focused(target) {
        return Err("target_unavailable");
    }
    unsafe {
        let down = CGEventCreateKeyboardEvent(std::ptr::null(), 9, true);
        let up = CGEventCreateKeyboardEvent(std::ptr::null(), 9, false);
        if down.is_null() || up.is_null() {
            if !down.is_null() {
                CFRelease(down)
            }
            if !up.is_null() {
                CFRelease(up)
            }
            return Err("event_creation");
        }
        CGEventSetFlags(down, 1 << 20);
        CGEventSetFlags(up, 1 << 20);
        if clipboard_version() != expected_version {
            CFRelease(down);
            CFRelease(up);
            return Err("clipboard_changed");
        }
        CGEventPostToPid(target.pid, down);
        CGEventPostToPid(target.pid, up);
        CFRelease(down);
        CFRelease(up);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::selection_result;

    #[test]
    fn selected_text_rejects_blank_content_and_preserves_nonblank_content() {
        assert_eq!(
            selection_result(" \t\n\u{3000}".into()),
            Err("selection_empty")
        );
        assert_eq!(
            selection_result("  Synthetic selection\n".into()),
            Ok("  Synthetic selection\n".into())
        );
    }
}
