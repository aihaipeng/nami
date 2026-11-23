use super::*;
use std::cell::RefCell;

const SOURCE_TAG: isize = 101;

struct ButtonState {
    app: AppHandle,
    label: RefCell<String>,
    width: Cell<f64>,
    hovered: Cell<bool>,
    tracking: RefCell<Option<Retained<AnyObject>>>,
    visible: Cell<bool>,
}

objc2::define_class!(
    #[unsafe(super = objc2_app_kit::NSButton)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ButtonState]
    struct ChromeButton;

    impl ChromeButton {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }

        #[unsafe(method(acceptsFirstMouse:))]
        fn first_mouse(&self, _: *mut AnyObject) -> bool { true }

        #[unsafe(method(pressed:))]
        fn pressed(&self, _: *mut AnyObject) {
            if let Some(view) = self.ivars().app.get_webview("translation") {
                if view.emit("translation_chrome_action", "source").is_err() {
                    let _ = view.emit("layout_error", "native_control_event");
                }
            }
        }

        #[unsafe(method(drawRect:))]
        fn draw(&self, _: objc2_foundation::NSRect) {
            unsafe { draw_button(self); }
        }

        #[unsafe(method(updateTrackingAreas))]
        fn tracking_areas(&self) {
            unsafe {
                let _: () = objc2::msg_send![super(self), updateTrackingAreas];
                if let Some(area) = self.ivars().tracking.borrow_mut().take() {
                    let _: () = objc2::msg_send![self, removeTrackingArea: &*area];
                }
                let bounds: objc2_foundation::NSRect = objc2::msg_send![self, bounds];
                let allocated: *mut AnyObject = objc2::msg_send![objc2::class!(NSTrackingArea), alloc];
                let area: *mut AnyObject = objc2::msg_send![allocated, initWithRect: bounds, options: 641usize, owner: self, userInfo: std::ptr::null::<AnyObject>()];
                let area = Retained::from_raw(area).expect("NSTrackingArea initialization");
                let _: () = objc2::msg_send![self, addTrackingArea: &*area];
                *self.ivars().tracking.borrow_mut() = Some(area);
            }
        }

        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _: *mut AnyObject) {
            self.ivars().hovered.set(true);
            objc2_app_kit::NSView::setNeedsDisplay(self, true);
        }

        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: *mut AnyObject) {
            self.ivars().hovered.set(false);
            objc2_app_kit::NSView::setNeedsDisplay(self, true);
        }

        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance(&self) {
            unsafe { let _: () = objc2::msg_send![super(self), viewDidChangeEffectiveAppearance]; }
            objc2_app_kit::NSView::setNeedsDisplay(self, true);
        }
    }
);

unsafe fn attributes(alpha: f64, size: f64) -> Retained<AnyObject> {
    let inter = objc2_foundation::ns_string!("Inter");
    let named: Option<Retained<AnyObject>> =
        objc2::msg_send![objc2::class!(NSFont), fontWithName: inter, size: size];
    let font =
        named.unwrap_or_else(|| objc2::msg_send![objc2::class!(NSFont), systemFontOfSize: size]);
    let label: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
    let color: *mut AnyObject = objc2::msg_send![label, colorWithAlphaComponent: alpha];
    let attrs: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSMutableDictionary), new];
    let _: () = objc2::msg_send![&*attrs, setObject: &*font, forKey: objc2_foundation::ns_string!("NSFont")];
    let _: () = objc2::msg_send![&*attrs, setObject: color, forKey: objc2_foundation::ns_string!("NSColor")];
    attrs
}

unsafe fn text_size(text: &str, size: f64) -> objc2_foundation::NSSize {
    let text = objc2_foundation::NSString::from_str(text);
    let attrs = attributes(1.0, size);
    objc2::msg_send![&*text, sizeWithAttributes: &*attrs]
}

unsafe fn fill_round(rect: objc2_foundation::NSRect, radius: f64, alpha: f64) {
    let color: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
    let color: *mut AnyObject = objc2::msg_send![color, colorWithAlphaComponent: alpha];
    let _: () = objc2::msg_send![color, set];
    let path: *mut AnyObject = objc2::msg_send![objc2::class!(NSBezierPath), bezierPathWithRoundedRect: rect, xRadius: radius, yRadius: radius];
    let _: () = objc2::msg_send![path, fill];
}

unsafe fn draw_text(text: &str, x: f64, y: f64, size: f64, alpha: f64) {
    let text = objc2_foundation::NSString::from_str(text);
    let attrs = attributes(alpha, size);
    let _: () = objc2::msg_send![&*text, drawAtPoint: objc2_foundation::NSPoint::new(x, y), withAttributes: &*attrs];
}

unsafe fn draw_button(button: &ChromeButton) {
    let bounds: objc2_foundation::NSRect = objc2::msg_send![button, bounds];
    let enabled: bool = objc2::msg_send![button, isEnabled];
    let highlighted: bool = objc2::msg_send![button, isHighlighted];
    let alpha = if enabled { 1.0 } else { 0.45 };
    if enabled && (highlighted || button.ivars().hovered.get()) {
        fill_round(bounds, 14.0, if highlighted { 0.1 } else { 0.06 });
    }
    let label = button.ivars().label.borrow();
    let text_height = text_size(&label, 13.0).height;
    let _: () = objc2::msg_send![objc2::class!(NSGraphicsContext), saveGraphicsState];
    let clip = objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(12.0, 0.0),
        objc2_foundation::NSSize::new((bounds.size.width - 42.0).max(0.0), 32.0),
    );
    let path: *mut AnyObject =
        objc2::msg_send![objc2::class!(NSBezierPath), bezierPathWithRect: clip];
    let _: () = objc2::msg_send![path, addClip];
    draw_text(&label, 12.0, (32.0 - text_height) / 2.0, 13.0, alpha);
    let _: () = objc2::msg_send![objc2::class!(NSGraphicsContext), restoreGraphicsState];
    let path: *mut AnyObject = objc2::msg_send![objc2::class!(NSBezierPath), bezierPath];
    let x = bounds.size.width - 17.0;
    let _: () = objc2::msg_send![path, moveToPoint: objc2_foundation::NSPoint::new(x - 3.0, 14.5)];
    let _: () = objc2::msg_send![path, lineToPoint: objc2_foundation::NSPoint::new(x, 17.5)];
    let _: () = objc2::msg_send![path, lineToPoint: objc2_foundation::NSPoint::new(x + 3.0, 14.5)];
    let color: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), secondaryLabelColor];
    let _: () = objc2::msg_send![color, setStroke];
    let _: () = objc2::msg_send![path, setLineWidth: 1.0_f64];
    let _: () = objc2::msg_send![path, stroke];
}

fn button(stage: &TranslationStage) -> Option<Retained<ChromeButton>> {
    unsafe {
        let pointer: *mut ChromeButton = objc2::msg_send![stage, viewWithTag: SOURCE_TAG];
        Retained::retain(pointer)
    }
}

pub(super) fn install(app: &AppHandle, stage: &TranslationStage) -> Result<(), String> {
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    let allocated = ChromeButton::alloc(marker).set_ivars(ButtonState {
        app: app.clone(),
        label: RefCell::new("Auto-Detect".into()),
        width: Cell::new(142.0),
        hovered: Cell::new(false),
        tracking: RefCell::new(None),
        visible: Cell::new(false),
    });
    let button: Retained<ChromeButton> = unsafe {
        objc2::msg_send![super(allocated), initWithFrame: objc2_foundation::NSRect::ZERO]
    };
    unsafe {
        let _: () = objc2::msg_send![&*button, setTag: SOURCE_TAG];
        let _: () = objc2::msg_send![&*button, setBordered: false];
        let _: () = objc2::msg_send![&*button, setButtonType: 0usize];
        let _: () = objc2::msg_send![&*button, setTarget: &*button];
        let _: () = objc2::msg_send![&*button, setAction: objc2::sel!(pressed:)];
        let _: () = objc2::msg_send![&*button, setAutoresizingMask: 0usize];
        let _: () = objc2::msg_send![&*button, setHidden: true];
        let _: () = objc2::msg_send![stage, addSubview: &*button, positioned: 1isize, relativeTo: std::ptr::null::<AnyObject>()];
    }
    Ok(())
}

pub(super) fn layout(
    stage: &TranslationStage,
    frames: StageFrames,
    height: f64,
    translation_visible: bool,
) {
    let translation = frames.translation;
    let Some(source) = button(stage) else {
        return;
    };
    let width = source.ivars().width.get();
    let hidden = !translation_visible || translation.width < 1.0 || !source.ivars().visible.get();
    let rect = objc2_foundation::NSRect::new(
        objc2_foundation::NSPoint::new(
            translation.x + translation.width - 24.0 - width,
            height - 40.0,
        ),
        objc2_foundation::NSSize::new(width, 32.0),
    );
    unsafe {
        let current: objc2_foundation::NSRect = objc2::msg_send![&*source, frame];
        if current != rect {
            let _: () = objc2::msg_send![&*source, setFrame: rect];
        }
        let is_hidden: bool = objc2::msg_send![&*source, isHidden];
        if is_hidden != hidden {
            let _: () = objc2::msg_send![&*source, setHidden: hidden];
        }
    }
}

pub fn update(app: &AppHandle, source_label: String, source_width: f64) -> Result<(), String> {
    let window = app.get_window("translation").ok_or("window_unavailable")?;
    let stage = translation_stage(&window)?;
    let button = button(&stage).ok_or("window_geometry")?;
    let width = source_width.max(unsafe { text_size(&source_label, 13.0).width.ceil() + 42.0 });
    button.ivars().width.set(width);
    button.ivars().visible.set(true);
    let title = objc2_foundation::NSString::from_str(&source_label);
    let accessible =
        objc2_foundation::NSString::from_str(&format!("Source language: {source_label}"));
    *button.ivars().label.borrow_mut() = source_label;
    unsafe {
        let _: () = objc2::msg_send![&*button, setTitle: &*title];
        let _: () = objc2::msg_send![&*button, setAccessibilityLabel: &*accessible];
        objc2_app_kit::NSView::setNeedsDisplay(&button, true);
    }
    stage.invalidate_layout();
    stage.finish_layout();
    Ok(())
}
