use crate::{
    model::{
        PanelFrame, WindowState, CLIPBOARD_MAX_WIDTH, CLIPBOARD_MIN_WIDTH,
        STANDALONE_TRANSLATION_DEFAULT_WIDTH,
    },
    platform, AppState,
};
use objc2::{rc::Retained, runtime::AnyObject, DefinedClass, MainThreadOnly};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::{
    cell::{Cell, RefCell},
    sync::atomic::Ordering,
};
use tauri::{
    window::{Effect, EffectState, EffectsBuilder},
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, Window,
    WindowEvent,
};

const TRANSLATION_WIDTH: f64 = STANDALONE_TRANSLATION_DEFAULT_WIDTH;
const STAGE_TRANSLATION_TAG: isize = 2;
const STAGE_CLIPBOARD_TAG: isize = 3;

#[path = "windows/native_controls.rs"]
mod native_controls;
pub use native_controls::update as update_translation_chrome;
#[path = "windows/native_resize.rs"]
mod native_resize;

unsafe extern "C" {
    static NSRunLoopCommonModes: *const objc2_foundation::NSString;
    fn objc_getAssociatedObject(object: *mut AnyObject, key: *const u8) -> *mut AnyObject;
    fn objc_setAssociatedObject(
        object: *mut AnyObject,
        key: *const u8,
        value: *mut AnyObject,
        policy: usize,
    );
}

#[cfg(feature = "ui-tests")]
#[path = "ui_tests.rs"]
mod ui_tests;
#[cfg(feature = "ui-tests")]
pub use ui_tests::start as start_ui_regression;

objc2::define_class!(
    #[unsafe(super = objc2_app_kit::NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    struct TranslationSurface;

    impl TranslationSurface {
        #[unsafe(method(wantsUpdateLayer))]
        fn wants_update_layer(&self) -> bool { true }

        #[unsafe(method(updateLayer))]
        fn update_layer(&self) {
            unsafe {
                let appearance: *mut objc2::runtime::AnyObject = objc2::msg_send![self, effectiveAppearance];
                let names = objc2_foundation::NSArray::from_slice(&[
                    objc2_foundation::ns_string!("NSAppearanceNameAqua"),
                    objc2_foundation::ns_string!("NSAppearanceNameDarkAqua"),
                ]);
                let name: Option<objc2::rc::Retained<objc2_foundation::NSString>> = objc2::msg_send![appearance, bestMatchFromAppearancesWithNames: &*names];
                let dark = name.is_some_and(|name| name.to_string().contains("Dark"));
                let workspace: *mut objc2::runtime::AnyObject = objc2::msg_send![objc2::class!(NSWorkspace), sharedWorkspace];
                let reduced: bool = objc2::msg_send![workspace, accessibilityDisplayShouldReduceTransparency];
                let (r, g, b) = if dark { (0.07875_f64, 0.07125_f64, 0.075_f64) } else { (1.0, 1.0, 1.0) };
                let color: *mut objc2::runtime::AnyObject = objc2::msg_send![objc2::class!(NSColor), colorWithSRGBRed: r, green: g, blue: b, alpha: if reduced { 1.0_f64 } else { 0.88_f64 }];
                let cg_color: *mut std::ffi::c_void = objc2::msg_send![color, CGColor];
                let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![self, layer];
                let _: () = objc2::msg_send![layer, setBackgroundColor: cg_color];
            }
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance_changed(&self) {
            unsafe { let _: () = objc2::msg_send![super(self), viewDidChangeEffectiveAppearance]; }
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(accessibilityChanged:))]
        fn accessibility_changed(&self, _: *mut objc2::runtime::AnyObject) {
            self.setNeedsDisplay(true);
        }

        #[unsafe(method(hitTest:))]
        fn hit_test(&self, _: objc2_foundation::NSPoint) -> *mut objc2::runtime::AnyObject {
            std::ptr::null_mut()
        }
    }
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StageMode {
    Explicit,
    NativeResize,
}

impl StageMode {
    fn raw(self) -> u32 {
        match self {
            Self::Explicit => 0,
            Self::NativeResize => 1,
        }
    }

    fn from_raw(value: u32) -> Self {
        match value {
            1 => Self::NativeResize,
            _ => Self::Explicit,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StageGeometry {
    mode: StageMode,
    clipboard_extent: f64,
    clipboard_width: f64,
    right_width: f64,
    clipboard_visible: bool,
    right_visible: bool,
    standalone_translation: bool,
}

impl StageGeometry {
    fn explicit(state: &WindowState, clipboard_extent: f64, right_width: f64) -> Self {
        Self {
            mode: StageMode::Explicit,
            clipboard_extent,
            clipboard_width: state.clipboard_width,
            right_width,
            clipboard_visible: state.clipboard_visible,
            right_visible: state.right_visible(),
            standalone_translation: state.standalone_translation_visible,
        }
    }

    fn frames(self, width: f64) -> StageFrames {
        let width = width.max(0.0);
        let clipboard_width = self.clipboard_width.max(0.0);
        let right_width = self.right_width.max(0.0);
        match self.mode {
            StageMode::Explicit => StageFrames {
                history: StageFrame {
                    x: self.clipboard_extent - clipboard_width,
                    width: clipboard_width,
                },
                translation: StageFrame {
                    x: self.clipboard_extent,
                    width: right_width,
                },
            },
            StageMode::NativeResize => {
                if !self.clipboard_visible {
                    StageFrames {
                        history: StageFrame {
                            x: -clipboard_width,
                            width: clipboard_width,
                        },
                        translation: StageFrame { x: 0.0, width },
                    }
                } else if !self.right_visible {
                    StageFrames {
                        history: StageFrame { x: 0.0, width },
                        translation: StageFrame {
                            x: width,
                            width: right_width,
                        },
                    }
                } else {
                    let right = (width - clipboard_width).max(0.0);
                    StageFrames {
                        history: StageFrame {
                            x: 0.0,
                            width: clipboard_width,
                        },
                        translation: StageFrame {
                            x: clipboard_width,
                            width: right,
                        },
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StageFrame {
    x: f64,
    width: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StageFrames {
    history: StageFrame,
    translation: StageFrame,
}

struct RecordsDividerState {
    app: AppHandle,
    last_x: Cell<f64>,
}

objc2::define_class!(
    #[unsafe(super = objc2_app_kit::NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = RecordsDividerState]
    struct RecordsDivider;
    impl RecordsDivider {
        #[unsafe(method(tag))]
        fn tag(&self) -> isize { 206 }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            unsafe {
                let bounds: NSRect = objc2::msg_send![self, bounds];
                let base: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
                let colors: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSMutableArray), new];
                for alpha in [0.0_f64, 0.28, 0.28, 0.28, 0.0] {
                    let color: *mut AnyObject = objc2::msg_send![base, colorWithAlphaComponent: alpha];
                    let _: () = objc2::msg_send![&*colors, addObject: color];
                }
                let positions = [0.0_f64, 0.25, 0.5, 0.75, 1.0];
                let space: *mut AnyObject = objc2::msg_send![objc2::class!(NSColorSpace), sRGBColorSpace];
                let gradient: *mut AnyObject = objc2::msg_send![objc2::class!(NSGradient), alloc];
                let gradient: *mut AnyObject = objc2::msg_send![gradient, initWithColors: &*colors, atLocations: positions.as_ptr(), colorSpace: space];
                if let Some(gradient) = Retained::from_raw(gradient) {
                    let _: () = objc2::msg_send![&*gradient, drawInRect: NSRect::new(NSPoint::new(bounds.size.width - 1.0, 0.0), NSSize::new(1.0, bounds.size.height)), angle: 90.0_f64];
                }
            }
        }
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance(&self) { unsafe { let _: () = objc2::msg_send![super(self), viewDidChangeEffectiveAppearance]; } self.setNeedsDisplay(true); }
        #[unsafe(method(acceptsFirstMouse:))]
        fn first_mouse(&self, _: *mut AnyObject) -> bool { true }
        #[unsafe(method(mouseDown:))]
        fn down(&self, _: *mut AnyObject) { unsafe { let mouse: NSPoint = objc2::msg_send![objc2::class!(NSEvent), mouseLocation]; self.ivars().last_x.set(mouse.x); } }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, _: *mut AnyObject) {
            unsafe {
                let mouse: NSPoint = objc2::msg_send![objc2::class!(NSEvent), mouseLocation];
                let delta = mouse.x - self.ivars().last_x.replace(mouse.x);
                let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().clipboard_width;
                let _ = resize_clipboard(&self.ivars().app, width + delta, false);
            }
        }
        #[unsafe(method(resetCursorRects))]
        fn cursor(&self) { unsafe { let bounds: NSRect = objc2::msg_send![self, bounds]; let cursor: *mut AnyObject = objc2::msg_send![objc2::class!(NSCursor), resizeLeftRightCursor]; let _: () = objc2::msg_send![self, addCursorRect: bounds, cursor: cursor]; } }
        #[unsafe(method(accessibilityPerformIncrement))]
        fn increment(&self) -> bool { let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().clipboard_width; resize_clipboard(&self.ivars().app, width + 24.0, false).is_ok() }
        #[unsafe(method(accessibilityPerformDecrement))]
        fn decrement(&self) -> bool { let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().clipboard_width; resize_clipboard(&self.ivars().app, width - 24.0, false).is_ok() }
    }
);

#[derive(Default)]
struct StageState {
    mode: Cell<u32>,
    clipboard_extent: Cell<f64>,
    clipboard_width: Cell<f64>,
    right_width: Cell<f64>,
    clipboard_visible: Cell<bool>,
    right_visible: Cell<bool>,
    standalone_translation: Cell<bool>,
    animating: Cell<bool>,
    geometry_version: Cell<u64>,
    applied_version: Cell<u64>,
    translation_view: Cell<usize>,
    clipboard_view: Cell<usize>,
    motion_link: RefCell<Option<Retained<AnyObject>>>,
}

struct MotionDriverState {
    app: AppHandle,
    window: Window,
    start: PanelFrame,
    target: PanelFrame,
    velocity: PanelFrame,
    revision: u64,
    ns_y: f64,
    height: f64,
    began: std::time::Instant,
}

objc2::define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = MotionDriverState]
    struct MotionDisplayLinkTarget;

    impl MotionDisplayLinkTarget {
        #[unsafe(method(step:))]
        fn step(&self, link: *mut AnyObject) {
            let driver = self.ivars();
            match advance_motion(
                &driver.app,
                &driver.window,
                driver.start,
                driver.target,
                driver.velocity,
                driver.revision,
                driver.ns_y,
                driver.height,
                driver.began.elapsed().as_secs_f64(),
            ) {
                Ok(true) => {}
                Ok(false) => {
                    if let Ok(stage) = translation_stage(&driver.window) {
                        stage.clear_motion_link(link);
                    }
                    unsafe {
                        let _: () = objc2::msg_send![link, invalidate];
                    }
                }
                Err(code) => {
                    if let Ok(stage) = translation_stage(&driver.window) {
                        stage.clear_motion_link(link);
                    }
                    unsafe {
                        let _: () = objc2::msg_send![link, invalidate];
                    }
                    let _ = driver.window.emit("layout_error", code);
                }
            }
        }
    }
);

objc2::define_class!(
    #[unsafe(super = objc2_app_kit::NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = StageState]
    struct TranslationStage;

    impl TranslationStage {
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: objc2_foundation::NSPoint) -> *mut AnyObject {
            unsafe {
                let edge: *mut AnyObject = objc2::msg_send![self, viewWithTag: 301isize];
                if !edge.is_null() {
                    let hit: *mut AnyObject = objc2::msg_send![edge, hitTest: point];
                    if !hit.is_null() { return hit; }
                }
                let state = self.ivars();
                let bounds: objc2_foundation::NSRect = objc2::msg_send![self, bounds];
                let seam = if StageMode::from_raw(state.mode.get()) == StageMode::NativeResize { state.clipboard_width.get() } else { state.clipboard_extent.get() };
                if state.clipboard_visible.get() && state.right_visible.get()
                    && point.x >= seam - 6.0 && point.x < seam + 6.0 && point.y >= native_resize::EDGE && point.y < bounds.size.height-native_resize::EDGE {
                    return objc2::msg_send![self, viewWithTag: 206isize];
                }
                objc2::msg_send![super(self), hitTest: point]
            }
        }

        #[unsafe(method(resetCursorRects))]
        fn cursors(&self) {
            unsafe {
                let state = self.ivars();
                if !state.clipboard_visible.get() || !state.right_visible.get() { return; }
                let bounds: objc2_foundation::NSRect = objc2::msg_send![self, bounds];
                let seam = if StageMode::from_raw(state.mode.get()) == StageMode::NativeResize { state.clipboard_width.get() } else { state.clipboard_extent.get() };
                let area = objc2_foundation::NSRect::new(objc2_foundation::NSPoint::new(seam-6.0,native_resize::EDGE),objc2_foundation::NSSize::new(12.0,(bounds.size.height-2.0*native_resize::EDGE).max(0.0)));
                let cursor: *mut AnyObject = objc2::msg_send![objc2::class!(NSCursor), resizeLeftRightCursor];
                let _: () = objc2::msg_send![self, addCursorRect: area, cursor: cursor];
            }
        }

        #[unsafe(method(layout))]
        fn layout(&self) {
            self.apply_frames();
            unsafe {
                let _: () = objc2::msg_send![super(self), layout];
            }
        }

        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resize_subviews(&self, old_size: objc2_foundation::NSSize) {
            unsafe {
                let _: () = objc2::msg_send![super(self), resizeSubviewsWithOldSize: old_size];
            }
            self.apply_frames();
        }

        #[unsafe(method(setStageView:forTag:))]
        fn set_stage_view(&self, view: *mut AnyObject, tag: isize) {
            match tag {
                STAGE_TRANSLATION_TAG => self.ivars().translation_view.set(view as usize),
                STAGE_CLIPBOARD_TAG => self.ivars().clipboard_view.set(view as usize),
                _ => {}
            }
        }
    }
);

impl TranslationStage {
    fn stop_motion(&self) {
        if let Some(link) = self.ivars().motion_link.borrow_mut().take() {
            unsafe {
                let _: () = objc2::msg_send![&*link, invalidate];
            }
        }
    }

    fn set_motion_link(&self, link: Retained<AnyObject>) {
        self.stop_motion();
        *self.ivars().motion_link.borrow_mut() = Some(link);
    }

    fn clear_motion_link(&self, pointer: *mut AnyObject) {
        let mut slot = self.ivars().motion_link.borrow_mut();
        if slot
            .as_ref()
            .is_some_and(|current| std::ptr::eq(Retained::as_ptr(current), pointer))
        {
            slot.take();
        }
    }

    fn apply_frames(&self) {
        let state = self.ivars();
        state.applied_version.set(state.geometry_version.get());
        let geometry = StageGeometry {
            mode: StageMode::from_raw(state.mode.get()),
            clipboard_extent: state.clipboard_extent.get(),
            clipboard_width: state.clipboard_width.get(),
            right_width: state.right_width.get(),
            clipboard_visible: state.clipboard_visible.get(),
            right_visible: state.right_visible.get(),
            standalone_translation: state.standalone_translation.get(),
        };
        unsafe {
            let bounds: objc2_foundation::NSRect = objc2::msg_send![self, bounds];
            let frames = geometry.frames(bounds.size.width);
            for (pointer, frame) in [
                (state.clipboard_view.get(), frames.history),
                (state.translation_view.get(), frames.translation),
            ] {
                if pointer == 0 {
                    continue;
                }
                let view = pointer as *mut AnyObject;
                let rect = objc2_foundation::NSRect::new(
                    objc2_foundation::NSPoint::new(bounds.origin.x + frame.x, bounds.origin.y),
                    objc2_foundation::NSSize::new(frame.width, bounds.size.height),
                );
                let current: objc2_foundation::NSRect = objc2::msg_send![view, frame];
                if current != rect {
                    let _: () = objc2::msg_send![view, setFrame: rect];
                }
            }
            let detail_shown = state.right_visible.get()
                || (geometry.mode == StageMode::Explicit
                    && bounds.size.width > geometry.clipboard_extent + 0.5);
            let divider: *mut AnyObject = objc2::msg_send![self, viewWithTag: 206isize];
            if !divider.is_null() {
                let divider_rect = NSRect::new(
                    NSPoint::new(frames.translation.x - 6.0, bounds.size.height * 0.03),
                    NSSize::new(6.0, bounds.size.height * 0.94),
                );
                let current: NSRect = objc2::msg_send![divider, frame];
                if current != divider_rect {
                    let _: () = objc2::msg_send![divider, setFrame: divider_rect];
                }
                let hidden = !state.clipboard_visible.get() || !detail_shown;
                let is_hidden: bool = objc2::msg_send![divider, isHidden];
                if is_hidden != hidden {
                    let _: () = objc2::msg_send![divider, setHidden: hidden];
                }
            }
            native_controls::layout(
                self,
                frames,
                bounds.size.height,
                state.standalone_translation.get() && detail_shown,
            );
            if !state.animating.get() {
                let window: *mut AnyObject = objc2::msg_send![self, window];
                if !window.is_null() {
                    let _: () = objc2::msg_send![window, invalidateCursorRectsForView: self];
                }
            }
        }
    }

    fn configure(&self, geometry: StageGeometry) {
        self.configure_with_motion(geometry, false);
    }

    fn configure_with_motion(&self, geometry: StageGeometry, animating: bool) {
        let state = self.ivars();
        state.mode.set(geometry.mode.raw());
        state.clipboard_extent.set(geometry.clipboard_extent);
        state.clipboard_width.set(geometry.clipboard_width);
        state.right_width.set(geometry.right_width);
        state.clipboard_visible.set(geometry.clipboard_visible);
        state.right_visible.set(geometry.right_visible);
        state
            .standalone_translation
            .set(geometry.standalone_translation);
        state.animating.set(animating);
        state
            .geometry_version
            .set(state.geometry_version.get().wrapping_add(1));
        unsafe {
            let _: () = objc2::msg_send![self, setNeedsLayout: true];
        }
    }

    fn invalidate_layout(&self) {
        let state = self.ivars();
        state
            .geometry_version
            .set(state.geometry_version.get().wrapping_add(1));
    }

    fn finish_layout(&self) {
        unsafe {
            // The stage is sized by the content view. Tests and programmatic
            // resizes may not have run an AppKit layout pass yet, so pull the
            // authoritative content bounds before laying out the pane views.
            let superview: *mut AnyObject = objc2::msg_send![self, superview];
            if !superview.is_null() {
                let bounds: objc2_foundation::NSRect = objc2::msg_send![superview, bounds];
                let current: objc2_foundation::NSRect = objc2::msg_send![self, frame];
                if current != bounds {
                    let _: () = objc2::msg_send![self, setFrame: bounds];
                }
            }
            let state = self.ivars();
            if state.applied_version.get() != state.geometry_version.get() {
                let _: () = objc2::msg_send![self, setNeedsLayout: true];
                let _: () = objc2::msg_send![self, layoutSubtreeIfNeeded];
            }
        }
    }

    fn invalidate_resize_cursors(&self) {
        unsafe {
            let window: *mut AnyObject = objc2::msg_send![self, window];
            if !window.is_null() {
                let _: () = objc2::msg_send![window, invalidateCursorRectsForView: self];
                let edge: *mut AnyObject = objc2::msg_send![self, viewWithTag: 301isize];
                if !edge.is_null() {
                    let _: () = objc2::msg_send![window, invalidateCursorRectsForView: edge];
                }
            }
        }
    }
}

static STAGE_KEY: u8 = 0;

fn translation_stage(window: &Window) -> Result<Retained<TranslationStage>, String> {
    let native = window
        .ns_window()
        .map_err(|_| "window_geometry")?
        .cast::<AnyObject>();
    let pointer = unsafe { objc_getAssociatedObject(native, &STAGE_KEY) };
    if pointer.is_null() {
        return Err("window_geometry".into());
    }
    unsafe {
        Retained::retain(pointer.cast::<TranslationStage>()).ok_or_else(|| "window_geometry".into())
    }
}

fn update_translation_stage(window: &Window, geometry: StageGeometry) -> Result<(), String> {
    let stage = translation_stage(window)?;
    stage.configure(geometry);
    stage.finish_layout();
    Ok(())
}

fn install_translation_surface(window: &Window) -> Result<(), String> {
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    let native = window
        .ns_window()
        .map_err(|_| "window_geometry")?
        .cast::<AnyObject>();
    let stage = unsafe {
        let content: *mut AnyObject = objc2::msg_send![native, contentView];
        let bounds: objc2_foundation::NSRect = objc2::msg_send![content, bounds];
        let surface = TranslationSurface::alloc(marker).set_ivars(());
        let surface: Retained<TranslationSurface> =
            objc2::msg_send![super(surface), initWithFrame: bounds];
        let _: () = objc2::msg_send![&*surface, setWantsLayer: true];
        let _: () = objc2::msg_send![&*surface, setAutoresizingMask: 18usize];
        let _: () = objc2::msg_send![&*surface, updateLayer];
        let workspace: *mut AnyObject =
            objc2::msg_send![objc2::class!(NSWorkspace), sharedWorkspace];
        let center: *mut AnyObject = objc2::msg_send![workspace, notificationCenter];
        let _: () = objc2::msg_send![center, addObserver: &*surface, selector: objc2::sel!(accessibilityChanged:), name: objc2_foundation::ns_string!("NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification"), object: std::ptr::null::<AnyObject>()];
        // The single tint surface covers newly exposed pixels before either
        // WebView repaints. History and translation share this stage above it.
        let _: () = objc2::msg_send![content, addSubview: &*surface];

        let stage = TranslationStage::alloc(marker).set_ivars(StageState::default());
        let stage: Retained<TranslationStage> =
            objc2::msg_send![super(stage), initWithFrame: bounds];
        let _: () = objc2::msg_send![&*stage, setAutoresizingMask: 18usize];
        let _: () = objc2::msg_send![content, addSubview: &*stage];
        objc_setAssociatedObject(
            native,
            &STAGE_KEY,
            Retained::as_ptr(&stage).cast_mut().cast(),
            1,
        );
        stage
    };

    let stage_pointer = Retained::as_ptr(&stage) as usize;
    {
        let view = window
            .app_handle()
            .get_webview("translation")
            .ok_or("window_geometry")?;
        view.with_webview(move |webview| unsafe {
            let view = webview.inner().cast::<AnyObject>();
            // The stage is the only frame owner; AppKit must not autoresize
            // either WebView behind its back.
            let _: () = objc2::msg_send![view, setAutoresizingMask: 0usize];
            let stage = stage_pointer as *mut TranslationStage;
            let _: () = objc2::msg_send![stage, addSubview: view, positioned: 1isize, relativeTo: std::ptr::null::<AnyObject>()];
            let _: () = objc2::msg_send![stage, setStageView: view, forTag: STAGE_TRANSLATION_TAG];
        })
        .map_err(|_| "window_geometry")?;
    }

    let clipboard = window
        .add_child(
            tauri::webview::WebviewBuilder::new("clipboard", WebviewUrl::App("index.html".into()))
                .transparent(true)
                .focused(false),
            tauri::LogicalPosition::new(-360.0, 0.0),
            LogicalSize::new(360.0, 620.0),
        )
        .map_err(|_| "window_create")?;
    clipboard
        .set_auto_resize(false)
        .map_err(|_| "window_create")?;
    clipboard
        .with_webview(move |webview| unsafe {
            let view = webview.inner().cast::<AnyObject>();
            let _: () = objc2::msg_send![view, setAutoresizingMask: 0usize];
            let _: () = objc2::msg_send![view, setFocusRingType: 1usize];
            let stage = stage_pointer as *mut TranslationStage;
            let _: () = objc2::msg_send![stage, addSubview: view];
            let _: () = objc2::msg_send![stage, setStageView: view, forTag: STAGE_CLIPBOARD_TAG];
            let _: () = objc2::msg_send![view, setHidden: true];
        })
        .map_err(|_| "window_geometry")?;

    unsafe {
        let divider = RecordsDivider::alloc(marker).set_ivars(RecordsDividerState {
            app: window.app_handle().clone(),
            last_x: Cell::new(0.0),
        });
        let divider: Retained<RecordsDivider> =
            objc2::msg_send![super(divider), initWithFrame: NSRect::ZERO];
        let _: () = objc2::msg_send![&*divider, setAccessibilityElement: true];
        let _: () = objc2::msg_send![&*divider, setAccessibilityRole: objc2_foundation::ns_string!("AXSplitter")];
        let _: () = objc2::msg_send![&*divider, setAccessibilityLabel: objc2_foundation::ns_string!("Resize clipboard panel")];
        let _: () = objc2::msg_send![&*stage, addSubview: &*divider];
    }
    native_controls::install(window.app_handle(), &stage)?;
    native_resize::install(window.app_handle(), &stage)?;
    let state = window
        .app_handle()
        .state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .clone();
    stage.configure(StageGeometry::explicit(
        &state,
        state.clipboard_extent,
        state.right_target_width().max(state.right_extent),
    ));
    stage.finish_layout();
    Ok(())
}

objc2::define_class!(
    #[unsafe(super = objc2_foundation::NSObject)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = AppHandle]
    struct TranslationResizeObserver;

    impl TranslationResizeObserver {
        #[unsafe(method(beginResize:))]
        fn begin_resize(&self, _: *mut AnyObject) {
            if let Err(code) = begin_translation_resize(self.ivars()) {
                let _ = self.ivars().emit("layout_error", code);
            }
        }

        #[unsafe(method(endResize:))]
        fn end_resize(&self, _: *mut AnyObject) {
            let app = self.ivars();
            if let Some(window) = app.get_window("translation") {
                if let Err(code) = end_translation_resize(app, &window) {
                    let _ = window.emit("layout_error", code);
                }
            }
        }
    }
);

fn observe_translation_resize(app: &AppHandle, window: &Window) -> Result<(), String> {
    static OBSERVER_KEY: u8 = 0;
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    let observer = TranslationResizeObserver::alloc(marker).set_ivars(app.clone());
    let observer: Retained<TranslationResizeObserver> =
        unsafe { objc2::msg_send![super(observer), init] };
    let native = window
        .ns_window()
        .map_err(|_| "window_geometry")?
        .cast::<AnyObject>();
    unsafe {
        let center: *mut AnyObject =
            objc2::msg_send![objc2::class!(NSNotificationCenter), defaultCenter];
        let _: () = objc2::msg_send![center, addObserver: &*observer, selector: objc2::sel!(beginResize:), name: objc2_foundation::ns_string!("NSWindowWillStartLiveResizeNotification"), object: native];
        let _: () = objc2::msg_send![center, addObserver: &*observer, selector: objc2::sel!(endResize:), name: objc2_foundation::ns_string!("NSWindowDidEndLiveResizeNotification"), object: native];
        // The window owns its observer for exactly the native window's lifetime.
        objc_setAssociatedObject(
            native,
            &OBSERVER_KEY,
            Retained::as_ptr(&observer).cast_mut().cast(),
            1,
        );
    }
    Ok(())
}

pub fn prewarm_records(app: &AppHandle) -> Result<(), String> {
    create(app, "translation").map(|_| ())
}

fn create(app: &AppHandle, label: &str) -> Result<Window, String> {
    if label == "clipboard" {
        return create(app, "translation");
    }
    if let Some(window) = app.get_window(label) {
        return Ok(window);
    }
    let (width, height) = match label {
        "translation" => (TRANSLATION_WIDTH, 620.0),
        "clipboard" => (420.0, 620.0),
        "settings" => (840.0, 640.0),
        _ => return Err("invalid_argument".into()),
    };
    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
        .title(if label == "settings" {
            "Nami · Settings"
        } else {
            "Nami"
        })
        .inner_size(width, height)
        .min_inner_size(
            if label == "settings" {
                760.0
            } else if label == "clipboard" {
                340.0
            } else {
                700.0
            },
            if label == "clipboard" { 320.0 } else { 440.0 },
        )
        .decorations(label == "settings")
        .transparent(true)
        .effects(
            EffectsBuilder::new()
                .effect(Effect::LiquidGlassRegular)
                .effect(Effect::Popover)
                .state(EffectState::Active)
                .radius(if label == "settings" { 0.0 } else { 20.0 })
                .build(),
        )
        .accept_first_mouse(true)
        .visible(false)
        .focused(false)
        .center()
        .build()
        .map_err(|_| "window_create")?;
    let window = window.as_ref().window();
    if label != "settings" {
        let native = window.ns_window().map_err(|_| "window_create")?;
        unsafe {
            let native = native.cast::<objc2::runtime::AnyObject>();
            let content: *mut objc2::runtime::AnyObject = objc2::msg_send![native, contentView];
            let _: () = objc2::msg_send![content, setWantsLayer: true];
            let layer: *mut objc2::runtime::AnyObject = objc2::msg_send![content, layer];
            // Match FinderX's outer clip around the glass, not just the glass radius.
            let _: () = objc2::msg_send![layer, setCornerRadius: 20.0_f64];
            let _: () = objc2::msg_send![layer, setMasksToBounds: true];
            let _: () = objc2::msg_send![native, invalidateShadow];
        }
    }
    if label == "translation" {
        let translation = app.get_webview("translation").ok_or("window_create")?;
        translation
            .set_auto_resize(false)
            .map_err(|_| "window_create")?;
        {
            let view = app.get_webview("translation").ok_or("window_create")?;
            view.with_webview(|webview| unsafe {
                // Internal WebViews use the controls' CSS focus indicators, not viewport outlines.
                let _: () = objc2::msg_send![webview.inner().cast::<objc2::runtime::AnyObject>(), setFocusRingType: 1usize];
            }).map_err(|_| "window_create")?;
        }
        install_translation_surface(&window)?;
        sync_translation_views(app, &window)?;
        observe_translation_resize(app, &window)?;
    }
    let handle = app.clone();
    let owned_label = label.to_string();
    window.on_window_event(move |event| match event {
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            let _ = hide(&handle, &owned_label);
        }
        WindowEvent::Resized(_) if owned_label == "translation" => {
            // Native live resize is laid out by the stage's own layout pass.
            // React/native state is synchronized once, when the drag ends.
            let resizing = handle
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .native_resizing;
            if !resizing {
                if let Some(window) = handle.get_window("translation") {
                    let _ = track_translation_resize(&handle, &window);
                }
            }
        }
        WindowEvent::Focused(false) => {
            if handle
                .state::<AppState>()
                .clipboard_dragging
                .load(Ordering::SeqCst)
            {
                return;
            }
            // A save sheet takes key status without leaving the records window.
            if handle
                .get_window(&owned_label)
                .and_then(|window| window.ns_window().ok())
                .is_some_and(|native| unsafe {
                    let sheet: *mut objc2::runtime::AnyObject =
                        objc2::msg_send![native.cast::<objc2::runtime::AnyObject>(), attachedSheet];
                    !sheet.is_null()
                })
            {
                return;
            }
            if owned_label == "translation" {
                handle.state::<AppState>().windows.lock().unwrap().blur();
                let _ = apply_translation(&handle);
            } else if owned_label == "clipboard"
                && !handle
                    .state::<AppState>()
                    .windows
                    .lock()
                    .unwrap()
                    .clipboard_pinned
            {
                if let Some(window) = handle.get_window("clipboard") {
                    // Losing focus also follows an automatic paste; keep its operation token valid.
                    let _ = crate::clipboard_runtime::visibility(&handle, false);
                    let _ = window.hide();
                }
            }
        }
        _ => {}
    });
    Ok(window)
}

pub fn emit_state(app: &AppHandle) -> Result<(), String> {
    let (state, ended) = {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        let ended = regions.sync_records_session();
        (regions.clone(), ended)
    };
    if ended {
        let app_state = app.state::<AppState>();
        app_state.clipboard_side_translation.clear();
        app_state.clipboard_translation.lock().unwrap().take();
        app_state.records_dirty.store(false, Ordering::SeqCst);
        app_state.records_navigation.lock().unwrap().take();
        crate::clipboard_runtime::visibility(app, false)?;
    }
    if let Some(view) = app.get_webview("clipboard") {
        let hidden = !state.clipboard_visible && state.clipboard_extent <= 0.0;
        view.with_webview(move |webview| unsafe {
            let _: () = objc2::msg_send![webview.inner().cast::<AnyObject>(), setHidden: hidden];
        })
        .map_err(|_| "window_geometry")?;
    }
    for label in ["translation", "clipboard", "settings"] {
        if let Some(window) = app.get_webview(label) {
            window
                .emit("window_state", &state)
                .map_err(|_| "event_emit")?;
        }
    }
    Ok(())
}

fn update_size_limits(app: &AppHandle, window: &Window) -> Result<(), String> {
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    let right_visible = state.right_visible();
    let minimum = if right_visible {
        state.right_min_width()
            + if state.clipboard_visible {
                state.clipboard_width
            } else {
                0.0
            }
    } else {
        CLIPBOARD_MIN_WIDTH
    };
    window
        .set_min_size(Some(LogicalSize::new(minimum, 440.0)))
        .map_err(|_| "window_geometry")?;
    window
        .set_max_size(if right_visible {
            None
        } else {
            Some(LogicalSize::new(CLIPBOARD_MAX_WIDTH, 100_000.0))
        })
        .map_err(|_| "window_geometry".into())
}

fn translation_window_size(window: &Window) -> Result<LogicalSize<f64>, String> {
    let native = window.ns_window().map_err(|_| "window_geometry")?;
    unsafe {
        let native = native.cast::<AnyObject>();
        let content: *mut AnyObject = objc2::msg_send![native, contentView];
        let bounds: objc2_foundation::NSRect = objc2::msg_send![content, bounds];
        // Child WebView sizes are independent of the visible native window.
        Ok(LogicalSize::new(bounds.size.width, bounds.size.height))
    }
}

fn native_translation_frame(window: &Window) -> Result<objc2_foundation::NSRect, String> {
    let native = window.ns_window().map_err(|_| "window_geometry")?;
    unsafe { Ok(objc2::msg_send![native.cast::<AnyObject>(), frame]) }
}

fn begin_translation_resize(app: &AppHandle) -> Result<(), String> {
    // Complete any interrupted extension before the native drag establishes its limits.
    let animating = app
        .state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .motion_anchor
        .is_some();
    if animating {
        layout_translation(app, false)?;
    }
    let window = app.get_window("translation").ok_or("window_unavailable")?;
    let geometry = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.native_resizing = true;
        StageGeometry {
            mode: StageMode::NativeResize,
            clipboard_extent: state.clipboard_extent,
            clipboard_width: state.clipboard_width,
            right_width: state.right_target_width(),
            clipboard_visible: state.clipboard_visible,
            right_visible: state.right_visible(),
            standalone_translation: state.standalone_translation_visible,
        }
    };
    // AppKit owns the window frame and the stage keeps both WebViews glued to
    // the correct edges for every frame of the live drag.
    update_translation_stage(&window, geometry)?;
    update_size_limits(app, &window)
}

fn end_translation_resize(app: &AppHandle, window: &Window) -> Result<(), String> {
    let size = translation_window_size(window)?;
    let clipboard_width = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if state.motion_anchor.is_some() || !state.any_visible() {
            state.native_resizing = false;
            return Ok(());
        }
        state.resize_visible_width(size.width);
        let save = state.clipboard_visible && !state.right_visible();
        let clipboard_width = save.then_some(state.clipboard_width);
        state.native_resizing = false;
        clipboard_width
    };
    if let Some(width) = clipboard_width {
        platform::save_history_width(width);
    }
    sync_translation_views(app, window)?;
    update_size_limits(app, window)?;
    invalidate_shadow(window);
    emit_state(app)
}

fn invalidate_shadow(window: &Window) {
    if let Ok(native) = window.ns_window() {
        unsafe {
            let _: () = objc2::msg_send![native.cast::<AnyObject>(), invalidateShadow];
        }
    }
}

fn track_translation_resize(app: &AppHandle, window: &Window) -> Result<(), String> {
    let size = translation_window_size(window)?;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if state.motion_anchor.is_some() || !state.any_visible() {
            return Ok(());
        }
        state.resize_visible_width(size.width);
        if state.clipboard_visible && !state.right_visible() {
            platform::save_history_width(state.clipboard_width);
        }
    }
    sync_translation_views(app, window)?;
    emit_state(app)
}

pub fn apply_translation(app: &AppHandle) -> Result<(), String> {
    layout_translation(app, true)
}

pub fn resize_clipboard(app: &AppHandle, width: f64, initialize: bool) -> Result<(), String> {
    if !width.is_finite() {
        return Err("invalid_argument".into());
    }
    let Some(window) = app.get_window("translation") else {
        return Err("window_unavailable".into());
    };
    let scale = window.scale_factor().map_err(|_| "window_geometry")?;
    let left = native_translation_frame(&window)?.origin.x;
    let mut width = width.clamp(CLIPBOARD_MIN_WIDTH, CLIPBOARD_MAX_WIDTH);
    let (frame, revision) = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if (initialize && state.clipboard_width_initialized)
            || (!initialize && state.motion_anchor.is_some())
        {
            return Ok(());
        }
        let right_width = state.right_target_width();
        if state.clipboard_visible {
            if let Some(monitor) = window.current_monitor().map_err(|_| "window_geometry")? {
                let work = monitor.work_area();
                let right = (work.position.x as f64 + work.size.width as f64) / scale;
                let available = right - left - right_width;
                width = width.min(available.max(CLIPBOARD_MIN_WIDTH));
            }
        }
        state.clipboard_width = width.round();
        state.clipboard_width_initialized = true;
        state.motion_revision += 1;
        state.motion_anchor = Some(
            left + if state.clipboard_visible {
                state.clipboard_width
            } else {
                0.0
            },
        );
        (
            PanelFrame {
                left,
                clipboard: if state.clipboard_visible {
                    state.clipboard_width
                } else {
                    0.0
                },
                right: right_width,
            },
            state.motion_revision,
        )
    };
    platform::save_history_width(width.round());
    if let Ok(stage) = translation_stage(&window) {
        stage.stop_motion();
    }
    window
        .set_min_size(Some(LogicalSize::new(1.0, 440.0)))
        .map_err(|_| "window_geometry")?;
    window
        .set_max_size::<LogicalSize<f64>>(None)
        .map_err(|_| "window_geometry")?;
    let top = window
        .outer_position()
        .map_err(|_| "window_geometry")?
        .to_logical::<f64>(scale)
        .y;
    let height = translation_window_size(&window)?.height;
    set_panel_frame(
        app,
        &window,
        revision,
        (frame, PanelFrame::default()),
        top,
        height,
        true,
    )?;
    Ok(())
}

fn sync_translation_views(app: &AppHandle, window: &Window) -> Result<(), String> {
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    let height = translation_window_size(window)?.height;
    set_native_frame(window, None, &state, height, false)
}

fn ns_origin_y(window: &Window, top: f64, height: f64) -> Result<f64, String> {
    let current_top = window
        .outer_position()
        .map_err(|_| "window_geometry")?
        .to_logical::<f64>(window.scale_factor().map_err(|_| "window_geometry")?)
        .y;
    let current = native_translation_frame(window)?;
    Ok(current.origin.y + current.size.height - height + current_top - top)
}

fn set_native_frame(
    window: &Window,
    frame: Option<(PanelFrame, f64)>,
    state: &WindowState,
    height: f64,
    finished: bool,
) -> Result<(), String> {
    let stage = translation_stage(window)?;
    let clipboard_extent = frame
        .map(|(frame, _)| frame.clipboard)
        .unwrap_or(state.clipboard_extent);
    let right_width = if state.right_visible() {
        state.right_target_width()
    } else {
        frame
            .map(|(frame, _)| frame.right)
            .unwrap_or(state.right_extent)
            .max(state.clipboard_detail_width)
    };
    stage.configure_with_motion(
        StageGeometry {
            mode: if state.native_resizing {
                StageMode::NativeResize
            } else {
                StageMode::Explicit
            },
            clipboard_extent,
            clipboard_width: state.clipboard_width,
            right_width,
            clipboard_visible: state.clipboard_visible,
            right_visible: state.right_visible(),
            standalone_translation: state.standalone_translation_visible,
        },
        frame.is_some() && !finished,
    );
    if let Some((frame, ns_y)) = frame {
        let native = window
            .ns_window()
            .map_err(|_| "window_geometry")?
            .cast::<AnyObject>();
        unsafe {
            let rect = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(frame.left, ns_y),
                objc2_foundation::NSSize::new(frame.clipboard + frame.right, height),
            );
            let _: () = objc2::msg_send![native, setFrame: rect, display: false];
        }
    }
    // Lay out both WebViews in the same turn as the window frame change. The
    // stage, not AppKit autoresizing, owns the sibling pane geometry.
    stage.finish_layout();
    if finished {
        invalidate_shadow(window);
    }
    Ok(())
}

fn set_panel_frame(
    app: &AppHandle,
    window: &Window,
    revision: u64,
    motion: (PanelFrame, PanelFrame),
    top: f64,
    height: f64,
    finished: bool,
) -> Result<bool, String> {
    let ns_y = ns_origin_y(window, top, height)?;
    set_panel_frame_ns_y(app, window, revision, motion, ns_y, height, finished)
}

fn set_panel_frame_ns_y(
    app: &AppHandle,
    window: &Window,
    revision: u64,
    motion: (PanelFrame, PanelFrame),
    ns_y: f64,
    height: f64,
    finished: bool,
) -> Result<bool, String> {
    let (frame, velocity) = motion;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if state.motion_revision != revision {
            return Ok(false);
        }
        state.clipboard_extent = frame.clipboard;
        state.right_extent = frame.right;
        state.motion_velocity = velocity;
    }
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    set_native_frame(window, Some((frame, ns_y)), &state, height, finished)?;
    if finished {
        update_size_limits(app, window)?;
        app.state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .motion_anchor = None;
        if let Ok(stage) = translation_stage(window) {
            stage.invalidate_resize_cursors();
        }
    }
    if finished {
        emit_state(app)?;
    }
    Ok(!finished)
}

#[allow(clippy::too_many_arguments)]
fn advance_motion(
    app: &AppHandle,
    window: &Window,
    start: PanelFrame,
    target: PanelFrame,
    velocity: PanelFrame,
    revision: u64,
    ns_y: f64,
    height: f64,
    elapsed: f64,
) -> Result<bool, String> {
    let (mut frame, mut speed) = start.spring(target, velocity, elapsed);
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    let anchor = frame.left + frame.clipboard;
    if frame.clipboard < 0.0 || frame.clipboard > state.clipboard_width {
        frame.clipboard = frame.clipboard.clamp(0.0, state.clipboard_width);
        speed.left += speed.clipboard;
        speed.clipboard = 0.0;
    }
    frame.left = anchor - frame.clipboard;
    let right_limit = target.right.max(start.right);
    if frame.right < 0.0 || frame.right > right_limit {
        frame.right = frame.right.clamp(0.0, right_limit);
        speed.right = 0.0;
    }
    let finished = frame.settled(target, speed);
    set_panel_frame_ns_y(
        app,
        window,
        revision,
        if finished {
            (target, PanelFrame::default())
        } else {
            (frame, speed)
        },
        ns_y,
        height,
        finished,
    )
}

fn layout_translation(app: &AppHandle, animated: bool) -> Result<(), String> {
    let Some(window) = app.get_window("translation") else {
        return emit_state(app);
    };
    if let Ok(stage) = translation_stage(&window) {
        stage.stop_motion();
    }
    let scale = window.scale_factor().map_err(|_| "window_geometry")?;
    let position = window
        .outer_position()
        .map_err(|_| "window_geometry")?
        .to_logical::<f64>(scale);
    let size = translation_window_size(&window)?;
    let monitor_work = window
        .current_monitor()
        .map_err(|_| "window_geometry")?
        .map(|monitor| {
            let work = monitor.work_area();
            (
                work.position.to_logical::<f64>(scale),
                work.size.to_logical::<f64>(scale),
            )
        });
    let (start, velocity, mut target, revision, visible) = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.motion_revision += 1;
        if !state.any_visible() {
            state.motion_anchor = None;
            state.motion_velocity = PanelFrame::default();
            drop(state);
            window.hide().map_err(|_| "window_hide")?;
            return emit_state(app);
        }
        if let Some((_, available)) = monitor_work {
            state.clamp_right_panes_to_available(available.width);
        }
        let initial_anchor = native_translation_frame(&window)?.origin.x + state.clipboard_extent;
        let anchor = *state.motion_anchor.get_or_insert(initial_anchor);
        let start = PanelFrame {
            left: native_translation_frame(&window)?.origin.x,
            clipboard: state.clipboard_extent,
            right: state.right_extent,
        };
        let target_clipboard = if state.clipboard_visible {
            state.clipboard_width
        } else {
            0.0
        };
        let target = PanelFrame {
            left: anchor - target_clipboard,
            clipboard: target_clipboard,
            right: state.right_target_width(),
        };
        (
            start,
            state.motion_velocity,
            target,
            state.motion_revision,
            window.is_visible().map_err(|_| "window_geometry")?,
        )
    };
    let mut top = position.y;
    let mut height = size.height;
    let mut ns_y_override: Option<f64> = None;
    if !visible {
        let mode = *app.state::<AppState>().window_position_mode.lock().unwrap();
        let pending_anchor = app
            .state::<AppState>()
            .pending_window_anchor
            .lock()
            .unwrap()
            .take();
        if mode != nami::model::WindowPositionMode::LastPosition {
            let anchor = pending_anchor.or_else(|| platform::capture_window_anchor(mode));
            if let Some(anchor) = anchor {
                height = height.min(anchor.visible_frame.height);
                if let Some(origin) = nami::model::compute_window_origin(
                    mode,
                    target.clipboard + target.right,
                    height,
                    anchor,
                ) {
                    target.left = origin.x;
                    ns_y_override = Some(origin.y);
                }
            }
        }
    }
    let ns_y = if let Some(ns_y) = ns_y_override {
        ns_y
    } else {
        if let Some((origin, available)) = monitor_work {
            target.left = target.left.clamp(
                origin.x,
                (origin.x + available.width - target.clipboard - target.right).max(origin.x),
            );
            height = height.min(available.height);
            top = top.clamp(
                origin.y,
                (origin.y + available.height - height).max(origin.y),
            );
        }
        let current_ns = native_translation_frame(&window)?;
        current_ns.origin.y + current_ns.size.height - height + position.y - top
    };
    window
        .set_min_size(Some(LogicalSize::new(1.0, 440.0)))
        .map_err(|_| "window_geometry")?;
    window
        .set_max_size::<LogicalSize<f64>>(None)
        .map_err(|_| "window_geometry")?;
    sync_translation_level(app, &window)?;
    if !animated
        || !visible
        || platform::reduce_motion()
        || ((start.clipboard - target.clipboard).abs() < 0.5
            && (start.right - target.right).abs() < 0.5
            && (start.left - target.left).abs() < 0.5)
    {
        set_panel_frame_ns_y(
            app,
            &window,
            revision,
            (target, PanelFrame::default()),
            ns_y,
            height,
            true,
        )?;
        return Ok(());
    }
    let refresh_rate = unsafe {
        let native = window
            .ns_window()
            .map_err(|_| "window_geometry")?
            .cast::<AnyObject>();
        let screen: *mut AnyObject = objc2::msg_send![native, screen];
        if screen.is_null() {
            60.0
        } else {
            let fps: isize = objc2::msg_send![screen, maximumFramesPerSecond];
            if fps > 0 {
                (fps as f64).clamp(30.0, 240.0)
            } else {
                60.0
            }
        }
    };
    let period = std::time::Duration::from_secs_f64(1.0 / refresh_rate);
    let now = std::time::Instant::now();
    let detail_only =
        start.clipboard == target.clipboard && (start.right - target.right).abs() >= 0.5;
    let (began, next_tick) = if detail_only {
        // Present the first detail frame before publishing selection and window changes.
        if !advance_motion(
            app,
            &window,
            start,
            target,
            velocity,
            revision,
            ns_y,
            height,
            period.as_secs_f64(),
        )? {
            return Ok(());
        }
        (now - period, now + period)
    } else {
        (now, now)
    };
    let started_display_link = if let (Some(marker), Ok(stage), Ok(native)) = (
        objc2::MainThreadMarker::new(),
        translation_stage(&window),
        window.ns_window(),
    ) {
        let native = native.cast::<AnyObject>();
        let supported: bool = unsafe {
            objc2::msg_send![native, respondsToSelector: objc2::sel!(displayLinkWithTarget:selector:)]
        };
        if supported {
            let driver = MotionDisplayLinkTarget::alloc(marker).set_ivars(MotionDriverState {
                app: app.clone(),
                window: window.clone(),
                start,
                target,
                velocity,
                revision,
                ns_y,
                height,
                began,
            });
            let driver: Retained<MotionDisplayLinkTarget> =
                unsafe { objc2::msg_send![super(driver), init] };
            let link: Option<Retained<AnyObject>> = unsafe {
                objc2::msg_send![native, displayLinkWithTarget: &*driver, selector: objc2::sel!(step:)]
            };
            if let Some(link) = link {
                unsafe {
                    let run_loop: *mut AnyObject =
                        objc2::msg_send![objc2::class!(NSRunLoop), mainRunLoop];
                    let _: () = objc2::msg_send![
                        &*link,
                        addToRunLoop: run_loop,
                        forMode: NSRunLoopCommonModes
                    ];
                }
                stage.set_motion_link(link);
                true
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };
    if !started_display_link {
        let owned = app.clone();
        tauri::async_runtime::spawn(async move {
            let app = owned;
            let mut ticks = tokio::time::interval_at(next_tick.into(), period);
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticks.tick().await;
                let owned_app = app.clone();
                let owned_window = window.clone();
                let (send, receive) = tokio::sync::oneshot::channel();
                if app
                    .run_on_main_thread(move || {
                        let _ = send.send(advance_motion(
                            &owned_app,
                            &owned_window,
                            start,
                            target,
                            velocity,
                            revision,
                            ns_y,
                            height,
                            began.elapsed().as_secs_f64(),
                        ));
                    })
                    .is_err()
                {
                    break;
                }
                match receive.await {
                    Ok(Ok(true)) => {}
                    Ok(Err(code)) => {
                        let _ = window.emit("layout_error", code);
                        break;
                    }
                    _ => break,
                }
            }
        });
    }
    emit_state(app)
}

fn sync_translation_level(app: &AppHandle, window: &Window) -> Result<(), String> {
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    let pinned = (state.standalone_translation_visible && state.translation_pinned)
        || (state.clipboard_visible && state.clipboard_pinned);
    if window.is_always_on_top().map_err(|_| "window_pin")? != pinned {
        window.set_always_on_top(pinned).map_err(|_| "window_pin")?;
    }
    Ok(())
}

pub fn update_translation_level(app: &AppHandle) -> Result<(), String> {
    if let Some(window) = app.get_window("translation") {
        sync_translation_level(app, &window)?;
    }
    emit_state(app)
}

pub fn toggle_record_details(app: &AppHandle, record_id: Option<String>) -> Result<(), String> {
    {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        if let Some(record_id) = record_id {
            if !regions.clipboard_visible
                || regions.records_editor_open
                || state.records_dirty.load(Ordering::SeqCst)
            {
                return Err("cancelled".into());
            }
            regions.preview_clipboard_record(record_id);
        } else {
            regions.toggle_clipboard_details();
        }
    }
    apply_translation(app)
}

pub fn open_clipboard_translation(app: &AppHandle, record_id: String) -> Result<(), String> {
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .open_clipboard_translation(record_id);
    apply_translation(app)
}

pub fn close_clipboard_translation(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .close_clipboard_translation();
    apply_translation(app)
}

pub fn show(app: &AppHandle, target: &str) -> Result<(), String> {
    show_entry(app, target, true)
}

pub fn resume_navigation(app: &AppHandle, target: &str) -> Result<(), String> {
    show_entry(app, target, false)
}

fn show_entry(app: &AppHandle, target: &str, capture: bool) -> Result<(), String> {
    if !["translation", "clipboard", "settings"].contains(&target) {
        return Err("invalid_argument".into());
    }
    if target != "settings" {
        let hidden = app
            .get_window("translation")
            .and_then(|window| window.is_visible().ok())
            != Some(true);
        if hidden {
            let state = app.state::<AppState>();
            let mode = *state.window_position_mode.lock().unwrap();
            if mode != nami::model::WindowPositionMode::LastPosition {
                let mut slot = state.pending_window_anchor.lock().unwrap();
                if slot.is_none() {
                    *slot = platform::capture_window_anchor(mode);
                }
            }
        }
    }
    let paste_target = (capture && target != "settings").then(platform::capture_target);
    if target != "settings"
        && crate::records_runtime::defer_navigation_with_target(app, target, paste_target)?
    {
        return Ok(());
    }
    if let Some(paste_target) = paste_target {
        let state = app.state::<AppState>();
        state.operation.fetch_add(1, Ordering::SeqCst);
        *state.target.lock().unwrap() = paste_target;
    }
    let window = create(app, target)?;
    if target != "settings" {
        {
            let state = app.state::<AppState>();
            let mut regions = state.windows.lock().unwrap();
            if target == "translation" {
                regions.summon_standalone_translation();
            } else {
                regions.show_clipboard();
                regions.records_focus += 1;
            }
        }
        apply_translation(app)?;
    }
    window.show().map_err(|_| "window_show")?;
    window.set_focus().map_err(|_| "window_focus")?;
    activate_application();
    if let Some(view) = app.get_webview(target) {
        view.set_focus().map_err(|_| "window_focus")?;
    }
    emit_state(app)
}

pub fn clipboard_has_focus(app: &AppHandle) -> bool {
    let state = app.state::<AppState>();
    let regions = state.windows.lock().unwrap();
    if !regions.clipboard_visible {
        return false;
    }
    drop(regions);
    let Some(window) = app.get_window("translation") else {
        return false;
    };
    let (Ok(native), Ok(stage)) = (window.ns_window(), translation_stage(&window)) else {
        return false;
    };
    let view = stage.ivars().clipboard_view.get() as *mut AnyObject;
    unsafe {
        let native = native.cast::<AnyObject>();
        let application: *mut AnyObject =
            objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
        let active: bool = objc2::msg_send![application, isActive];
        let key: bool = objc2::msg_send![native, isKeyWindow];
        if !active || !key || view.is_null() {
            return false;
        }
        let responder: *mut AnyObject = objc2::msg_send![native, firstResponder];
        let is_view: bool = objc2::msg_send![responder, isKindOfClass: objc2::class!(NSView)];
        is_view && objc2::msg_send![responder, isDescendantOf: view]
    }
}

pub fn focus_records_editor(app: &AppHandle) -> Result<(), String> {
    let view = app
        .get_webview("clipboard")
        .or_else(|| app.get_webview("translation"))
        .ok_or("window_unavailable")?;
    view.window().set_focus().map_err(|_| "window_focus")?;
    activate_application();
    view.set_focus().map_err(|_| "window_focus".into())
}

pub fn reveal_clipboard_failure(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>()
        .windows
        .lock()
        .unwrap()
        .show_clipboard();
    apply_translation(app)?;
    if let Some(window) = app.get_window("translation") {
        reveal_without_focus(&window)?;
    }
    Ok(())
}

pub fn hide_after_clipboard_use(app: &AppHandle) -> Result<(), String> {
    app.state::<AppState>().windows.lock().unwrap().dismiss();
    apply_translation(app)
}

fn activate_application() {
    unsafe {
        let application: *mut AnyObject =
            objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
        let _: () = objc2::msg_send![application, activateIgnoringOtherApps: true];
    }
}

pub fn reveal_without_focus(window: &Window) -> Result<(), String> {
    let native = window.ns_window().map_err(|_| "window_show")?;
    unsafe {
        let native = native.cast::<objc2::runtime::AnyObject>();
        let _: () =
            objc2::msg_send![native, orderFront: std::ptr::null::<objc2::runtime::AnyObject>()];
    }
    Ok(())
}

pub fn hide(app: &AppHandle, region: &str) -> Result<(), String> {
    match region {
        "translation" | "clipboard" => {
            if region == "clipboard" {
                app.state::<AppState>()
                    .operation
                    .fetch_add(1, Ordering::SeqCst);
            }
            app.state::<AppState>().windows.lock().unwrap().dismiss();
            apply_translation(app)
        }
        "settings" => {
            if let Some(window) = app.get_window("settings") {
                window.hide().map_err(|_| "window_hide")?;
            }
            Ok(())
        }
        _ => Err("invalid_argument".into()),
    }
}

#[cfg(test)]
mod stage_layout_tests {
    use super::*;

    fn geometry(
        mode: StageMode,
        clipboard_extent: f64,
        clipboard_width: f64,
        right_width: f64,
        clipboard_visible: bool,
        right_visible: bool,
    ) -> StageGeometry {
        StageGeometry {
            mode,
            clipboard_extent,
            clipboard_width,
            right_width,
            clipboard_visible,
            right_visible,
            standalone_translation: false,
        }
    }

    #[test]
    fn clipboard_modes_hide_standalone_translation_chrome() {
        let mut state = WindowState::default();
        state.show_clipboard();
        state.toggle_clipboard_details();
        state.open_clipboard_translation("rec-1".into());
        let attached =
            StageGeometry::explicit(&state, state.clipboard_width, state.right_target_width());
        assert!(!attached.standalone_translation);
        assert!(attached.clipboard_visible && attached.right_visible);
        assert_eq!(attached.right_width, 920.0);

        state.summon_standalone_translation();
        let standalone = StageGeometry::explicit(&state, 0.0, state.right_target_width());
        assert!(standalone.standalone_translation);
        assert!(!standalone.clipboard_visible && standalone.right_visible);
    }

    #[test]
    fn left_edge_drag_preserves_history_width_like_finderx() {
        let frames =
            geometry(StageMode::NativeResize, 360.0, 360.0, 920.0, true, true).frames(1_400.0);
        assert_eq!(
            frames.history,
            StageFrame {
                x: 0.0,
                width: 360.0
            }
        );
        assert_eq!(
            frames.translation,
            StageFrame {
                x: 360.0,
                width: 1040.0
            }
        );
        assert_eq!(frames.history.width + frames.translation.width, 1_400.0);
        assert_eq!(
            frames.translation.x + frames.translation.width,
            1_400.0,
            "translation must stay glued to the window's fixed right edge"
        );
    }

    #[test]
    fn right_edge_drag_keeps_history_fixed_at_the_left_edge() {
        let frames =
            geometry(StageMode::NativeResize, 360.0, 360.0, 920.0, true, true).frames(1_460.0);
        assert_eq!(
            frames.history,
            StageFrame {
                x: 0.0,
                width: 360.0
            }
        );
        assert_eq!(
            frames.translation,
            StageFrame {
                x: 360.0,
                width: 1_100.0
            }
        );
        assert_eq!(frames.history.width + frames.translation.width, 1_460.0);
    }

    #[test]
    fn history_only_live_resize_uses_the_whole_window() {
        let frames =
            geometry(StageMode::NativeResize, 500.0, 500.0, 920.0, true, false).frames(620.0);
        assert_eq!(
            frames.history,
            StageFrame {
                x: 0.0,
                width: 620.0
            }
        );
        assert_eq!(
            frames.translation,
            StageFrame {
                x: 620.0,
                width: 920.0
            }
        );
    }

    #[test]
    fn translation_only_live_resize_keeps_history_offscreen() {
        let frames =
            geometry(StageMode::NativeResize, 0.0, 360.0, 920.0, false, true).frames(1_080.0);
        assert_eq!(
            frames.history,
            StageFrame {
                x: -360.0,
                width: 360.0
            }
        );
        assert_eq!(
            frames.translation,
            StageFrame {
                x: 0.0,
                width: 1_080.0
            }
        );
    }

    #[test]
    fn explicit_animation_keeps_the_seam_anchored_while_history_extent_grows() {
        for extent in [0.0, 90.0, 180.0, 270.0, 360.0] {
            let frames = geometry(StageMode::Explicit, extent, 360.0, 920.0, true, true)
                .frames(extent + 920.0);
            assert_eq!(frames.history.x + frames.history.width, extent);
            assert_eq!(frames.translation.x, extent);
            assert_eq!(
                frames.history.x + frames.history.width,
                frames.translation.x
            );
            assert_eq!(
                frames.translation.x + frames.translation.width,
                extent + 920.0
            );
        }
    }
}
