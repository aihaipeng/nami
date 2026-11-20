use super::{
    CGEventCreateKeyboardEvent, CGEventPost, CGEventPostToPid, CGEventSetFlags,
    CGEventSourceCreate, Target,
};
use objc2_foundation::{ns_string, NSString};
use std::ffi::c_void;
use std::time::{Duration, Instant};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> *const c_void;
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementSetAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *const c_void,
    ) -> i32;
    fn CGEventSourceSetLocalEventsFilterDuringSuppressionState(
        source: *const c_void,
        mask: u32,
        state: u32,
    );
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetLength(value: *const c_void) -> isize;
    fn CFDataGetBytePtr(value: *const c_void) -> *const u8;
    fn CFBooleanGetTypeID() -> usize;
    fn CFGetTypeID(value: *const c_void) -> usize;
    fn CFStringGetTypeID() -> usize;
    fn CFStringGetLength(value: *const c_void) -> isize;
    fn CFStringGetCharacters(value: *const c_void, range: CFRange, buffer: *mut u16);
    fn CFRelease(value: *const c_void);
    static kCFBooleanTrue: *const c_void;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *const c_void;
    fn TISGetInputSourceProperty(source: *const c_void, key: *const c_void) -> *const c_void;
    fn LMGetKbdType() -> u8;
    fn UCKeyTranslate(
        layout: *const u8,
        key: u16,
        action: u16,
        modifiers: u32,
        keyboard_type: u32,
        options: u32,
        dead_key_state: *mut u32,
        capacity: usize,
        actual: *mut usize,
        characters: *mut u16,
    ) -> i32;
}

#[repr(C)]
struct CFRange {
    location: isize,
    length: isize,
}

struct OwnedCf(*const c_void);

impl Drop for OwnedCf {
    fn drop(&mut self) {
        unsafe { CFRelease(self.0) }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManualAccessibility {
    Enabled,
    AlreadyEnabled,
    Unsupported,
}

unsafe fn ax_attribute(element: *const c_void, name: &NSString) -> Option<OwnedCf> {
    let mut value = std::ptr::null();
    let status =
        AXUIElementCopyAttributeValue(element, (name as *const NSString).cast(), &mut value);
    if status == 0 && !value.is_null() {
        Some(OwnedCf(value))
    } else {
        if !value.is_null() {
            CFRelease(value);
        }
        None
    }
}

unsafe fn ax_string(value: &OwnedCf) -> Option<String> {
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

const AX_MESSAGING_TIMEOUT: f32 = 0.25;

unsafe fn apply_messaging_timeout(element: *const c_void) {
    // A hung target must not hold selection_busy for the system default
    // timeout. Apply the short budget to every element before reading it.
    let _ = AXUIElementSetMessagingTimeout(element, AX_MESSAGING_TIMEOUT);
}

unsafe fn application_element(target: Target) -> Result<OwnedCf, &'static str> {
    let application = AXUIElementCreateApplication(target.pid);
    if application.is_null() {
        return Err("selection_application_unavailable");
    }
    apply_messaging_timeout(application);
    Ok(OwnedCf(application))
}

/// Reads the focused element from the application when the target is known,
/// or from the system-wide element when the foreground application could not
/// be resolved.
unsafe fn focused_element(target: Option<Target>) -> Result<OwnedCf, &'static str> {
    let root = match target {
        Some(target) => application_element(target)?,
        None => {
            let system = AXUIElementCreateSystemWide();
            if system.is_null() {
                return Err("selection_application_unavailable");
            }
            apply_messaging_timeout(system);
            OwnedCf(system)
        }
    };
    let focused = ax_attribute(root.0, ns_string!("AXFocusedUIElement"))
        .ok_or("selection_focus_unavailable")?;
    apply_messaging_timeout(focused.0);
    Ok(focused)
}

/// Reads `AXSelectedText` directly. An unavailable or empty attribute is
/// reported as an empty string; the caller decides whether to wake the
/// application accessibility tree.
pub fn selected_text(target: Option<Target>) -> Result<String, &'static str> {
    unsafe {
        let focused = focused_element(target)?;
        Ok(ax_attribute(focused.0, ns_string!("AXSelectedText"))
            .and_then(|value| ax_string(&value))
            .unwrap_or_default())
    }
}

/// Enables the application-level manual accessibility switch used by
/// Chromium-based applications. Returns `AlreadyEnabled` when the tree was
/// already woken, and `Unsupported` when the application does not expose the
/// attribute or rejects the write.
pub fn enable_selection_accessibility(target: Target) -> Result<ManualAccessibility, &'static str> {
    unsafe {
        let application = application_element(target)?;
        let attribute = ns_string!("AXManualAccessibility");
        if ax_attribute(application.0, attribute).is_some_and(|current| {
            CFGetTypeID(current.0) == CFBooleanGetTypeID() && current.0 == kCFBooleanTrue
        }) {
            return Ok(ManualAccessibility::AlreadyEnabled);
        }
        // A missing attribute is also attempted: some Chromium-family
        // applications expose the setter before the getter.
        if AXUIElementSetAttributeValue(
            application.0,
            (attribute as *const NSString).cast(),
            kCFBooleanTrue,
        ) == 0
        {
            Ok(ManualAccessibility::Enabled)
        } else {
            Ok(ManualAccessibility::Unsupported)
        }
    }
}

/// Polls the application role until the accessibility tree answers or the
/// readiness window elapses.
pub fn wait_for_ax_ready(target: Target) -> bool {
    unsafe {
        let application = match application_element(target) {
            Ok(application) => application,
            Err(_) => return false,
        };
        if AXUIElementSetMessagingTimeout(application.0, AX_MESSAGING_TIMEOUT) != 0 {
            return false;
        }
        let deadline = Instant::now() + Duration::from_millis(250);
        loop {
            if ax_attribute(application.0, ns_string!("AXRole")).is_some() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

unsafe fn copy_keycode(source: *const c_void) -> Option<u16> {
    if source.is_null() {
        return None;
    }
    let data = TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData);
    if data.is_null() || CFDataGetLength(data) <= 0 {
        return None;
    }
    let layout = CFDataGetBytePtr(data);
    let keyboard_type = u32::from(LMGetKbdType());
    (0..128).find(|&key| {
        let mut dead_keys = 0;
        let mut length = 0;
        let mut chars = [0; 4];
        // Use the Command mapping itself (Dvorak–QWERTY changes layout under Command).
        let status = UCKeyTranslate(
            layout,
            key,
            0, // kUCKeyActionDown
            1, // cmdKey >> 8
            keyboard_type,
            1, // kUCKeyTranslateNoDeadKeysMask
            &mut dead_keys,
            chars.len(),
            &mut length,
            chars.as_mut_ptr(),
        );
        status == 0 && length == 1 && matches!(chars[0], 67 | 99)
    })
}

pub(in crate::platform) fn send_copy(target: Option<Target>) -> Result<(), &'static str> {
    unsafe {
        let key_from = |source: *const c_void| {
            if source.is_null() {
                return None;
            }
            let source = OwnedCf(source);
            copy_keycode(source.0)
        };
        let key = key_from(TISCopyCurrentKeyboardLayoutInputSource())
            .or_else(|| key_from(TISCopyCurrentASCIICapableKeyboardLayoutInputSource()))
            .ok_or("copy_key_unavailable")?;
        let source = CGEventSourceCreate(0); // kCGEventSourceStateCombinedSessionState
        if source.is_null() {
            return Err("event_creation");
        }
        let source = OwnedCf(source);
        CGEventSourceSetLocalEventsFilterDuringSuppressionState(source.0, 3, 0);
        let event = |down| {
            let event = CGEventCreateKeyboardEvent(source.0, key, down);
            if event.is_null() {
                return Err("event_creation");
            }
            let event = OwnedCf(event);
            CGEventSetFlags(event.0.cast_mut(), 1 << 20); // kCGEventFlagMaskCommand
            Ok(event)
        };
        let (down, up) = (event(true)?, event(false)?);
        #[cfg(feature = "ui-tests")]
        if std::env::var_os("NAMI_UI_TEST_REPORT").is_some()
            && target.and_then(super::target_bundle_id).as_deref()
                != Some("dev.local.nami.cross-app-fixture")
        {
            return Err("target_unavailable");
        }
        match target {
            Some(target) => {
                CGEventPostToPid(target.pid, down.0);
                CGEventPostToPid(target.pid, up.0);
            }
            None => {
                // kCGHIDEventTap: route the shortcut through the system focus.
                CGEventPost(2, down.0);
                CGEventPost(2, up.0);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::OwnedCf;
    use std::ffi::c_void;

    #[test]
    fn selection_copy_uses_the_command_mapping_of_each_keyboard_layout() {
        use objc2_foundation::{NSDictionary, NSString};
        unsafe extern "C" {
            static kTISPropertyInputSourceID: *const NSString;
            fn TISCreateInputSourceList(filter: *const c_void, installed: bool) -> *const c_void;
            fn CFArrayGetCount(array: *const c_void) -> isize;
            fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        }
        for (id, expected) in [
            ("com.apple.keylayout.US", 8),
            ("com.apple.keylayout.Dvorak", 34),
            ("com.apple.keylayout.DVORAK-QWERTYCMD", 8),
        ] {
            let id = NSString::from_str(id);
            unsafe {
                let filter = NSDictionary::from_slices(&[&*kTISPropertyInputSourceID], &[&*id]);
                let list =
                    TISCreateInputSourceList((&*filter as *const NSDictionary<_, _>).cast(), true);
                assert!(!list.is_null(), "input source lookup failed: {id}");
                let list = OwnedCf(list);
                assert_eq!(CFArrayGetCount(list.0), 1, "layout not found: {id}");
                let source = CFArrayGetValueAtIndex(list.0, 0);
                assert_eq!(super::copy_keycode(source), Some(expected), "{id}");
            }
        }
    }
}
