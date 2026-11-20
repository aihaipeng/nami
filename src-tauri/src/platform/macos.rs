use block2::RcBlock;
use objc2::{
    class, msg_send,
    rc::{Allocated, Retained},
    runtime::AnyObject,
};
use objc2_foundation::{ns_string, NSArray, NSData, NSPoint, NSRect, NSSize, NSString, NSURL};
use std::ffi::c_void;
use std::hash::{Hash, Hasher};
use std::ptr::NonNull;
use std::sync::{Mutex, Once, OnceLock};

mod selection;
pub(super) use selection::send_copy;
pub use selection::{
    enable_selection_accessibility, selected_text, wait_for_ax_ready, ManualAccessibility,
};

/// Last non-Nami application that received focus. Used when Nami is frontmost
/// (for example while a settings window is open) so a shortcut still targets
/// the application the user was working in before.
static LAST_FOREIGN_FRONTMOST: OnceLock<Mutex<Option<Target>>> = OnceLock::new();
static FRONTMOST_TRACKER: Once = Once::new();

fn last_foreign_frontmost() -> Option<Target> {
    (*LAST_FOREIGN_FRONTMOST
        .get_or_init(|| Mutex::new(None))
        .lock()
        .ok()?)
    .filter(|target| unsafe { running(*target).is_some() })
}

fn remember_foreign_frontmost(target: Target) {
    if let Ok(mut last) = LAST_FOREIGN_FRONTMOST
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *last = Some(target);
    }
}

/// Tracks application activation independently of any shortcut press. Current
/// workflows rely on the frontmost application, while the previous owner is
/// kept for the case where Nami itself has just become active.
pub fn install_frontmost_tracker() {
    FRONTMOST_TRACKER.call_once(|| unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        if workspace.is_null() {
            return;
        }
        let center: *mut AnyObject = msg_send![workspace, notificationCenter];
        if center.is_null() {
            return;
        }
        let handler = RcBlock::new(|notification: NonNull<AnyObject>| {
            let notification = notification.as_ref();
            let info: *mut AnyObject = msg_send![notification, userInfo];
            if info.is_null() {
                return;
            }
            let app: *mut AnyObject =
                msg_send![info, objectForKey: ns_string!("NSWorkspaceApplicationKey")];
            if app.is_null() {
                return;
            }
            if let Some(target) = identity(&*app) {
                if target.pid != std::process::id() as i32 {
                    remember_foreign_frontmost(target);
                }
            }
        });
        let token: *mut AnyObject = msg_send![
            center,
            addObserverForName: ns_string!("NSWorkspaceDidActivateApplicationNotification"),
            object: std::ptr::null::<AnyObject>(),
            queue: std::ptr::null::<AnyObject>(),
            usingBlock: &*handler
        ];
        if !token.is_null() {
            // The tracker lives for the whole process; keeping the token
            // retained avoids relying on autorelease lifetime.
            std::mem::forget(Retained::retain(token));
        }
        // Seed the previous owner in case an external application was already
        // frontmost when the tracker was installed.
        if let Some(target) = workspace_frontmost_target() {
            remember_foreign_frontmost(target);
        }
    });
}

#[derive(Clone, Copy, Debug)]
pub struct Target {
    pid: i32,
    started: f64,
    bundle: u64,
}

#[cfg(all(feature = "ui-tests", not(feature = "portable-ui")))]
impl Target {
    pub(crate) fn test_identity() -> Self {
        Self {
            pid: -1,
            started: 0.0,
            bundle: 0,
        }
    }

    pub(crate) fn pid(&self) -> i32 {
        self.pid
    }
}

fn same_target(actual: Target, expected: Target) -> bool {
    actual.pid == expected.pid
        && (expected.started == 0.0 || actual.started == 0.0 || actual.started == expected.started)
        && (expected.bundle == 0 || actual.bundle == 0 || actual.bundle == expected.bundle)
}

#[cfg(all(feature = "ui-tests", not(feature = "portable-ui")))]
pub fn test_self_target() -> Option<Target> {
    unsafe {
        let application: Option<Retained<AnyObject>> = msg_send![
            class!(NSRunningApplication),
            runningApplicationWithProcessIdentifier: std::process::id() as i32
        ];
        identity(application.as_deref()?)
    }
}

#[cfg(all(feature = "ui-tests", not(feature = "portable-ui")))]
pub fn test_set_last_foreign(target: Option<Target>) {
    if let Ok(mut last) = LAST_FOREIGN_FRONTMOST
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *last = target;
    }
}

#[cfg(all(feature = "ui-tests", not(feature = "portable-ui")))]
pub fn test_last_foreign() -> Option<Target> {
    last_foreign_frontmost()
}

#[link(name = "AppKit", kind = "framework")]
unsafe extern "C" {}

#[link(name = "UniformTypeIdentifiers", kind = "framework")]
unsafe extern "C" {}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
    static kAXTrustedCheckOptionPrompt: *const c_void;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> *mut c_void;
    fn CGEventSetFlags(event: *mut c_void, flags: u64);
    fn CGEventSourceCreate(state_id: i32) -> *mut c_void;
    fn CGEventPost(tap: u32, event: *const c_void);
    fn CGEventPostToPid(pid: i32, event: *const c_void);
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementGetPid(element: *const c_void, pid: *mut i32) -> i32;
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
    if pid <= 0 {
        return None;
    }
    // launchDate is unavailable for some system-managed applications on
    // current macOS versions; a missing value must not discard the target.
    let date: Option<Retained<AnyObject>> = msg_send![application, launchDate];
    let started = date
        .as_ref()
        .map_or(0.0, |date| msg_send![date, timeIntervalSince1970]);
    let bundle: Option<Retained<NSString>> = msg_send![application, bundleIdentifier];
    let bundle_url: Option<Retained<NSURL>> = msg_send![application, bundleURL];
    let executable_url: Option<Retained<NSURL>> = msg_send![application, executableURL];
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    if let Some(bundle) = bundle {
        bundle.to_string().hash(&mut hasher);
    }
    if let Some(url) = bundle_url.and_then(|url| url.path()) {
        url.to_string().hash(&mut hasher);
    }
    if let Some(url) = executable_url.and_then(|url| url.path()) {
        url.to_string().hash(&mut hasher);
    }
    Some(Target {
        pid,
        started,
        bundle: hasher.finish(),
    })
}

fn choose_clipboard_target(current: Option<Target>) -> Option<Target> {
    current
}

fn choose_selection_target(
    current: Option<Target>,
    previous: Option<Target>,
    ax: impl FnOnce() -> Option<Target>,
) -> Option<Target> {
    current.or(previous).or_else(ax)
}

/// Resolves the current external frontmost application for clipboard capture
/// and paste targeting. Never falls back to a previously active application:
/// pasting into an app the user has already left would be unsafe.
pub fn capture_target() -> Option<Target> {
    install_frontmost_tracker();
    let target = choose_clipboard_target(unsafe { workspace_frontmost_target() });
    if let Some(target) = target {
        remember_foreign_frontmost(target);
    }
    target
}

/// Resolves the application that owned the selection when the selection
/// shortcut fired. The current external frontmost application wins; when Nami
/// itself is frontmost, the most recent external activation is used. AX focus
/// is only a fallback because it can lag behind the workspace or be
/// unavailable. A missing target is allowed; callers then use the system-wide
/// element and broadcast key events.
pub fn capture_selection_target() -> Option<Target> {
    install_frontmost_tracker();
    let target = choose_selection_target(
        unsafe { workspace_frontmost_target() },
        last_foreign_frontmost(),
        || unsafe { ax_focused_application() },
    );
    if let Some(target) = target {
        remember_foreign_frontmost(target);
    }
    target
}

unsafe fn workspace_frontmost_target() -> Option<Target> {
    let application = frontmost()?;
    let target = identity(&application)?;
    (target.pid != std::process::id() as i32).then_some(target)
}

unsafe fn ax_focused_application() -> Option<Target> {
    let system = AXUIElementCreateSystemWide();
    if system.is_null() {
        return None;
    }
    let _ = AXUIElementSetMessagingTimeout(system, 0.25);
    let mut value: *const c_void = std::ptr::null();
    let status = AXUIElementCopyAttributeValue(
        system,
        (ns_string!("AXFocusedApplication") as *const NSString).cast(),
        &mut value,
    );
    CFRelease(system);
    if status != 0 || value.is_null() {
        return None;
    }
    let mut pid: i32 = 0;
    let valid = AXUIElementGetPid(value, &mut pid) == 0;
    CFRelease(value);
    if !valid || pid <= 0 || pid == std::process::id() as i32 {
        return None;
    }
    let application: Option<Retained<AnyObject>> = msg_send![
        class!(NSRunningApplication),
        runningApplicationWithProcessIdentifier: pid
    ];
    identity(application.as_deref()?)
}

unsafe fn running(target: Target) -> Option<Retained<AnyObject>> {
    let application: Option<Retained<AnyObject>> = msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: target.pid];
    let application = application?;
    let terminated: bool = msg_send![&*application, isTerminated];
    let actual = identity(&application)?;
    (!terminated && same_target(actual, target)).then_some(application)
}

pub fn open_clipboard_target(
    target: nami::clipboard::open_target::OpenTarget,
) -> Result<(), String> {
    use nami::clipboard::open_target::OpenTarget;
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        match target {
            OpenTarget::Link(url) | OpenTarget::Email(url) => {
                let url = NSURL::URLWithString(&NSString::from_str(url.as_str()))
                    .ok_or("invalid_argument")?;
                let opened: bool = msg_send![workspace, openURL: &*url];
                if !opened {
                    return Err("clipboard_open_failed".into());
                }
            }
            OpenTarget::File(path) => {
                let path = path.to_str().ok_or("invalid_argument")?;
                let urls = NSArray::from_retained_slice(&[NSURL::fileURLWithPath(
                    &NSString::from_str(path),
                )]);
                let _: () = msg_send![workspace, activateFileViewerSelectingURLs: &*urls];
            }
            OpenTarget::ChooseFile => return Err("invalid_argument".into()),
        }
    }
    Ok(())
}

#[cfg(feature = "ui-tests")]
pub fn test_activate_fixture(pid: u32, bundle: &std::path::Path) -> Result<Target, &'static str> {
    let pid = i32::try_from(pid).map_err(|_| "invalid fixture PID")?;
    let target = unsafe {
        let application: Option<Retained<AnyObject>> =
            msg_send![class!(NSRunningApplication), runningApplicationWithProcessIdentifier: pid];
        let application = application.ok_or("fixture process was not registered with AppKit")?;
        if !matching_fixture_bundle(&application, bundle) {
            return Err("fixture process did not register the generated bundle path");
        }
        identity(&application).ok_or("fixture process has no launch identity")?
    };
    if target_bundle_id(target).as_deref() != Some("dev.local.nami.cross-app-fixture") {
        return Err("fixture process did not register the expected bundle identity");
    }
    if !target_focused(target) {
        activate(target);
    }
    Ok(target)
}

#[cfg(feature = "ui-tests")]
unsafe fn matching_fixture_bundle(application: &AnyObject, bundle: &std::path::Path) -> bool {
    let url: Option<Retained<NSURL>> = msg_send![application, bundleURL];
    url.and_then(|url| url.path())
        .is_some_and(|path| std::path::Path::new(&path.to_string()) == bundle)
}

#[cfg(feature = "ui-tests")]
pub fn test_close_fixture(bundle: &std::path::Path) {
    unsafe {
        let applications: Retained<AnyObject> = msg_send![class!(NSRunningApplication), runningApplicationsWithBundleIdentifier: ns_string!("dev.local.nami.cross-app-fixture")];
        let count: usize = msg_send![&*applications, count];
        for index in 0..count {
            let application: Retained<AnyObject> = msg_send![&*applications, objectAtIndex: index];
            if matching_fixture_bundle(&application, bundle) {
                let _: bool = msg_send![&*application, forceTerminate];
            }
        }
    }
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
        same_target(actual, target) && target_alive(target)
    }
}

pub fn accessibility() -> bool {
    unsafe { AXIsProcessTrusted() }
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
        let name = application_bundle_name(&bundle)
            .or_else(|| {
                let filename: Option<Retained<NSString>> = msg_send![&*selected, lastPathComponent];
                filename.map(|value| value.to_string().trim_end_matches(".app").to_owned())
            })
            .unwrap_or_else(|| bundle_id.clone());
        Ok(Some(crate::clipboard_preferences::ExcludedApp {
            bundle_id,
            name,
            icon_data_url: None,
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
    #[cfg(feature = "ui-tests")]
    if std::env::var_os("NAMI_UI_TEST_REPORT").is_some()
        && std::env::var("NAMI_UI_TEST_CASE").as_deref() == Ok("cross-app")
        && (std::env::var("NAMI_UI_TEST_CROSS_APP").as_deref() != Ok("full")
            || target_bundle_id(target).as_deref() != Some("dev.local.nami.cross-app-fixture"))
    {
        return Err("cross_app_test_paste_not_enabled");
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
        let tiff = file_icon_tiff(workspace, &NSString::from_str(path))?;
        let bitmap: Option<Retained<AnyObject>> =
            msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff];
        bitmap_png(&*bitmap?).map(|data| png_data_url(&data))
    })
}

pub(crate) fn application_presentation(
    bundle_id: &str,
) -> Option<(Option<String>, Option<String>)> {
    objc2::rc::autoreleasepool(|_| unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let url: Option<Retained<AnyObject>> = msg_send![workspace,
            URLForApplicationWithBundleIdentifier: &*NSString::from_str(bundle_id)];
        let url = url?;
        let bundle: Option<Retained<AnyObject>> = msg_send![class!(NSBundle), bundleWithURL: &*url];
        let name = bundle
            .as_deref()
            .and_then(|bundle| application_bundle_name(bundle));
        let path: Option<Retained<NSString>> = msg_send![&*url, path];
        let icon = path.and_then(|path| application_thumbnail(workspace, &path));
        Some((name, icon))
    })
}

unsafe fn application_bundle_name(bundle: &AnyObject) -> Option<String> {
    [
        ns_string!("CFBundleDisplayName"),
        ns_string!("CFBundleName"),
    ]
    .into_iter()
    .find_map(|key| {
        let value: Option<Retained<AnyObject>> = msg_send![bundle, objectForInfoDictionaryKey: key];
        let name = value?.downcast::<NSString>().ok()?.to_string();
        (!name.trim().is_empty() && name.len() <= 512).then_some(name)
    })
}

unsafe fn application_thumbnail(workspace: *mut AnyObject, path: &NSString) -> Option<String> {
    let icon: Option<Retained<AnyObject>> = msg_send![workspace, iconForFile: path];
    let icon = icon?;
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(40.0, 40.0));
    // Export a single representation; TIFFRepresentation can expand all sizes beyond 64 MiB.
    let image: *const c_void = msg_send![&*icon, CGImageForProposedRect: &mut rect as *mut NSRect,
        context: std::ptr::null::<AnyObject>(), hints: std::ptr::null::<AnyObject>()];
    if image.is_null() {
        return None;
    }
    let bitmap: Allocated<AnyObject> = msg_send![class!(NSBitmapImageRep), alloc];
    let bitmap: Option<Retained<AnyObject>> = msg_send![bitmap, initWithCGImage: image];
    let png = bitmap_png(&*bitmap?)?;
    let thumb = super::clipboard_image_preview(png.as_bytes_unchecked(), 40).ok()?;
    Some(png_data_url(&NSData::from_vec(thumb)))
}

unsafe fn file_icon_tiff(workspace: *mut AnyObject, path: &NSString) -> Option<Retained<NSData>> {
    let icon: Option<Retained<AnyObject>> = msg_send![workspace, iconForFile: path];
    msg_send![&*icon?, TIFFRepresentation]
}

unsafe fn bitmap_png(bitmap: &AnyObject) -> Option<Retained<NSData>> {
    let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
    msg_send![bitmap, representationUsingType: 4usize, properties: properties]
}

fn png_data_url(data: &NSData) -> String {
    let encoded: Retained<NSString> =
        unsafe { msg_send![data, base64EncodedStringWithOptions: 0usize] };
    format!("data:image/png;base64,{encoded}")
}

#[cfg(test)]
mod tests {
    use super::{choose_clipboard_target, choose_selection_target, paste_chord, Target};

    fn target(pid: i32) -> Target {
        Target {
            pid,
            started: 0.0,
            bundle: 0,
        }
    }

    #[test]
    fn selection_target_falls_back_but_clipboard_target_does_not() {
        assert_eq!(
            choose_selection_target(Some(target(1)), Some(target(2)), || Some(target(3)))
                .map(|target| target.pid),
            Some(1)
        );
        assert_eq!(
            choose_selection_target(None, Some(target(2)), || Some(target(3)))
                .map(|target| target.pid),
            Some(2)
        );
        assert_eq!(
            choose_selection_target(None, None, || Some(target(3))).map(|target| target.pid),
            Some(3)
        );
        assert!(choose_selection_target(None, None, || None).is_none());
        assert!(choose_clipboard_target(None).is_none());
    }

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
}
