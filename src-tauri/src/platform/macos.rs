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
use std::sync::atomic::{AtomicU64, Ordering};
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
static ACTIVATION_EPOCH: AtomicU64 = AtomicU64::new(0);

pub fn activation_epoch() -> u64 {
    ACTIVATION_EPOCH.load(Ordering::SeqCst)
}

pub fn system_prefers_zh() -> bool {
    unsafe {
        let langs: Retained<NSArray<NSString>> = msg_send![class!(NSLocale), preferredLanguages];
        langs.firstObject().is_some_and(|first| {
            let tag = first.to_string().to_ascii_lowercase();
            tag == "zh-cn"
                || tag.starts_with("zh-cn-")
                || tag == "zh-sg"
                || tag.starts_with("zh-sg-")
                || tag == "zh-hans"
                || tag.starts_with("zh-hans-")
        })
    }
}

pub fn is_main_thread() -> bool {
    unsafe { msg_send![class!(NSThread), isMainThread] }
}

#[derive(Clone)]
struct TrayDiskUsageConfig {
    item_index: usize,
    value: String,
    min_width: f64,
}

struct TrayMenuObserverState {
    observer_ptr: usize,
    observed_status_item: usize,
}

static TRAY_DISK_USAGE_CONFIG: OnceLock<Mutex<Option<TrayDiskUsageConfig>>> = OnceLock::new();
static TRAY_MENU_OBSERVER: OnceLock<Mutex<Option<TrayMenuObserverState>>> = OnceLock::new();

objc2::define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ()]
    struct NamiTrayMenuObserver;

    impl NamiTrayMenuObserver {
        #[unsafe(method(observeValueForKeyPath:ofObject:change:context:))]
        fn observe_value(
            &self,
            _key_path: *mut AnyObject,
            object: *mut AnyObject,
            _change: *mut AnyObject,
            _context: *mut c_void,
        ) {
            if object.is_null() {
                return;
            }
            unsafe {
                let menu: *mut AnyObject = msg_send![object, menu];
                if menu.is_null() {
                    return;
                }
                let config = TRAY_DISK_USAGE_CONFIG
                    .get()
                    .and_then(|lock| lock.lock().ok())
                    .and_then(|guard| guard.clone());
                if let Some(config) = config {
                    apply_tray_disk_usage_to_menu(menu, &config);
                }
            }
        }
    }
);

unsafe fn apply_tray_disk_usage_to_menu(
    menu: *mut AnyObject,
    config: &TrayDiskUsageConfig,
) -> bool {
    if menu.is_null() {
        return false;
    }
    let _: () = msg_send![menu, setMinimumWidth: config.min_width];
    let count: isize = msg_send![menu, numberOfItems];
    if config.item_index as isize >= count {
        return false;
    }
    let Some(badge_class) = objc2::runtime::AnyClass::get(c"NSMenuItemBadge") else {
        return false;
    };
    let item: *mut AnyObject = msg_send![menu, itemAtIndex: config.item_index as isize];
    if item.is_null() {
        return false;
    }
    let alloc: Allocated<AnyObject> = msg_send![badge_class, alloc];
    let text = NSString::from_str(&config.value);
    let badge: Option<Retained<AnyObject>> = msg_send![alloc, initWithString: &*text];
    let Some(badge) = badge else {
        return false;
    };
    let _: () = msg_send![item, setBadge: &*badge];
    true
}

/// Applies a stable minimum width to the tray `NSMenu` attached to an `NSStatusItem`
/// and places the disk usage metric in the right-aligned `NSMenuItemBadge` column (macOS 14+)
/// matching shortcut alignment. Because `tray-icon` only attaches the `NSMenu` to `NSStatusItem`
/// right before `performClick:` opens the menu, this installs a KVO observer on `NSStatusItem.menu`
/// to apply the latest width and badge synchronously before every popup (and immediately if open).
/// Returns `true` when the native badge mechanism is available and active.
pub fn configure_tray_disk_usage(
    ns_status_item: *mut c_void,
    item_index: usize,
    value: &str,
    min_width: f64,
) -> bool {
    if ns_status_item.is_null() || objc2::runtime::AnyClass::get(c"NSMenuItemBadge").is_none() {
        return false;
    }
    let Some(mtm) = objc2_foundation::MainThreadMarker::new() else {
        return false;
    };
    let config = TrayDiskUsageConfig {
        item_index,
        value: value.to_owned(),
        min_width,
    };
    if let Ok(mut guard) = TRAY_DISK_USAGE_CONFIG
        .get_or_init(|| Mutex::new(None))
        .lock()
    {
        *guard = Some(config.clone());
    }
    let status_item_addr = ns_status_item as usize;
    unsafe {
        let status_item = ns_status_item.cast::<AnyObject>();
        if let Ok(mut observer_guard) = TRAY_MENU_OBSERVER.get_or_init(|| Mutex::new(None)).lock() {
            let needs_attach = observer_guard
                .as_ref()
                .is_none_or(|state| state.observed_status_item != status_item_addr);
            if needs_attach {
                let observer: Retained<NamiTrayMenuObserver> = {
                    let alloc = mtm.alloc::<NamiTrayMenuObserver>().set_ivars(());
                    msg_send![super(alloc), init]
                };
                // NSKeyValueObservingOptionNew = 1
                let _: () = msg_send![
                    status_item,
                    addObserver: &*observer,
                    forKeyPath: ns_string!("menu"),
                    options: 1usize,
                    context: std::ptr::null_mut::<c_void>()
                ];
                *observer_guard = Some(TrayMenuObserverState {
                    observer_ptr: Retained::into_raw(observer) as usize,
                    observed_status_item: status_item_addr,
                });
            }
        }
        let menu: *mut AnyObject = msg_send![status_item, menu];
        if !menu.is_null() {
            apply_tray_disk_usage_to_menu(menu, &config);
        }
    }
    true
}

/// Detaches the `NSStatusItem.menu` KVO observer before the tray icon is dropped on exit.
pub fn cleanup_tray_disk_usage_observer(ns_status_item: *mut c_void) {
    let Some(state) = TRAY_MENU_OBSERVER
        .get()
        .and_then(|lock| lock.lock().ok())
        .and_then(|mut guard| guard.take())
    else {
        return;
    };
    unsafe {
        if !ns_status_item.is_null() && state.observed_status_item == ns_status_item as usize {
            let status_item = ns_status_item.cast::<AnyObject>();
            let observer = state.observer_ptr as *mut AnyObject;
            let _: () = msg_send![
                status_item,
                removeObserver: observer,
                forKeyPath: ns_string!("menu")
            ];
        }
        if let Some(observer) = Retained::from_raw(state.observer_ptr as *mut NamiTrayMenuObserver)
        {
            drop(observer);
        }
    }
}

#[cfg(all(feature = "ui-tests", not(feature = "portable-ui")))]
pub fn test_verify_tray_disk_usage_kvo() -> Result<(), String> {
    if objc2::runtime::AnyClass::get(c"NSMenuItemBadge").is_none() {
        return Ok(());
    }
    unsafe {
        let status_bar: *mut AnyObject = msg_send![class!(NSStatusBar), systemStatusBar];
        // NSVariableStatusItemLength = -1.0
        let status_item: Retained<AnyObject> = msg_send![status_bar, statusItemWithLength: -1.0f64];
        let menu_alloc: Allocated<AnyObject> = msg_send![class!(NSMenu), alloc];
        let menu: Retained<AnyObject> = msg_send![menu_alloc, init];
        for (title, key) in [
            ("拷贝记录", "`"),
            ("打开翻译", "t"),
            ("选字翻译", "o"),
            ("偏好设置", ","),
            ("暂停监听", ""),
            ("磁盘占用", ""),
            ("退出 Nami", "q"),
        ] {
            let item_alloc: Allocated<AnyObject> = msg_send![class!(NSMenuItem), alloc];
            let title_ns = NSString::from_str(title);
            let key_ns = NSString::from_str(key);
            let item: Retained<AnyObject> = msg_send![
                item_alloc,
                initWithTitle: &*title_ns,
                action: Option::<objc2::runtime::Sel>::None,
                keyEquivalent: &*key_ns
            ];
            let _: () = msg_send![&*menu, addItem: &*item];
        }
        let disk_index = 5usize;
        let status_ptr = Retained::as_ptr(&status_item) as *mut c_void;
        let result = (|| -> Result<(), String> {
            // Reproduces tray-icon behavior where NSStatusItem.menu is nil until popup.
            if !configure_tray_disk_usage(status_ptr, disk_index, "1.3 MB", 200.0) {
                return Err("configure_tray_disk_usage returned false on macOS 14+".into());
            }
            let _: () = msg_send![&*status_item, setMenu: &*menu];
            let _: () = msg_send![&*menu, update];
            let item: *mut AnyObject = msg_send![&*menu, itemAtIndex: disk_index as isize];
            let title: Retained<NSString> = msg_send![item, title];
            if title.to_string() != "磁盘占用" {
                return Err(format!("unexpected disk usage title: {}", title));
            }
            let badge: Option<Retained<AnyObject>> = msg_send![item, badge];
            let badge = badge.ok_or("missing NSMenuItemBadge after setMenu:")?;
            let badge_text: Option<Retained<NSString>> = msg_send![&*badge, stringValue];
            if badge_text.map(|s| s.to_string()).as_deref() != Some("1.3 MB") {
                return Err("unexpected initial badge text".into());
            }
            let size1: NSSize = msg_send![&*menu, size];
            if (size1.width - 200.0).abs() > 0.5 {
                return Err(format!("unexpected menu width for 1.3 MB: {}", size1.width));
            }
            let _: () = msg_send![&*status_item, setMenu: std::ptr::null::<AnyObject>()];
            if !configure_tray_disk_usage(status_ptr, disk_index, "999.9 GB", 200.0) {
                return Err("configure_tray_disk_usage update returned false".into());
            }
            let _: () = msg_send![&*status_item, setMenu: &*menu];
            let _: () = msg_send![&*menu, update];
            let badge2: Option<Retained<AnyObject>> = msg_send![item, badge];
            let badge2 = badge2.ok_or("missing updated NSMenuItemBadge")?;
            let badge_text2: Option<Retained<NSString>> = msg_send![&*badge2, stringValue];
            if badge_text2.map(|s| s.to_string()).as_deref() != Some("999.9 GB") {
                return Err("unexpected updated badge text".into());
            }
            let size2: NSSize = msg_send![&*menu, size];
            if (size2.width - size1.width).abs() > 0.01 {
                return Err(format!(
                    "menu width jittered from {} to {}",
                    size1.width, size2.width
                ));
            }
            Ok(())
        })();
        let _: () = msg_send![&*status_item, setMenu: std::ptr::null::<AnyObject>()];
        cleanup_tray_disk_usage_observer(status_ptr);
        let _: () = msg_send![status_bar, removeStatusItem: &*status_item];
        result
    }
}

const MAX_CLIPBOARD_FOCUS_SOURCES: usize = 32;

#[derive(Default)]
struct ClipboardFocus {
    active: Option<String>,
    boundary: Option<i64>,
    outgoing: Vec<String>,
}

impl ClipboardFocus {
    fn observe(&mut self, active: Option<String>, version: i64) {
        if self.active == active {
            return;
        }
        if self.boundary != Some(version) {
            self.boundary = Some(version);
            self.outgoing.clear();
        }
        if let Some(previous) = std::mem::replace(&mut self.active, active) {
            if self.outgoing.len() < MAX_CLIPBOARD_FOCUS_SOURCES
                && !self.outgoing.contains(&previous)
            {
                self.outgoing.push(previous);
            }
        }
    }

    fn blocks(&self, version: i64, excluded: impl Fn(&str) -> bool) -> bool {
        self.active.as_deref().is_some_and(&excluded)
            || (self.boundary == Some(version)
                && (self.outgoing.len() == MAX_CLIPBOARD_FOCUS_SOURCES
                    || self.outgoing.iter().any(|bundle| excluded(bundle))))
    }
}

static CLIPBOARD_FOCUS: Mutex<ClipboardFocus> = Mutex::new(ClipboardFocus {
    active: None,
    boundary: None,
    outgoing: Vec::new(),
});

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
            ACTIVATION_EPOCH.fetch_add(1, Ordering::SeqCst);
            CLIPBOARD_FOCUS
                .lock()
                .unwrap()
                .observe(bundle_identifier(&*app), clipboard_version());
            if let Some(target) = identity(&*app) {
                if target.pid != std::process::id() as i32 {
                    remember_foreign_frontmost(target);
                }
            }
        });
        let main_queue: *mut AnyObject = msg_send![class!(NSOperationQueue), mainQueue];
        let token: *mut AnyObject = msg_send![
            center,
            addObserverForName: ns_string!("NSWorkspaceDidActivateApplicationNotification"),
            object: std::ptr::null::<AnyObject>(),
            queue: main_queue,
            usingBlock: &*handler
        ];
        if !token.is_null() {
            // The tracker lives for the whole process; keeping the token
            // retained avoids relying on autorelease lifetime.
            std::mem::forget(Retained::retain(token));
        }
        CLIPBOARD_FOCUS.lock().unwrap().active = frontmost()
            .as_deref()
            .and_then(|app| bundle_identifier(app));
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

#[cfg(feature = "ui-tests")]
pub fn test_copy_from_fixture(target: Target) -> Result<(), &'static str> {
    selection::send_copy(Some(target))
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
    #[cfg(feature = "ui-tests")]
    fn CGEventCreate(source: *const c_void) -> *mut c_void;
    #[cfg(feature = "ui-tests")]
    fn CGEventGetLocation(event: *const c_void) -> NSPoint;
    #[cfg(feature = "ui-tests")]
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        kind: u32,
        position: NSPoint,
        button: u32,
    ) -> *mut c_void;
    fn AXUIElementCreateSystemWide() -> *const c_void;
    fn AXUIElementSetMessagingTimeout(element: *const c_void, timeout: f32) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXUIElementCopyParameterizedAttributeValue(
        element: *const c_void,
        attribute: *const c_void,
        parameter: *const c_void,
        value: *mut *const c_void,
    ) -> i32;
    fn AXValueGetValue(value: *const c_void, kind: i32, out: *mut c_void) -> bool;
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

// libSystem exports the main queue object itself; `dispatch_get_main_queue()`
// is only an inline wrapper around it.
unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *mut c_void,
        context: *mut c_void,
        work: Option<unsafe extern "C" fn(*mut c_void)>,
    );
}

/// Finder reveals that returned on the main queue; observed by the native UI case.
#[cfg(feature = "ui-tests")]
pub(crate) static FINDER_REVEALS: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

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

/// Uses only the current external app for paste; the user may have left any previous target.
pub fn capture_target() -> Option<Target> {
    install_frontmost_tracker();
    let target = choose_clipboard_target(unsafe { workspace_frontmost_target() });
    if let Some(target) = target {
        remember_foreign_frontmost(target);
    }
    target
}

/// None means skip the capture. A transition version can be kept without an
/// app label if none of its possible foreground sources is excluded.
pub(super) fn clipboard_source(
    version: i64,
    excluded: impl Fn(&str) -> bool,
) -> Option<Option<Target>> {
    install_frontmost_tracker();
    let application = unsafe { frontmost() };
    let mut focus = CLIPBOARD_FOCUS.lock().unwrap();
    // AppKit's frontmost value can change before its activation notification arrives.
    focus.observe(
        application
            .as_deref()
            .and_then(|app| unsafe { bundle_identifier(app) }),
        version,
    );
    if focus.blocks(version, excluded) {
        return None;
    }
    Some(if focus.boundary == Some(version) {
        None
    } else {
        application
            .as_deref()
            .and_then(|app| unsafe { identity(app) })
            .filter(|target| target.pid != std::process::id() as i32)
    })
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

/// Opens a clipboard target. Link and Email open synchronously on the main
/// thread and report failure. Revealing a file is queued to the main queue and
/// cannot report failure: AppKit runs a nested event loop until Finder answers,
/// and tao keeps its event handler lock for the whole callback of a task posted
/// with `run_on_main_thread`. A view redraw inside that nested loop re-takes
/// the lock and would block the main thread against itself.
pub fn open_translation_language_settings() -> Result<(), String> {
    if !nami::translation::apple_available() {
        return Err("platform_unavailable".into());
    }
    unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let url = NSURL::URLWithString(ns_string!(
            "x-apple.systempreferences:com.apple.Localization-Settings.extension"
        ))
        .ok_or("translation_settings_open_failed")?;
        let opened: bool = msg_send![workspace, openURL: &*url];
        opened
            .then_some(())
            .ok_or_else(|| "translation_settings_open_failed".into())
    }
}

pub fn open_clipboard_target(
    target: nami::clipboard::open_target::OpenTarget,
) -> Result<(), String> {
    use nami::clipboard::open_target::OpenTarget;
    unsafe {
        match target {
            OpenTarget::Link(url) | OpenTarget::Email(url) => {
                let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
                let url = NSURL::URLWithString(&NSString::from_str(url.as_str()))
                    .ok_or("invalid_argument")?;
                let opened: bool = msg_send![workspace, openURL: &*url];
                if !opened {
                    return Err("clipboard_open_failed".into());
                }
            }
            OpenTarget::File(path) => {
                let path = path.to_str().ok_or("invalid_argument")?.to_owned();
                dispatch_async_f(
                    std::ptr::from_ref(&_dispatch_main_q).cast_mut(),
                    Box::into_raw(Box::new(path)).cast(),
                    Some(reveal_in_finder),
                );
            }
            OpenTarget::ChooseFile => return Err("invalid_argument".into()),
        }
    }
    Ok(())
}

// Runs on the main thread from the main queue, outside tao's event handler callbacks.
unsafe extern "C" fn reveal_in_finder(context: *mut c_void) {
    let path = Box::from_raw(context.cast::<String>());
    objc2::rc::autoreleasepool(|_| {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let urls =
            NSArray::from_retained_slice(&[NSURL::fileURLWithPath(&NSString::from_str(&path))]);
        let _: () = msg_send![workspace, activateFileViewerSelectingURLs: &*urls];
    });
    #[cfg(feature = "ui-tests")]
    FINDER_REVEALS.fetch_add(1, Ordering::SeqCst);
}

pub fn open_directory(path: &std::path::Path) -> Result<(), String> {
    if !path.is_dir() {
        return Err("storage_path".into());
    }
    let path = path.to_str().ok_or("storage_path")?.to_owned();
    unsafe {
        dispatch_async_f(
            std::ptr::from_ref(&_dispatch_main_q).cast_mut(),
            Box::into_raw(Box::new(path)).cast(),
            Some(open_directory_in_finder),
        );
    }
    Ok(())
}

unsafe extern "C" fn open_directory_in_finder(context: *mut c_void) {
    let path = Box::from_raw(context.cast::<String>());
    objc2::rc::autoreleasepool(|_| {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let url = NSURL::fileURLWithPath(&NSString::from_str(&path));
        let _: bool = msg_send![workspace, openURL: &*url];
    });
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

#[cfg(feature = "ui-tests")]
static TEST_LAUNCH_AT_LOGIN: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

pub fn launch_at_login_enabled() -> bool {
    #[cfg(feature = "ui-tests")]
    {
        TEST_LAUNCH_AT_LOGIN.load(Ordering::SeqCst)
    }
    #[cfg(not(feature = "ui-tests"))]
    unsafe {
        let Some(cls) = objc2::runtime::AnyClass::get(c"SMAppService") else {
            return false;
        };
        let service: Retained<AnyObject> = msg_send![cls, mainAppService];
        let status: isize = msg_send![&*service, status];
        status == 1
    }
}

pub fn set_launch_at_login(enabled: bool) -> Result<bool, String> {
    #[cfg(feature = "ui-tests")]
    {
        TEST_LAUNCH_AT_LOGIN.store(enabled, Ordering::SeqCst);
        Ok(enabled)
    }
    #[cfg(not(feature = "ui-tests"))]
    unsafe {
        let cls = objc2::runtime::AnyClass::get(c"SMAppService")
            .ok_or_else(|| "platform_unavailable".to_string())?;
        let service: Retained<AnyObject> = msg_send![cls, mainAppService];
        let mut error: *mut AnyObject = std::ptr::null_mut();
        let ok: bool = if enabled {
            msg_send![&*service, registerAndReturnError: &mut error]
        } else {
            msg_send![&*service, unregisterAndReturnError: &mut error]
        };
        if !ok {
            return Err("launch_at_login_failed".into());
        }
        Ok(launch_at_login_enabled())
    }
}

unsafe fn bundle_identifier(application: &AnyObject) -> Option<String> {
    let bundle: Option<Retained<NSString>> = msg_send![application, bundleIdentifier];
    bundle.map(|id| id.to_string())
}

#[cfg(feature = "ui-tests")]
pub(super) fn target_bundle_id(target: Target) -> Option<String> {
    unsafe { bundle_identifier(&*running(target)?) }
}

pub(crate) fn pick_image_save_path(
    window: &tauri::Window,
    complete: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    use std::cell::Cell;
    unsafe {
        let parent = window.ns_window().map_err(|_| "clipboard_save_dialog")?;
        let parent = parent.cast::<AnyObject>();
        let visible: bool = msg_send![parent, isVisible];
        let sheet: *mut AnyObject = msg_send![parent, attachedSheet];
        if !visible || !sheet.is_null() {
            return Err("clipboard_save_dialog".into());
        }
        let panel: Retained<AnyObject> = msg_send![class!(NSSavePanel), savePanel];
        let png: Retained<AnyObject> =
            msg_send![class!(UTType), typeWithIdentifier: ns_string!("public.png")];
        let types = NSArray::from_retained_slice(&[png]);
        let _: () = msg_send![&*panel, setAllowedContentTypes: &*types];
        let _: () = msg_send![&*panel, setAllowsOtherFileTypes: false];
        let _: () = msg_send![&*panel, setCanCreateDirectories: true];
        let name = chrono::Local::now()
            .format("Nami_%Y-%m-%d_%H-%M-%S.png")
            .to_string();
        let _: () = msg_send![&*panel, setNameFieldStringValue: &*NSString::from_str(&name)];
        let result_panel = panel.clone();
        let complete = Cell::new(Some(complete));
        let finish = RcBlock::new(move |response: isize| {
            let Some(complete) = complete.take() else {
                return;
            };
            complete(match response {
                1 => {
                    let url: Option<Retained<NSURL>> = msg_send![&*result_panel, URL];
                    url.and_then(|url| url.path())
                        .map(|path| Some(path.to_string().into()))
                        .ok_or_else(|| "clipboard_save_dialog".into())
                }
                0 => Ok(None),
                _ => Err("clipboard_save_dialog".into()),
            });
        });
        let _: () =
            msg_send![&*panel, beginSheetModalForWindow: parent, completionHandler: &*finish];
    }
    Ok(())
}

pub(crate) fn write_image_export(
    destination: &std::path::Path,
    png: Vec<u8>,
) -> Result<(), String> {
    let filename = destination.to_str().ok_or("clipboard_save_failed")?;
    let url = NSURL::fileURLWithPath(&NSString::from_str(filename));
    // The save panel confirms replacement. Foundation commits atomically so a failed write
    // cannot leave a partly written destination or truncate the prior file.
    NSData::from_vec(png)
        .writeToURL_atomically(&url, true)
        .then_some(())
        .ok_or_else(|| "clipboard_save_failed".into())
}

pub(crate) fn pick_backup_save_path(
    window: &tauri::Window,
    complete: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    use std::cell::Cell;
    unsafe {
        let parent = window.ns_window().map_err(|_| "backup_dialog_failed")?;
        let parent = parent.cast::<AnyObject>();
        let visible: bool = msg_send![parent, isVisible];
        let sheet: *mut AnyObject = msg_send![parent, attachedSheet];
        if !visible || !sheet.is_null() {
            return Err("backup_dialog_failed".into());
        }
        let panel: Retained<AnyObject> = msg_send![class!(NSSavePanel), savePanel];
        let backup_type: Option<Retained<AnyObject>> =
            msg_send![class!(UTType), typeWithFilenameExtension: ns_string!("namibak")];
        if let Some(backup_type) = backup_type {
            let types = NSArray::from_retained_slice(&[backup_type]);
            let _: () = msg_send![&*panel, setAllowedContentTypes: &*types];
        }
        let _: () = msg_send![&*panel, setAllowsOtherFileTypes: false];
        let _: () = msg_send![&*panel, setCanCreateDirectories: true];
        let name = chrono::Local::now()
            .format("Nami_Backup_%Y-%m-%d_%H-%M-%S.namibak")
            .to_string();
        let _: () = msg_send![&*panel, setNameFieldStringValue: &*NSString::from_str(&name)];
        let result_panel = panel.clone();
        let complete = Cell::new(Some(complete));
        let finish = RcBlock::new(move |response: isize| {
            let Some(complete) = complete.take() else {
                return;
            };
            complete(match response {
                1 => {
                    let url: Option<Retained<NSURL>> = msg_send![&*result_panel, URL];
                    url.and_then(|url| url.path())
                        .map(|path| Some(path.to_string().into()))
                        .ok_or_else(|| "backup_dialog_failed".into())
                }
                0 => Ok(None),
                _ => Err("backup_dialog_failed".into()),
            });
        });
        let _: () =
            msg_send![&*panel, beginSheetModalForWindow: parent, completionHandler: &*finish];
    }
    Ok(())
}

pub(crate) fn pick_backup_open_path(
    window: &tauri::Window,
    complete: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    use std::cell::Cell;
    unsafe {
        let parent = window.ns_window().map_err(|_| "backup_dialog_failed")?;
        let parent = parent.cast::<AnyObject>();
        let visible: bool = msg_send![parent, isVisible];
        let sheet: *mut AnyObject = msg_send![parent, attachedSheet];
        if !visible || !sheet.is_null() {
            return Err("backup_dialog_failed".into());
        }
        let panel: Retained<AnyObject> = msg_send![class!(NSOpenPanel), openPanel];
        let _: () = msg_send![&*panel, setCanChooseFiles: true];
        let _: () = msg_send![&*panel, setCanChooseDirectories: false];
        let _: () = msg_send![&*panel, setAllowsMultipleSelection: false];
        let backup_type: Option<Retained<AnyObject>> =
            msg_send![class!(UTType), typeWithFilenameExtension: ns_string!("namibak")];
        if let Some(backup_type) = backup_type {
            let types = NSArray::from_retained_slice(&[backup_type]);
            let _: () = msg_send![&*panel, setAllowedContentTypes: &*types];
        }
        let result_panel = panel.clone();
        let complete = Cell::new(Some(complete));
        let finish = RcBlock::new(move |response: isize| {
            let Some(complete) = complete.take() else {
                return;
            };
            complete(match response {
                1 => {
                    let url: Option<Retained<NSURL>> = msg_send![&*result_panel, URL];
                    url.and_then(|url| url.path())
                        .map(|path| Some(path.to_string().into()))
                        .ok_or_else(|| "backup_dialog_failed".into())
                }
                0 => Ok(None),
                _ => Err("backup_dialog_failed".into()),
            });
        });
        let _: () =
            msg_send![&*panel, beginSheetModalForWindow: parent, completionHandler: &*finish];
    }
    Ok(())
}

pub(crate) fn pick_excluded_app(
    window: &tauri::Window,
    complete: impl FnOnce(Result<Option<crate::clipboard_preferences::ExcludedApp>, String>) + 'static,
) -> Result<(), String> {
    use std::cell::Cell;
    unsafe {
        let parent = window.ns_window().map_err(|_| "clipboard_manage_apps")?;
        let parent = parent.cast::<AnyObject>();
        let visible: bool = msg_send![parent, isVisible];
        let sheet: *mut AnyObject = msg_send![parent, attachedSheet];
        if !visible || !sheet.is_null() {
            return Err("clipboard_manage_apps".into());
        }
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
        let result_panel = panel.clone();
        let complete = Cell::new(Some(complete));
        let finish = RcBlock::new(move |response: isize| {
            let Some(complete) = complete.take() else {
                return;
            };
            let outcome = match response {
                1 => (|| {
                    let selected: Option<Retained<AnyObject>> = msg_send![&*result_panel, URL];
                    let selected = selected.ok_or_else(|| "clipboard_invalid_app".to_string())?;
                    let extension: Option<Retained<NSString>> =
                        msg_send![&*selected, pathExtension];
                    if !extension.is_some_and(|value| value.to_string().eq_ignore_ascii_case("app"))
                    {
                        return Err("clipboard_invalid_app".into());
                    }
                    let bundle: Option<Retained<AnyObject>> =
                        msg_send![class!(NSBundle), bundleWithURL: &*selected];
                    let bundle = bundle.ok_or_else(|| "clipboard_invalid_app".to_string())?;
                    let identifier: Option<Retained<NSString>> =
                        msg_send![&*bundle, bundleIdentifier];
                    let bundle_id = identifier
                        .ok_or_else(|| "clipboard_invalid_app".to_string())?
                        .to_string();
                    // Select by Bundle ID; retain no app path and launch no selected app.
                    let name = application_bundle_name(&bundle)
                        .or_else(|| {
                            let filename: Option<Retained<NSString>> =
                                msg_send![&*selected, lastPathComponent];
                            filename
                                .map(|value| value.to_string().trim_end_matches(".app").to_owned())
                        })
                        .unwrap_or_else(|| bundle_id.clone());
                    Ok(Some(crate::clipboard_preferences::ExcludedApp {
                        bundle_id,
                        name,
                        icon_data_url: None,
                    }))
                })(),
                0 => Ok(None),
                _ => Err("clipboard_manage_apps".into()),
            };
            complete(outcome);
        });
        let _: () =
            msg_send![&*panel, beginSheetModalForWindow: parent, completionHandler: &*finish];
    }
    Ok(())
}

pub(crate) fn pick_storage_directory(
    window: &tauri::Window,
    initial_dir: &std::path::Path,
    complete: impl FnOnce(Result<Option<std::path::PathBuf>, String>) + 'static,
) -> Result<(), String> {
    use std::cell::Cell;
    unsafe {
        let parent = window.ns_window().map_err(|_| "storage_dialog_failed")?;
        let parent = parent.cast::<AnyObject>();
        let visible: bool = msg_send![parent, isVisible];
        let sheet: *mut AnyObject = msg_send![parent, attachedSheet];
        if !visible || !sheet.is_null() {
            return Err("storage_dialog_failed".into());
        }
        let panel: Retained<AnyObject> = msg_send![class!(NSOpenPanel), openPanel];
        if let Some(dir_str) = initial_dir.to_str() {
            if initial_dir.is_dir() {
                let dir_url = NSURL::fileURLWithPath(&NSString::from_str(dir_str));
                let _: () = msg_send![&*panel, setDirectoryURL: &*dir_url];
            }
        }
        let _: () = msg_send![&*panel, setCanChooseFiles: false];
        let _: () = msg_send![&*panel, setCanChooseDirectories: true];
        let _: () = msg_send![&*panel, setCanCreateDirectories: true];
        let _: () = msg_send![&*panel, setAllowsMultipleSelection: false];
        let result_panel = panel.clone();
        let complete = Cell::new(Some(complete));
        let finish = RcBlock::new(move |response: isize| {
            let Some(complete) = complete.take() else {
                return;
            };
            complete(match response {
                1 => {
                    let url: Option<Retained<NSURL>> = msg_send![&*result_panel, URL];
                    url.and_then(|url| url.path())
                        .map(|path| Some(path.to_string().into()))
                        .ok_or_else(|| "storage_dialog_failed".into())
                }
                0 => Ok(None),
                _ => Err("storage_dialog_failed".into()),
            });
        });
        let _: () =
            msg_send![&*panel, beginSheetModalForWindow: parent, completionHandler: &*finish];
    }
    Ok(())
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
        let icon_data_url =
            path.and_then(|path| file_icon_data_url(&path.to_string(), true, false));
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

/// Pointer position in global screen points, measured from the top-left corner of the main display.
#[cfg(feature = "ui-tests")]
pub fn test_pointer_location() -> Result<(f64, f64), String> {
    unsafe {
        let event = CGEventCreate(std::ptr::null());
        if event.is_null() {
            return Err("pointer_location".into());
        }
        let point = CGEventGetLocation(event);
        CFRelease(event);
        Ok((point.x, point.y))
    }
}

/// Posts a real pointer move to the HID tap, so the view under it sees the hover a user's move produces.
#[cfg(feature = "ui-tests")]
pub fn test_move_pointer(x: f64, y: f64) -> Result<(), String> {
    unsafe {
        // HID system state, so the move is a user event rather than a private one.
        let source = CGEventSourceCreate(1);
        // kCGEventMouseMoved with kCGMouseButtonLeft.
        let event = CGEventCreateMouseEvent(source, 5, NSPoint::new(x, y), 0);
        if !source.is_null() {
            CFRelease(source);
        }
        if event.is_null() {
            return Err("pointer_move".into());
        }
        // Only the HID tap moves the pointer; the annotated tap that carries key chords does not.
        CGEventPost(0, event);
        CFRelease(event);
    }
    // The window server drops the event when the process is not trusted: never report a move that did not happen.
    for _ in 0..50 {
        if test_pointer_location()
            .is_ok_and(|(now_x, now_y)| (now_x - x).abs() < 1.0 && (now_y - y).abs() < 1.0)
        {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Err(match test_pointer_location() {
        Ok((now_x, now_y)) => format!("pointer_move: ({now_x}, {now_y}) instead of ({x}, {y})"),
        Err(error) => format!("pointer_move: {error} instead of ({x}, {y})"),
    })
}

// NSWorkspace file icons may be queried on a worker; no window/view API is touched.
pub fn file_icon_data_url(path: &str, exists: bool, is_dir: bool) -> Option<String> {
    if !exists && is_dir {
        return folder_icon_data_url();
    }
    objc2::rc::autoreleasepool(|_| unsafe {
        let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
        let ns_path = NSString::from_str(path);
        application_thumbnail(workspace, &ns_path).or_else(|| {
            let tiff = file_icon_tiff(workspace, &ns_path)?;
            let bitmap: Option<Retained<AnyObject>> =
                msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff];
            bitmap_png(&*bitmap?).map(|data| png_data_url(&data))
        })
    })
}

fn folder_icon_data_url() -> Option<String> {
    static FOLDER_ICON: OnceLock<Option<String>> = OnceLock::new();
    FOLDER_ICON
        .get_or_init(|| {
            objc2::rc::autoreleasepool(|_| unsafe {
                let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
                let ut_type: Option<Retained<AnyObject>> =
                    msg_send![class!(UTType), typeWithIdentifier: ns_string!("public.folder")];
                let ut_type = ut_type?;
                let icon: Option<Retained<AnyObject>> =
                    msg_send![workspace, iconForContentType: &*ut_type];
                let icon = icon?;
                image_thumbnail(&icon).or_else(|| {
                    let tiff: Option<Retained<NSData>> = msg_send![&*icon, TIFFRepresentation];
                    let bitmap: Option<Retained<AnyObject>> =
                        msg_send![class!(NSBitmapImageRep), imageRepWithData: &*tiff?];
                    bitmap_png(&*bitmap?).map(|data| png_data_url(&data))
                })
            })
        })
        .clone()
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
    image_thumbnail(&icon)
}

unsafe fn image_thumbnail(icon: &AnyObject) -> Option<String> {
    let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(40.0, 40.0));
    // Export a single representation; TIFFRepresentation can expand all sizes beyond 64 MiB.
    let image: *const c_void = msg_send![icon, CGImageForProposedRect: &mut rect as *mut NSRect,
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

unsafe fn primary_screen_height() -> Option<f64> {
    let screens: Option<Retained<NSArray<AnyObject>>> = msg_send![class!(NSScreen), screens];
    let screens = screens?;
    let first = screens.firstObject()?;
    let frame: NSRect = msg_send![&*first, frame];
    (frame.size.height > 0.0).then_some(frame.size.height)
}

unsafe fn screen_visible_frame_for_point(
    point: Option<NSPoint>,
) -> Option<nami::model::ScreenRect> {
    let screens: Option<Retained<NSArray<AnyObject>>> = msg_send![class!(NSScreen), screens];
    let to_rect = |rect: NSRect| nami::model::ScreenRect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    };
    if let (Some(point), Some(screens)) = (point, &screens) {
        for index in 0..screens.count() {
            let screen = screens.objectAtIndex(index);
            let frame: NSRect = msg_send![&*screen, frame];
            if point.x >= frame.origin.x
                && point.x <= frame.origin.x + frame.size.width
                && point.y >= frame.origin.y
                && point.y <= frame.origin.y + frame.size.height
            {
                let visible: NSRect = msg_send![&*screen, visibleFrame];
                return Some(to_rect(visible));
            }
        }
    }
    let main: Option<Retained<AnyObject>> = msg_send![class!(NSScreen), mainScreen];
    if let Some(main) = main {
        let visible: NSRect = msg_send![&*main, visibleFrame];
        return Some(to_rect(visible));
    }
    screens.and_then(|screens| {
        screens.firstObject().map(|screen| {
            let visible: NSRect = msg_send![&*screen, visibleFrame];
            to_rect(visible)
        })
    })
}

unsafe fn focused_caret_rect() -> Option<nami::model::ScreenRect> {
    const K_AX_VALUE_CGPOINT_TYPE: i32 = 1;
    const K_AX_VALUE_CGSIZE_TYPE: i32 = 2;
    const K_AX_VALUE_CGRECT_TYPE: i32 = 3;

    if !AXIsProcessTrusted() {
        return None;
    }
    let primary_height = primary_screen_height()?;
    let system = AXUIElementCreateSystemWide();
    if system.is_null() {
        return None;
    }
    let _ = AXUIElementSetMessagingTimeout(system, 0.08);
    let mut focused: *const c_void = std::ptr::null();
    let status = AXUIElementCopyAttributeValue(
        system,
        (ns_string!("AXFocusedUIElement") as *const NSString).cast(),
        &mut focused,
    );
    CFRelease(system);
    if status != 0 || focused.is_null() {
        return None;
    }
    let _ = AXUIElementSetMessagingTimeout(focused, 0.08);

    let mut range: *const c_void = std::ptr::null();
    let range_ok = AXUIElementCopyAttributeValue(
        focused,
        (ns_string!("AXSelectedTextRange") as *const NSString).cast(),
        &mut range,
    ) == 0
        && !range.is_null();

    let mut cg_rect: Option<NSRect> = None;
    if range_ok {
        let mut bounds_val: *const c_void = std::ptr::null();
        let bounds_ok = AXUIElementCopyParameterizedAttributeValue(
            focused,
            (ns_string!("AXBoundsForRange") as *const NSString).cast(),
            range,
            &mut bounds_val,
        ) == 0
            && !bounds_val.is_null();
        CFRelease(range);
        if bounds_ok {
            let mut rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0));
            if AXValueGetValue(
                bounds_val,
                K_AX_VALUE_CGRECT_TYPE,
                (&mut rect as *mut NSRect).cast(),
            ) && rect.size.height > 0.0
                && rect.origin.x.is_finite()
                && rect.origin.y.is_finite()
            {
                cg_rect = Some(rect);
            }
            CFRelease(bounds_val);
        }
    }

    if cg_rect.is_none() {
        // Fallback for compact single-line inputs that expose AXPosition/AXSize without range bounds.
        let mut pos_val: *const c_void = std::ptr::null();
        let mut size_val: *const c_void = std::ptr::null();
        let pos_ok = AXUIElementCopyAttributeValue(
            focused,
            (ns_string!("AXPosition") as *const NSString).cast(),
            &mut pos_val,
        ) == 0
            && !pos_val.is_null();
        let size_ok = AXUIElementCopyAttributeValue(
            focused,
            (ns_string!("AXSize") as *const NSString).cast(),
            &mut size_val,
        ) == 0
            && !size_val.is_null();
        if pos_ok && size_ok {
            let mut pos = NSPoint::new(0.0, 0.0);
            let mut size = NSSize::new(0.0, 0.0);
            if AXValueGetValue(
                pos_val,
                K_AX_VALUE_CGPOINT_TYPE,
                (&mut pos as *mut NSPoint).cast(),
            ) && AXValueGetValue(
                size_val,
                K_AX_VALUE_CGSIZE_TYPE,
                (&mut size as *mut NSSize).cast(),
            ) && size.height > 0.0
                && size.height <= 64.0
                && pos.x.is_finite()
                && pos.y.is_finite()
            {
                cg_rect = Some(NSRect::new(pos, size));
            }
        }
        if !pos_val.is_null() {
            CFRelease(pos_val);
        }
        if !size_val.is_null() {
            CFRelease(size_val);
        }
    }

    CFRelease(focused);
    let rect = cg_rect?;
    // Convert CoreGraphics global coordinates (origin at primary screen top-left, y down)
    // to AppKit global coordinates (origin at primary screen bottom-left, y up).
    Some(nami::model::ScreenRect {
        x: rect.origin.x,
        y: primary_height - (rect.origin.y + rect.size.height),
        width: rect.size.width.max(1.0),
        height: rect.size.height,
    })
}

pub fn capture_window_anchor(
    mode: nami::model::WindowPositionMode,
) -> Option<nami::model::PlacementAnchor> {
    if mode == nami::model::WindowPositionMode::LastPosition {
        return None;
    }
    unsafe {
        let mouse_pt: NSPoint = msg_send![class!(NSEvent), mouseLocation];
        let mouse = (mouse_pt.x.is_finite() && mouse_pt.y.is_finite()).then_some(
            nami::model::ScreenPoint {
                x: mouse_pt.x,
                y: mouse_pt.y,
            },
        );
        let caret = if mode == nami::model::WindowPositionMode::Cursor {
            focused_caret_rect()
        } else {
            None
        };
        let reference = caret
            .map(|rect| NSPoint::new(rect.x + rect.width / 2.0, rect.y + rect.height / 2.0))
            .or_else(|| mouse.map(|point| NSPoint::new(point.x, point.y)));
        let visible_frame = screen_visible_frame_for_point(reference)?;
        Some(nami::model::PlacementAnchor {
            visible_frame,
            mouse,
            caret,
        })
    }
}

struct DragSourceState {
    on_end: std::cell::RefCell<Option<Box<dyn FnOnce()>>>,
}

objc2::define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = DragSourceState]
    struct NamiDragSource;

    impl NamiDragSource {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn source_operation_mask(&self, _session: *mut AnyObject, _context: isize) -> usize {
            // NSDragOperationCopy = 1 so external drops copy files instead of moving them.
            1
        }

        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn session_ended(&self, _session: *mut AnyObject, _point: NSPoint, _operation: usize) {
            let callback = objc2::DefinedClass::ivars(self).on_end.borrow_mut().take();
            ACTIVE_DRAG_SOURCES.with(|sources| {
                sources
                    .borrow_mut()
                    .retain(|item| !std::ptr::eq(Retained::as_ptr(item), self as *const Self));
            });
            if let Some(callback) = callback {
                callback();
            }
        }
    }
);

thread_local! {
    static ACTIVE_DRAG_SOURCES: std::cell::RefCell<Vec<Retained<NamiDragSource>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

unsafe fn drag_image_for_file(
    path: &std::path::Path,
    preview: bool,
) -> Option<(Retained<AnyObject>, NSSize)> {
    let path_str = path.to_str()?;
    let ns_path = NSString::from_str(path_str);
    if preview {
        let alloc: Allocated<AnyObject> = msg_send![class!(NSImage), alloc];
        let image: Option<Retained<AnyObject>> =
            msg_send![alloc, initWithContentsOfFile: &*ns_path];
        if let Some(image) = image {
            let size: NSSize = msg_send![&*image, size];
            if size.width > 0.0 && size.height > 0.0 {
                let scale = (96.0 / size.width.max(size.height)).min(1.0);
                let target = NSSize::new(
                    (size.width * scale).max(24.0),
                    (size.height * scale).max(24.0),
                );
                let _: () = msg_send![&*image, setSize: target];
                return Some((image, target));
            }
        }
    }
    let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
    let icon: Option<Retained<AnyObject>> = msg_send![workspace, iconForFile: &*ns_path];
    if let Some(icon) = icon {
        let target = NSSize::new(48.0, 48.0);
        let _: () = msg_send![&*icon, setSize: target];
        return Some((icon, target));
    }
    None
}

unsafe fn drag_image_for_text() -> Option<(Retained<AnyObject>, NSSize)> {
    let workspace: *mut AnyObject = msg_send![class!(NSWorkspace), sharedWorkspace];
    let plain_type: Option<Retained<AnyObject>> =
        msg_send![class!(UTType), typeWithIdentifier: ns_string!("public.plain-text")];
    let icon: Option<Retained<AnyObject>> = if let Some(ut_type) = plain_type {
        msg_send![workspace, iconForContentType: &*ut_type]
    } else {
        None
    };
    let icon = icon?;
    let target = NSSize::new(36.0, 36.0);
    let _: () = msg_send![&*icon, setSize: target];
    Some((icon, target))
}

unsafe fn drag_trigger_event(ns_window: *mut AnyObject) -> Option<Retained<AnyObject>> {
    let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
    let current: Option<Retained<AnyObject>> = msg_send![app, currentEvent];
    if let Some(event) = current {
        let kind: usize = msg_send![&*event, type];
        let event_window: *mut AnyObject = msg_send![&*event, window];
        // NSEventTypeLeftMouseDown = 1, NSEventTypeLeftMouseDragged = 6
        if (kind == 1 || kind == 6) && event_window == ns_window {
            return Some(event);
        }
    }
    let screen_pt: NSPoint = msg_send![class!(NSEvent), mouseLocation];
    let screen_rect = NSRect::new(screen_pt, NSSize::new(0.0, 0.0));
    let win_rect: NSRect = msg_send![ns_window, convertRectFromScreen: screen_rect];
    let flags: usize = msg_send![class!(NSEvent), modifierFlags];
    let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
    let timestamp: f64 = msg_send![info, systemUptime];
    let window_number: isize = msg_send![ns_window, windowNumber];
    let event: Option<Retained<AnyObject>> = msg_send![
        class!(NSEvent),
        mouseEventWithType: 6usize,
        location: win_rect.origin,
        modifierFlags: flags,
        timestamp: timestamp,
        windowNumber: window_number,
        context: std::ptr::null::<AnyObject>(),
        eventNumber: 0isize,
        clickCount: 1isize,
        pressure: 1.0_f32
    ];
    event
}

pub(crate) fn start_clipboard_drag(
    webview: &tauri::Webview,
    item: super::DragItem,
    on_end: impl FnOnce() + Send + 'static,
    complete: impl FnOnce(Result<(), String>) + Send + 'static,
) -> Result<(), String> {
    let pressed: usize = unsafe { msg_send![class!(NSEvent), pressedMouseButtons] };
    if (pressed & 1) == 0 {
        return Err("cancelled".into());
    }
    let mut on_end = Some(Box::new(on_end) as Box<dyn FnOnce() + Send>);
    let mut complete = Some(complete);
    webview
        .with_webview(move |wv| unsafe {
            let mut finish = |res: Result<(), String>| {
                if let Some(cb) = complete.take() {
                    cb(res);
                }
            };
            let Some(marker) = objc2::MainThreadMarker::new() else {
                finish(Err("main_thread_unavailable".into()));
                return;
            };
            let view = wv.inner().cast::<AnyObject>();
            let ns_window: *mut AnyObject = msg_send![view, window];
            if view.is_null() || ns_window.is_null() {
                finish(Err("window_unavailable".into()));
                return;
            }
            let Some(event) = drag_trigger_event(ns_window) else {
                finish(Err("clipboard_drag_failed".into()));
                return;
            };
            let location_in_window: NSPoint = msg_send![&*event, locationInWindow];
            let cursor: NSPoint = msg_send![view, convertPoint: location_in_window, fromView: std::ptr::null::<AnyObject>()];

            let mut dragging_items: Vec<Retained<AnyObject>> = Vec::new();
            match &item {
                super::DragItem::Text(text) => {
                    let pb_item: Retained<AnyObject> = msg_send![class!(NSPasteboardItem), new];
                    let ns_text = NSString::from_str(text);
                    let ok: bool = msg_send![
                        &*pb_item,
                        setString: &*ns_text,
                        forType: ns_string!("public.utf8-plain-text")
                    ];
                    if !ok {
                        finish(Err("clipboard_drag_failed".into()));
                        return;
                    }
                    let alloc: Allocated<AnyObject> = msg_send![class!(NSDraggingItem), alloc];
                    let dragging_item: Retained<AnyObject> =
                        msg_send![alloc, initWithPasteboardWriter: &*pb_item];
                    if let Some((image, size)) = drag_image_for_text() {
                        let frame = NSRect::new(
                            NSPoint::new(cursor.x - size.width / 2.0, cursor.y - size.height / 2.0),
                            size,
                        );
                        let _: () =
                            msg_send![&*dragging_item, setDraggingFrame: frame, contents: &*image];
                    } else {
                        let frame = NSRect::new(cursor, NSSize::new(32.0, 32.0));
                        let _: () = msg_send![&*dragging_item, setDraggingFrame: frame, contents: std::ptr::null::<AnyObject>()];
                    }
                    dragging_items.push(dragging_item);
                }
                super::DragItem::Files {
                    paths,
                    preview_image,
                } => {
                    for (index, path) in paths.iter().enumerate() {
                        let Some(path_str) = path.to_str() else {
                            finish(Err("clipboard_file_missing".into()));
                            return;
                        };
                        let url = NSURL::fileURLWithPath(&NSString::from_str(path_str));
                        let alloc: Allocated<AnyObject> = msg_send![class!(NSDraggingItem), alloc];
                        let dragging_item: Retained<AnyObject> =
                            msg_send![alloc, initWithPasteboardWriter: &*url];
                        let is_preview = preview_image.as_deref() == Some(path.as_path());
                        let offset = (index.min(4) as f64) * 6.0;
                        if let Some((image, size)) = drag_image_for_file(path, is_preview) {
                            let frame = NSRect::new(
                                NSPoint::new(
                                    cursor.x - size.width / 2.0 + offset,
                                    cursor.y - size.height / 2.0 - offset,
                                ),
                                size,
                            );
                            let _: () = msg_send![&*dragging_item, setDraggingFrame: frame, contents: &*image];
                        } else {
                            let frame = NSRect::new(
                                NSPoint::new(cursor.x + offset, cursor.y - offset),
                                NSSize::new(48.0, 48.0),
                            );
                            let _: () = msg_send![&*dragging_item, setDraggingFrame: frame, contents: std::ptr::null::<AnyObject>()];
                        }
                        dragging_items.push(dragging_item);
                    }
                }
            }
            if dragging_items.is_empty() {
                finish(Err("invalid_argument".into()));
                return;
            }
            let end_cb = on_end.take().map(|cb| cb as Box<dyn FnOnce()>);
            let source = <NamiDragSource as objc2::MainThreadOnly>::alloc(marker).set_ivars(
                DragSourceState {
                    on_end: std::cell::RefCell::new(end_cb),
                },
            );
            let source: Retained<NamiDragSource> = msg_send![super(source), init];
            ACTIVE_DRAG_SOURCES.with(|sources| sources.borrow_mut().push(source.clone()));
            let array = NSArray::from_retained_slice(&dragging_items);
            let session: Option<Retained<AnyObject>> = msg_send![
                view,
                beginDraggingSessionWithItems: &*array,
                event: &*event,
                source: &*source
            ];
            if session.is_some() {
                finish(Ok(()));
            } else {
                ACTIVE_DRAG_SOURCES.with(|sources| {
                    sources.borrow_mut().retain(|item| {
                        !std::ptr::eq(Retained::as_ptr(item), Retained::as_ptr(&source))
                    });
                });
                finish(Err("clipboard_drag_failed".into()));
            }
        })
        .map_err(|_| "clipboard_drag_failed".into())
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
    fn clipboard_exclusion_follows_one_version_across_multiple_activations() {
        let mut focus = super::ClipboardFocus::default();
        let excluded = |bundle: &str| bundle == "excluded";
        focus.observe(Some("excluded".into()), 20);
        assert!(focus.blocks(21, excluded));
        focus.observe(Some("other".into()), 21);
        focus.observe(Some("third".into()), 21);
        assert!(focus.blocks(21, excluded));
        assert!(!focus.blocks(22, excluded));
        focus.observe(None, 22);
        assert!(!focus.blocks(22, excluded));
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

    #[test]
    fn missing_directory_reuses_macos_folder_icon() {
        let missing =
            std::env::temp_dir().join(format!("nami-missing-folder-icon-{}", uuid::Uuid::new_v4()));
        let path = missing.to_string_lossy();
        let dir_icon = super::file_icon_data_url(&path, false, true);
        let file_icon = super::file_icon_data_url(&path, false, false);
        assert!(
            dir_icon
                .as_deref()
                .is_some_and(|url| url.starts_with("data:image/png;base64,")),
            "expected PNG data URL for missing folder icon"
        );
        assert_ne!(
            dir_icon, file_icon,
            "missing folder icon must reuse folder icon rather than generic missing file icon"
        );
    }
}
