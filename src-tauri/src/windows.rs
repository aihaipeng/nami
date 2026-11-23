use crate::{
    model::{PanelFrame, RecordView, WindowState},
    platform, AppState,
};
use objc2::{rc::Retained, runtime::AnyObject, DefinedClass, MainThreadOnly};
use objc2_foundation::{NSPoint, NSRect, NSSize};
use std::{cell::Cell, sync::atomic::Ordering};
use tauri::{
    window::{Effect, EffectState, EffectsBuilder},
    AppHandle, Emitter, LogicalSize, Manager, WebviewUrl, WebviewWindowBuilder, Window,
    WindowEvent,
};

const TRANSLATION_WIDTH: f64 = 920.0;
const HISTORY_MIN_WIDTH: f64 = 280.0;
const HISTORY_MAX_WIDTH: f64 = 720.0;
const STAGE_TRANSLATION_TAG: isize = 2;
const STAGE_CLIPBOARD_TAG: isize = 3;

#[path = "windows/native_controls.rs"]
mod native_controls;
pub use native_controls::update as update_translation_chrome;
#[path = "windows/native_resize.rs"]
mod native_resize;

#[cfg(feature = "ui-tests")]
pub(crate) use crate::history_runtime::history_add;
pub(crate) use crate::history_runtime::{history_record, select_native_history};

unsafe extern "C" {
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
    history_extent: f64,
    history_width: f64,
    translation_width: f64,
    history_visible: bool,
    translation_visible: bool,
    translation_content: bool,
}

impl StageGeometry {
    fn explicit(state: &WindowState, history_extent: f64) -> Self {
        Self {
            mode: StageMode::Explicit,
            history_extent,
            history_width: state.history_width,
            translation_width: state.translation_width,
            history_visible: state.history_visible,
            translation_visible: state.translation_visible,
            translation_content: state.record_view == RecordView::Translation,
        }
    }

    fn frames(self, width: f64) -> StageFrames {
        let width = width.max(0.0);
        let history_width = self.history_width.max(0.0);
        let translation_width = self.translation_width.max(0.0);
        match self.mode {
            StageMode::Explicit => StageFrames {
                // history_extent is the animated seam between the panes.
                history: StageFrame {
                    x: self.history_extent - history_width,
                    width: history_width,
                },
                translation: StageFrame {
                    x: self.history_extent,
                    width: translation_width,
                },
            },
            StageMode::NativeResize => {
                if !self.history_visible {
                    StageFrames {
                        history: StageFrame {
                            x: -history_width,
                            width: history_width,
                        },
                        translation: StageFrame { x: 0.0, width },
                    }
                } else if !self.translation_visible {
                    StageFrames {
                        history: StageFrame { x: 0.0, width },
                        translation: StageFrame {
                            x: width,
                            width: translation_width,
                        },
                    }
                } else {
                    // The history keeps its width against the fixed left edge;
                    // translation absorbs the delta.
                    let translation = (width - history_width).max(0.0);
                    StageFrames {
                        history: StageFrame {
                            x: 0.0,
                            width: history_width,
                        },
                        translation: StageFrame {
                            x: history_width,
                            width: translation,
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
                let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().history_width;
                let _ = resize_history(&self.ivars().app, width + delta, false);
            }
        }
        #[unsafe(method(resetCursorRects))]
        fn cursor(&self) { unsafe { let bounds: NSRect = objc2::msg_send![self, bounds]; let cursor: *mut AnyObject = objc2::msg_send![objc2::class!(NSCursor), resizeLeftRightCursor]; let _: () = objc2::msg_send![self, addCursorRect: bounds, cursor: cursor]; } }
        #[unsafe(method(accessibilityPerformIncrement))]
        fn increment(&self) -> bool { let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().history_width; resize_history(&self.ivars().app, width + 24.0, false).is_ok() }
        #[unsafe(method(accessibilityPerformDecrement))]
        fn decrement(&self) -> bool { let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().history_width; resize_history(&self.ivars().app, width - 24.0, false).is_ok() }
    }
);

#[derive(Default)]
struct StageState {
    mode: Cell<u32>,
    history_extent: Cell<f64>,
    history_width: Cell<f64>,
    translation_width: Cell<f64>,
    history_visible: Cell<bool>,
    translation_visible: Cell<bool>,
    translation_content: Cell<bool>,
    translation_view: Cell<usize>,
    clipboard_view: Cell<usize>,
}

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
                let seam = if StageMode::from_raw(state.mode.get()) == StageMode::NativeResize { state.history_width.get() } else { state.history_extent.get() };
                if state.history_visible.get() && state.translation_visible.get()
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
                if !state.history_visible.get() || !state.translation_visible.get() { return; }
                let bounds: objc2_foundation::NSRect = objc2::msg_send![self, bounds];
                let seam = if StageMode::from_raw(state.mode.get()) == StageMode::NativeResize { state.history_width.get() } else { state.history_extent.get() };
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
    fn apply_frames(&self) {
        let state = self.ivars();
        let geometry = StageGeometry {
            mode: StageMode::from_raw(state.mode.get()),
            history_extent: state.history_extent.get(),
            history_width: state.history_width.get(),
            translation_width: state.translation_width.get(),
            history_visible: state.history_visible.get(),
            translation_visible: state.translation_visible.get(),
            translation_content: state.translation_content.get(),
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
            let divider: *mut AnyObject = objc2::msg_send![self, viewWithTag: 206isize];
            if !divider.is_null() {
                let _: () = objc2::msg_send![divider, setFrame: NSRect::new(NSPoint::new(frames.translation.x - 6.0, bounds.size.height * 0.03), NSSize::new(6.0, bounds.size.height * 0.94))];
                let _: () = objc2::msg_send![divider, setHidden: !state.history_visible.get() || !state.translation_visible.get()];
            }
            native_controls::layout(
                self,
                frames,
                bounds.size.height,
                state.translation_content.get()
                    && (state.translation_visible.get()
                        || (geometry.mode == StageMode::Explicit
                            && bounds.size.width > geometry.history_extent + 0.5)),
            );
            let window: *mut AnyObject = objc2::msg_send![self, window];
            if !window.is_null() {
                let _: () = objc2::msg_send![window, invalidateCursorRectsForView: self];
            }
        }
    }

    fn configure(&self, geometry: StageGeometry) {
        let state = self.ivars();
        state.mode.set(geometry.mode.raw());
        state.history_extent.set(geometry.history_extent);
        state.history_width.set(geometry.history_width);
        state.translation_width.set(geometry.translation_width);
        state.history_visible.set(geometry.history_visible);
        state.translation_visible.set(geometry.translation_visible);
        state.translation_content.set(geometry.translation_content);
        unsafe {
            let _: () = objc2::msg_send![self, setNeedsLayout: true];
        }
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
            let _: () = objc2::msg_send![self, setNeedsLayout: true];
            let _: () = objc2::msg_send![self, layoutSubtreeIfNeeded];
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
        let _: () = objc2::msg_send![&*divider, setAccessibilityLabel: objc2_foundation::ns_string!("Resize translation history")];
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
    stage.configure(StageGeometry::explicit(&state, state.history_extent));
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
        if ended {
            regions.record_view = RecordView::Clipboard;
        }
        (regions.clone(), ended)
    };
    if ended {
        app.state::<AppState>()
            .records_dirty
            .store(false, Ordering::SeqCst);
        app.state::<AppState>()
            .records_navigation
            .lock()
            .unwrap()
            .take();
        crate::clipboard_runtime::visibility(app, false)?;
    }
    if !state.translation_visible {
        let state = app.state::<AppState>();
        if state.history.lock().unwrap().selected_id().is_some() {
            state.history.lock().unwrap().clear_visual_selection();
            crate::history_runtime::publish_history(app)?;
        }
    }
    if let Some(view) = app.get_webview("clipboard") {
        let hidden = !state.history_visible && state.history_extent <= 0.0;
        view.with_webview(move |webview| unsafe {
            let _: () = objc2::msg_send![webview.inner().cast::<AnyObject>(), setHidden: hidden];
        })
        .map_err(|_| "window_geometry")?;
    }
    for label in ["translation", "history", "clipboard", "settings"] {
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
    let minimum = if state.translation_visible {
        state.translation_min_width()
            + if state.history_visible {
                state.history_width
            } else {
                0.0
            }
    } else {
        HISTORY_MIN_WIDTH
    };
    window
        .set_min_size(Some(LogicalSize::new(minimum, 440.0)))
        .map_err(|_| "window_geometry")?;
    window
        .set_max_size(if state.translation_visible {
            None
        } else {
            Some(LogicalSize::new(HISTORY_MAX_WIDTH, 100_000.0))
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
            history_extent: state.history_extent,
            history_width: state.history_width,
            translation_width: state.translation_width,
            history_visible: state.history_visible,
            translation_visible: state.translation_visible,
            translation_content: state.record_view == RecordView::Translation,
        }
    };
    // AppKit owns the window frame and the stage keeps both WebViews glued to
    // the correct edges for every frame of the live drag.
    update_translation_stage(&window, geometry)?;
    update_size_limits(app, &window)
}

fn end_translation_resize(app: &AppHandle, window: &Window) -> Result<(), String> {
    let size = translation_window_size(window)?;
    let history_width = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if state.motion_anchor.is_some() || (!state.translation_visible && !state.history_visible) {
            state.native_resizing = false;
            return Ok(());
        }
        state.resize_visible_width(size.width);
        let save = state.history_visible && !state.translation_visible;
        let history_width = save.then_some(state.history_width);
        state.native_resizing = false;
        history_width
    };
    if let Some(width) = history_width {
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
        if state.motion_anchor.is_some() || (!state.translation_visible && !state.history_visible) {
            return Ok(());
        }
        state.resize_visible_width(size.width);
        if state.history_visible && !state.translation_visible {
            platform::save_history_width(state.history_width);
        }
    }
    sync_translation_views(app, window)?;
    emit_state(app)
}

pub fn apply_translation(app: &AppHandle) -> Result<(), String> {
    layout_translation(app, true)
}

pub fn resize_history(app: &AppHandle, width: f64, initialize: bool) -> Result<(), String> {
    if !width.is_finite() {
        return Err("invalid_argument".into());
    }
    let Some(window) = app.get_window("translation") else {
        return Err("window_unavailable".into());
    };
    let scale = window.scale_factor().map_err(|_| "window_geometry")?;
    let left = native_translation_frame(&window)?.origin.x;
    let mut width = width.clamp(280.0, 720.0);
    let (frame, revision) = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if (initialize && state.history_width_initialized)
            || (!initialize && state.motion_anchor.is_some())
        {
            return Ok(());
        }
        if state.history_visible {
            if let Some(monitor) = window.current_monitor().map_err(|_| "window_geometry")? {
                let work = monitor.work_area();
                let right = (work.position.x as f64 + work.size.width as f64) / scale;
                let translation = if state.translation_visible {
                    state.translation_width
                } else {
                    0.0
                };
                let available = right - left - translation;
                width = width.min(available.max(280.0));
            }
        }
        state.history_width = width.round();
        state.history_width_initialized = true;
        state.motion_revision += 1;
        state.motion_anchor = Some(
            left + if state.history_visible {
                state.history_width
            } else {
                0.0
            },
        );
        (
            PanelFrame {
                left,
                history: if state.history_visible {
                    state.history_width
                } else {
                    0.0
                },
                translation: if state.translation_visible {
                    state.translation_width
                } else {
                    0.0
                },
            },
            state.motion_revision,
        )
    };
    platform::save_history_width(width.round());
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

fn set_native_frame(
    window: &Window,
    frame: Option<(PanelFrame, f64)>,
    state: &WindowState,
    height: f64,
    finished: bool,
) -> Result<(), String> {
    let stage = translation_stage(window)?;
    let history_extent = frame
        .map(|(frame, _)| frame.history)
        .unwrap_or(state.history_extent);
    stage.configure(StageGeometry {
        mode: if state.native_resizing {
            StageMode::NativeResize
        } else {
            StageMode::Explicit
        },
        history_extent,
        history_width: state.history_width,
        translation_width: state.translation_width,
        history_visible: state.history_visible,
        translation_visible: state.translation_visible,
        translation_content: state.record_view == RecordView::Translation,
    });
    if let Some((frame, top)) = frame {
        let current_top = window
            .outer_position()
            .map_err(|_| "window_geometry")?
            .to_logical::<f64>(window.scale_factor().map_err(|_| "window_geometry")?)
            .y;
        let native = window
            .ns_window()
            .map_err(|_| "window_geometry")?
            .cast::<AnyObject>();
        unsafe {
            let current: objc2_foundation::NSRect = objc2::msg_send![native, frame];
            let rect = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(
                    frame.left,
                    current.origin.y + current.size.height - height + current_top - top,
                ),
                objc2_foundation::NSSize::new(frame.history + frame.translation, height),
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
    let (frame, velocity) = motion;
    {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        if state.motion_revision != revision {
            return Ok(false);
        }
        state.history_extent = frame.history;
        state.translation_extent = frame.translation;
        state.motion_velocity = velocity;
    }
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    set_native_frame(window, Some((frame, top)), &state, height, finished)?;
    if finished {
        update_size_limits(app, window)?;
        app.state::<AppState>()
            .windows
            .lock()
            .unwrap()
            .motion_anchor = None;
    }
    if finished {
        emit_state(app)?;
    }
    Ok(!finished)
}

fn layout_translation(app: &AppHandle, animated: bool) -> Result<(), String> {
    let Some(window) = app.get_window("translation") else {
        return emit_state(app);
    };
    let scale = window.scale_factor().map_err(|_| "window_geometry")?;
    let position = window
        .outer_position()
        .map_err(|_| "window_geometry")?
        .to_logical::<f64>(scale);
    let size = translation_window_size(&window)?;
    let (start, velocity, mut target, revision, visible) = {
        let state = app.state::<AppState>();
        let mut state = state.windows.lock().unwrap();
        state.motion_revision += 1;
        if !state.translation_visible && !state.history_visible {
            state.motion_anchor = None;
            state.motion_velocity = PanelFrame::default();
            drop(state);
            window.hide().map_err(|_| "window_hide")?;
            return emit_state(app);
        }
        let initial_anchor = native_translation_frame(&window)?.origin.x + state.history_extent;
        let anchor = *state.motion_anchor.get_or_insert(initial_anchor);
        let start = PanelFrame {
            left: native_translation_frame(&window)?.origin.x,
            history: state.history_extent,
            translation: state.translation_extent,
        };
        let target = PanelFrame {
            left: anchor
                - if state.history_visible {
                    state.history_width
                } else {
                    0.0
                },
            history: if state.history_visible {
                state.history_width
            } else {
                0.0
            },
            translation: if state.translation_visible {
                state.translation_width
            } else {
                0.0
            },
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
    if let Some(monitor) = window.current_monitor().map_err(|_| "window_geometry")? {
        let work = monitor.work_area();
        let origin = work.position.to_logical::<f64>(scale);
        let available = work.size.to_logical::<f64>(scale);
        target.left = target.left.clamp(
            origin.x,
            (origin.x + available.width - target.history - target.translation).max(origin.x),
        );
        height = height.min(available.height);
        top = top.clamp(
            origin.y,
            (origin.y + available.height - height).max(origin.y),
        );
    }
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
        || ((start.history - target.history).abs() < 0.5
            && (start.translation - target.translation).abs() < 0.5
            && (start.left - target.left).abs() < 0.5)
    {
        set_panel_frame(
            app,
            &window,
            revision,
            (target, PanelFrame::default()),
            top,
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
    let advance = move |app: &AppHandle, window: &Window, elapsed: f64| {
        let (mut frame, mut speed) = start.spring(target, velocity, elapsed);
        let state = app.state::<AppState>().windows.lock().unwrap().clone();
        let anchor = frame.left + frame.history;
        if frame.history < 0.0 || frame.history > state.history_width {
            frame.history = frame.history.clamp(0.0, state.history_width);
            speed.left += speed.history;
            speed.history = 0.0;
        }
        frame.left = anchor - frame.history;
        if frame.translation < 0.0 || frame.translation > state.translation_width {
            frame.translation = frame.translation.clamp(0.0, state.translation_width);
            speed.translation = 0.0;
        }
        let finished = frame.settled(target, speed);
        set_panel_frame(
            app,
            window,
            revision,
            if finished {
                (target, PanelFrame::default())
            } else {
                (frame, speed)
            },
            top,
            height,
            finished,
        )
    };
    let period = std::time::Duration::from_secs_f64(1.0 / refresh_rate);
    let now = std::time::Instant::now();
    let detail_only =
        start.history == target.history && (start.translation - target.translation).abs() >= 0.5;
    let (began, next_tick) = if detail_only {
        // Present the first detail frame before publishing selection and window changes.
        if !advance(app, &window, period.as_secs_f64())? {
            return Ok(());
        }
        (now - period, now + period)
    } else {
        (now, now)
    };
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
                    let _ = send.send(advance(
                        &owned_app,
                        &owned_window,
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
    emit_state(app)
}

fn sync_translation_level(app: &AppHandle, window: &Window) -> Result<(), String> {
    let state = app.state::<AppState>().windows.lock().unwrap().clone();
    let pinned = (state.translation_visible && state.translation_pinned)
        || (state.history_visible && state.history_pinned);
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

pub fn set_record_view(app: &AppHandle, view: &str) -> Result<(), String> {
    let view = match view {
        "clipboard" => RecordView::Clipboard,
        "translation" => RecordView::Translation,
        _ => return Err("invalid_argument".into()),
    };
    let current = app.state::<AppState>().windows.lock().unwrap().record_view;
    if current == view {
        return Ok(());
    }
    if crate::records_runtime::defer_navigation(
        app,
        if view == RecordView::Clipboard {
            "view:clipboard"
        } else {
            "view:translation"
        },
    )? {
        return Ok(());
    }
    {
        let state = app.state::<AppState>();
        let mut windows = state.windows.lock().unwrap();
        windows.set_record_view(view);
        windows.records_focus += 1;
    }
    apply_translation(app)?;
    if let Some(view) = app.get_webview("clipboard") {
        view.set_focus().map_err(|_| "window_focus")?;
    }
    Ok(())
}

pub fn toggle_record_details(app: &AppHandle, record_id: Option<String>) -> Result<(), String> {
    let restore_selection = {
        let state = app.state::<AppState>();
        let mut regions = state.windows.lock().unwrap();
        if let Some(record_id) = record_id {
            if regions.record_view != RecordView::Clipboard
                || !regions.history_visible
                || regions.records_editor_open
                || state
                    .records_dirty
                    .load(std::sync::atomic::Ordering::SeqCst)
            {
                return Err("cancelled".into());
            }
            regions.preview_clipboard_record(record_id);
        } else {
            regions.toggle_record_details();
        }
        let restore_selection =
            regions.translation_visible && regions.record_view == RecordView::Translation;
        if restore_selection {
            state.history.lock().unwrap().restore_visual_selection();
        }
        restore_selection
    };
    if restore_selection {
        crate::history_runtime::publish_history(app)?;
    }
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
                regions.summon_translation();
            } else {
                regions.set_record_view(RecordView::Clipboard);
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
        .set_record_view(RecordView::Clipboard);
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
        "translation" | "history" | "clipboard" => {
            if region == "clipboard" {
                app.state::<AppState>()
                    .operation
                    .fetch_add(1, Ordering::SeqCst);
            }
            {
                let state = app.state::<AppState>();
                let mut regions = state.windows.lock().unwrap();
                if region == "history" {
                    regions.history_visible = false;
                } else {
                    regions.dismiss();
                }
            }
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
        history_extent: f64,
        history_width: f64,
        translation_width: f64,
        history_visible: bool,
        translation_visible: bool,
    ) -> StageGeometry {
        StageGeometry {
            mode,
            history_extent,
            history_width,
            translation_width,
            history_visible,
            translation_visible,
            translation_content: true,
        }
    }

    #[test]
    fn clipboard_details_keep_the_same_stage_frames_without_translation_controls() {
        let mut state = WindowState {
            history_visible: true,
            history_extent: 360.0,
            ..Default::default()
        };
        let clipboard = StageGeometry::explicit(&state, state.history_extent);
        assert!(!clipboard.translation_content);
        state.record_view = RecordView::Translation;
        let translation = StageGeometry::explicit(&state, state.history_extent);
        assert!(translation.translation_content);
        assert_eq!(clipboard.frames(1280.0), translation.frames(1280.0));
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
