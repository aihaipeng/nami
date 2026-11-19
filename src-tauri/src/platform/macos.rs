use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{ns_string, NSArray, NSString};
use std::ffi::c_void;

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pid: i32,
    started: f64,
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "UniformTypeIdentifiers", kind = "framework")]
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
    fn AXValueGetTypeID() -> usize;
    fn AXValueGetType(value: *const c_void) -> u32;
    fn AXValueGetValue(value: *const c_void, kind: u32, output: *mut c_void) -> bool;
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
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSourceCreate(state_id: i32) -> *mut c_void;
    fn CGEventPost(tap: u32, event: *const c_void);
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
        Ok(String::new())
    } else {
        Ok(text)
    }
}

unsafe fn ax_range_length(value: &AXValue) -> Option<isize> {
    const CF_RANGE: u32 = 4;
    if CFGetTypeID(value.0) != AXValueGetTypeID() || AXValueGetType(value.0) != CF_RANGE {
        return None;
    }
    let mut range = CFRange {
        location: 0,
        length: 0,
    };
    (AXValueGetValue(value.0, CF_RANGE, (&mut range as *mut CFRange).cast())
        && range.location >= 0
        && range.length >= 0)
        .then_some(range.length)
}

fn resolve_selection(
    selected: Option<String>,
    range_length: Option<isize>,
    read_range: impl FnOnce() -> Option<String>,
) -> Result<String, &'static str> {
    if let Some(text) = selected.as_ref().filter(|text| !text.trim().is_empty()) {
        return Ok(text.clone());
    }
    match range_length {
        Some(0) => Ok(String::new()),
        Some(length) if length > 0 => read_range()
            .ok_or("selection_unavailable")
            .and_then(selection_result),
        None if selected.is_some() => Ok(String::new()),
        _ => Err("selection_unavailable"),
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
            return Err("selection_application_unavailable");
        }
        let application = AXValue(application);
        if AXUIElementSetMessagingTimeout(application.0, 0.25) != 0 {
            return Err("selection_application_timeout");
        }
        // Chromium exposes its accessibility tree lazily when the application role is read.
        let _ = ax_attribute(application.0, ns_string!("AXRole"));
        let focused = ax_attribute(application.0, ns_string!("AXFocusedUIElement"))
            .ok_or("selection_focus_unavailable")?;
        if CFGetTypeID(focused.0) != AXUIElementGetTypeID() {
            return Err("selection_focus_invalid");
        }
        let mut focused_pid = 0;
        if AXUIElementGetPid(focused.0, &mut focused_pid) != 0 || focused_pid != target.pid {
            return Err("target_unavailable");
        }
        if AXUIElementSetMessagingTimeout(focused.0, 0.25) != 0 {
            return Err("selection_focus_timeout");
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
        let selected = ax_attribute(focused.0, ns_string!("AXSelectedText"))
            .and_then(|value| ax_string(&value));
        if selected
            .as_ref()
            .is_some_and(|text| !text.trim().is_empty())
        {
            return Ok(selected.unwrap());
        }
        let range = ax_attribute(focused.0, ns_string!("AXSelectedTextRange"));
        let range_length = range.as_ref().and_then(|value| ax_range_length(value));
        resolve_selection(selected, range_length, || {
            let range = range.as_ref()?;
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
    }
}

pub(super) fn target_bundle_id(target: Target) -> Option<String> {
    unsafe {
        let application = running(target)?;
        let bundle: Option<Retained<NSString>> = msg_send![&*application, bundleIdentifier];
        bundle.map(|id| id.to_string())
    }
}

pub(crate) fn pick_excluded_app(
) -> Result<Option<crate::clipboard_preferences::ExcludedApp>, String> {
    unsafe {
        let panel: Retained<AnyObject> = msg_send![class!(NSOpenPanel), openPanel];
        let applications: Retained<AnyObject> =
            msg_send![class!(NSURL), fileURLWithPath: ns_string!("/Applications")];
        let _: () = msg_send![&*panel, setDirectoryURL: &*applications];
        let _: () = msg_send![&*panel, setCanChooseFiles: true];
        let _: () = msg_send![&*panel, setCanChooseDirectories: false];
        let app_type: Retained<AnyObject> = msg_send![class!(UTType), typeWithIdentifier: ns_string!("com.apple.application-bundle")];
        let types = NSArray::from_retained_slice(&[app_type]);
        let _: () = msg_send![&*panel, setAllowedContentTypes: &*types];
        let _: () = msg_send![&*panel, setAllowsMultipleSelection: false];
        let _: () = msg_send![&*panel, setTreatsFilePackagesAsDirectories: false];
        let _: () = msg_send![&*panel, setMessage: ns_string!("Choose an application to exclude from clipboard recording.")];
        let _: () = msg_send![&*panel, setPrompt: ns_string!("Exclude")];
        let result: isize = msg_send![&*panel, runModal];
        if result != 1 {
            return Ok(None);
        }
        let selected: Option<Retained<AnyObject>> = msg_send![&*panel, URL];
        let selected = selected.ok_or("clipboard_invalid_app")?;
        let extension: Option<Retained<NSString>> = msg_send![&*selected, pathExtension];
        if !extension.is_some_and(|value| value.to_string().eq_ignore_ascii_case("app")) {
            return Err("clipboard_invalid_app".into());
        }
        let bundle: Option<Retained<AnyObject>> =
            msg_send![class!(NSBundle), bundleWithURL: &*selected];
        let bundle = bundle.ok_or("clipboard_invalid_app")?;
        let identifier: Option<Retained<NSString>> = msg_send![&*bundle, bundleIdentifier];
        let bundle_id = identifier.ok_or("clipboard_invalid_app")?.to_string();
        // Select by Bundle ID; retain no app path and launch no selected app.
        let key = ns_string!("CFBundleDisplayName");
        let display_name: Option<Retained<AnyObject>> =
            msg_send![&*bundle, objectForInfoDictionaryKey: key];
        let name = display_name
            .and_then(|value| value.downcast::<NSString>().ok())
            .map(|value| value.to_string())
            .or_else(|| {
                let filename: Option<Retained<NSString>> = msg_send![&*selected, lastPathComponent];
                filename.map(|value| value.to_string().trim_end_matches(".app").to_owned())
            })
            .unwrap_or_else(|| bundle_id.clone());
        Ok(Some(crate::clipboard_preferences::ExcludedApp {
            bundle_id,
            name,
        }))
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
        let icon_data_url = path.and_then(|path| file_icon_data_url(&path.to_string()));
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

// Native UI fixtures never read or replace the user's general pasteboard.
pub(super) unsafe fn runtime_pasteboard() -> *mut AnyObject {
    #[cfg(feature = "ui-tests")]
    {
        msg_send![class!(NSPasteboard), pasteboardWithName: ns_string!("dev.local.nami.ui-tests.runtime")]
    }
    #[cfg(not(feature = "ui-tests"))]
    {
        msg_send![class!(NSPasteboard), generalPasteboard]
    }
}

pub fn clipboard_version() -> i64 {
    unsafe {
        let board = runtime_pasteboard();
        let count: isize = msg_send![board, changeCount];
        count as i64
    }
}

pub fn write_text(text: &str) -> Result<i64, (&'static str, Option<i64>)> {
    unsafe {
        let board = runtime_pasteboard();
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

// Command down, V down, V up, Command up. Flags match a physical Cmd+V.
fn paste_chord() -> [(u16, bool, u64); 4] {
    const COMMAND: u64 = 1 << 20;
    [
        (0x37, true, COMMAND),
        (0x09, true, COMMAND),
        (0x09, false, COMMAND),
        (0x37, false, 0),
    ]
}

pub fn paste_keys_down(target: Target, expected_version: i64) -> Result<(), &'static str> {
    if !accessibility() {
        return Err("permission_required");
    }
    if !target_focused(target) {
        return Err("target_unavailable");
    }
    if clipboard_version() != expected_version {
        return Err("clipboard_changed");
    }
    let chord = paste_chord();
    // Session tap, not the process id: a V event posted only to the pid never
    // becomes the composer paste event. Keyup is a separate call so the main
    // thread can return during the gap.
    post_chord([chord[0], chord[1]])
}

pub fn paste_keys_up() -> Result<(), &'static str> {
    let chord = paste_chord();
    post_chord([chord[2], chord[3]])
}

fn post_chord(steps: [(u16, bool, u64); 2]) -> Result<(), &'static str> {
    unsafe {
        // HID system state, so the chord is a user key rather than a private event.
        let source = CGEventSourceCreate(1);
        let mut events = [std::ptr::null_mut(); 2];
        for (index, (key, down, flags)) in steps.into_iter().enumerate() {
            let event = CGEventCreateKeyboardEvent(source, key, down);
            if event.is_null() {
                release_events(&events);
                if !source.is_null() {
                    CFRelease(source);
                }
                return Err("event_creation");
            }
            CGEventSetFlags(event, flags);
            events[index] = event;
        }
        if !source.is_null() {
            CFRelease(source);
        }
        for event in events {
            CGEventPost(2, event);
        }
        release_events(&events);
    }
    Ok(())
}

unsafe fn release_events(events: &[*mut c_void; 2]) {
    for event in events {
        if !event.is_null() {
            CFRelease(*event);
        }
    }
}

// NSWorkspace file icons may be queried on a worker; no window/view API is touched.
pub fn file_icon_data_url(path: &str) -> Option<String> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let icon: Option<Retained<AnyObject>> =
            msg_send![workspace, iconForFile: &*NSString::from_str(path)];
        icon.and_then(|icon| {
            let tiff: Option<Retained<AnyObject>> = msg_send![&*icon, TIFFRepresentation];
            let bitmap: Option<Retained<AnyObject>> =
                msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff?];
            let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
            let png: Option<Retained<AnyObject>> =
                msg_send![&*bitmap?, representationUsingType: 4usize, properties: properties];
            let encoded: Retained<NSString> =
                msg_send![&*png?, base64EncodedStringWithOptions: 0usize];
            Some(format!("data:image/png;base64,{encoded}"))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::{
        ax_range_length, paste_chord, resolve_selection, selection_result, AXValue, CFRange,
    };

    #[test]
    fn image_paste_chord_matches_a_physical_command_v() {
        let chord = paste_chord();
        assert_eq!(
            chord.map(|(key, down, _)| (key, down)),
            [(0x37, true), (0x09, true), (0x09, false), (0x37, false)]
        );
        assert!(chord.iter().take(3).all(|(_, _, flags)| *flags == 1 << 20));
        assert_eq!(chord[3].2, 0);
        assert!(chord[..2].iter().all(|(_, down, _)| *down));
        assert!(chord[2..].iter().all(|(_, down, _)| !*down));
    }

    #[test]
    fn native_ax_range_distinguishes_empty_invalid_and_non_range_values() {
        unsafe extern "C" {
            fn AXValueCreate(kind: u32, value: *const std::ffi::c_void) -> *const std::ffi::c_void;
        }
        for (location, length, expected) in [
            (0, 0, Some(0)),
            (9, 3, Some(3)),
            (-1, 0, None),
            (0, -1, None),
        ] {
            let range = CFRange { location, length };
            unsafe {
                let value = AXValue(AXValueCreate(4, (&range as *const CFRange).cast()));
                assert!(!value.0.is_null());
                assert_eq!(ax_range_length(&value), expected);
            }
        }
        let text = objc2_foundation::NSString::from_str("Synthetic non-range value");
        unsafe {
            let value = AXValue(objc2::rc::Retained::into_raw(text).cast());
            assert_eq!(ax_range_length(&value), None);
        }
    }

    #[test]
    fn empty_selection_does_not_read_range() {
        for (selected, range) in [
            (Some(String::new()), Some(0)),
            (Some(" \n".into()), None),
            (None, Some(0)),
        ] {
            assert_eq!(
                resolve_selection(selected, range, || panic!(
                    "empty selection must not read range text"
                )),
                Ok(String::new())
            );
        }
    }

    #[test]
    fn empty_selected_attribute_still_uses_a_nonempty_range() {
        assert_eq!(
            resolve_selection(Some(String::new()), Some(12), || Some("Range text".into())),
            Ok("Range text".into())
        );
        assert_eq!(
            resolve_selection(None, Some(12), || Some(" \n".into())),
            Ok(String::new())
        );
    }

    #[test]
    fn unknown_or_failed_range_reports_unavailable() {
        assert_eq!(
            resolve_selection(None, None, || panic!("no valid range")),
            Err("selection_unavailable")
        );
        assert_eq!(
            resolve_selection(None, Some(-1), || panic!("invalid range")),
            Err("selection_unavailable")
        );
        assert_eq!(
            resolve_selection(Some(String::new()), Some(10), || None),
            Err("selection_unavailable")
        );
    }

    #[test]
    fn nonempty_selected_text_does_not_query_range() {
        assert_eq!(
            resolve_selection(Some("  Selected text\n".into()), None, || panic!(
                "direct text must not read range"
            )),
            Ok("  Selected text\n".into())
        );
    }

    #[test]
    fn selected_text_accepts_an_explicit_empty_selection() {
        assert_eq!(selection_result(" \t\n\u{3000}".into()), Ok(String::new()));
        assert_eq!(
            selection_result("  Synthetic selection\n".into()),
            Ok("  Synthetic selection\n".into())
        );
    }
}
