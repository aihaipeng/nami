use super::*;
use std::cell::RefCell;

const SOURCE_TAG: isize = 101;
const TRANSLATE_TAG: isize = 102;
const ACTIONS_TAG: isize = 103;

struct ButtonState {
    app: AppHandle,
    action: &'static str,
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
                if view.emit("translation_chrome_action", self.ivars().action).is_err() {
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

unsafe fn fill_round(rect: objc2_foundation::NSRect, radius: f64, alpha: f64, stroke: bool) {
    let color: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
    let color: *mut AnyObject = objc2::msg_send![color, colorWithAlphaComponent: alpha];
    let _: () = objc2::msg_send![color, set];
    let path: *mut AnyObject = objc2::msg_send![objc2::class!(NSBezierPath), bezierPathWithRoundedRect: rect, xRadius: radius, yRadius: radius];
    if stroke {
        let _: () = objc2::msg_send![path, setLineWidth: 1.0_f64];
        let _: () = objc2::msg_send![path, stroke];
    } else {
        let _: () = objc2::msg_send![path, fill];
    }
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
        let mut background = bounds;
        let radius = if button.ivars().action == "source" {
            14.0
        } else {
            background.origin.y += 4.0;
            background.size.height -= 8.0;
            12.0
        };
        fill_round(
            background,
            radius,
            if highlighted { 0.1 } else { 0.06 },
            false,
        );
    }
    let label = button.ivars().label.borrow();
    let text_height = text_size(&label, 13.0).height;
    if button.ivars().action == "source" {
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
        let _: () =
            objc2::msg_send![path, moveToPoint: objc2_foundation::NSPoint::new(x - 3.0, 14.5)];
        let _: () = objc2::msg_send![path, lineToPoint: objc2_foundation::NSPoint::new(x, 17.5)];
        let _: () =
            objc2::msg_send![path, lineToPoint: objc2_foundation::NSPoint::new(x + 3.0, 14.5)];
        let color: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), secondaryLabelColor];
        let _: () = objc2::msg_send![color, setStroke];
        let _: () = objc2::msg_send![path, setLineWidth: 1.0_f64];
        let _: () = objc2::msg_send![path, stroke];
    } else {
        draw_text(&label, 14.0, (32.0 - text_height) / 2.0, 13.0, alpha);
        let keys = if button.ivars().action == "translate" {
            ["⌘", "↵"]
        } else {
            ["⌘", "K"]
        };
        let start = bounds.size.width - 14.0 - 43.0;
        for (index, key) in keys.into_iter().enumerate() {
            let x = start + index as f64 * 23.0;
            let rect = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(x + 0.5, 6.5),
                objc2_foundation::NSSize::new(19.0, 19.0),
            );
            fill_round(rect, 4.0, 0.06 * alpha, false);
            fill_round(rect, 4.0, 0.12 * alpha, true);
            let size = text_size(key, 12.0);
            draw_text(
                key,
                x + (20.0 - size.width) / 2.0,
                (32.0 - size.height) / 2.0,
                12.0,
                alpha,
            );
        }
    }
}

fn button(stage: &TranslationStage, tag: isize) -> Option<Retained<ChromeButton>> {
    unsafe {
        let pointer: *mut ChromeButton = objc2::msg_send![stage, viewWithTag: tag];
        Retained::retain(pointer)
    }
}

pub(super) fn install(app: &AppHandle, stage: &TranslationStage) -> Result<(), String> {
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    for (tag, action, label, width) in [
        (SOURCE_TAG, "source", "Auto-Detect", 142.0),
        (TRANSLATE_TAG, "translate", "Translate", 140.0),
        (ACTIONS_TAG, "actions", "Actions", 130.0),
    ] {
        let allocated = ChromeButton::alloc(marker).set_ivars(ButtonState {
            app: app.clone(),
            action,
            label: RefCell::new(label.into()),
            width: Cell::new(width),
            hovered: Cell::new(false),
            tracking: RefCell::new(None),
            visible: Cell::new(false),
        });
        let button: Retained<ChromeButton> = unsafe {
            objc2::msg_send![super(allocated), initWithFrame: objc2_foundation::NSRect::ZERO]
        };
        unsafe {
            let _: () = objc2::msg_send![&*button, setTag: tag];
            let _: () = objc2::msg_send![&*button, setBordered: false];
            let _: () = objc2::msg_send![&*button, setButtonType: 0usize];
            let _: () = objc2::msg_send![&*button, setTarget: &*button];
            let _: () = objc2::msg_send![&*button, setAction: objc2::sel!(pressed:)];
            let _: () = objc2::msg_send![&*button, setAutoresizingMask: 0usize];
            let _: () = objc2::msg_send![&*button, setHidden: true];
            let _: () = objc2::msg_send![stage, addSubview: &*button, positioned: 1isize, relativeTo: std::ptr::null::<AnyObject>()];
        }
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
    let Some(source) = button(stage, SOURCE_TAG) else {
        return;
    };
    let Some(translate) = button(stage, TRANSLATE_TAG) else {
        return;
    };
    let Some(actions) = button(stage, ACTIONS_TAG) else {
        return;
    };
    let right = translation.x + translation.width;
    let actions_width = actions.ivars().width.get();
    let translate_width = translate.ivars().width.get();
    for (button, x, y) in [
        (
            &source,
            right - 24.0 - source.ivars().width.get(),
            height - 40.0,
        ),
        (
            &translate,
            right - 24.0 - actions_width - 8.0 - translate_width,
            0.0,
        ),
        (&actions, right - 24.0 - actions_width, 0.0),
    ] {
        let hidden =
            !translation_visible || translation.width < 1.0 || !button.ivars().visible.get();
        unsafe {
            let rect = objc2_foundation::NSRect::new(
                objc2_foundation::NSPoint::new(x, y),
                objc2_foundation::NSSize::new(button.ivars().width.get(), 32.0),
            );
            let _: () = objc2::msg_send![&**button, setFrame: rect];
            let _: () = objc2::msg_send![&**button, setHidden: hidden];
        }
    }
}

pub fn update(
    app: &AppHandle,
    source_label: String,
    source_width: f64,
    translate_label: String,
    actions_label: String,
    can_translate: bool,
    actions_visible: bool,
) -> Result<(), String> {
    let window = app.get_window("translation").ok_or("window_unavailable")?;
    let stage = translation_stage(&window)?;
    for (tag, label, enabled, visible) in [
        (SOURCE_TAG, source_label, true, true),
        (
            TRANSLATE_TAG,
            translate_label,
            can_translate,
            actions_visible,
        ),
        (ACTIONS_TAG, actions_label, true, actions_visible),
    ] {
        let button = button(&stage, tag).ok_or("window_geometry")?;
        let width = if tag == SOURCE_TAG {
            source_width.max(unsafe { text_size(&label, 13.0).width.ceil() + 42.0 })
        } else {
            unsafe { text_size(&label, 13.0).width.ceil() + 83.0 }
        };
        button.ivars().width.set(width);
        button.ivars().visible.set(visible);
        *button.ivars().label.borrow_mut() = label.clone();
        unsafe {
            let title = objc2_foundation::NSString::from_str(&label);
            let accessible = if tag == SOURCE_TAG {
                format!("Source language: {label}")
            } else {
                label
            };
            let accessible = objc2_foundation::NSString::from_str(&accessible);
            let _: () = objc2::msg_send![&*button, setTitle: &*title];
            let _: () = objc2::msg_send![&*button, setAccessibilityLabel: &*accessible];
            let _: () = objc2::msg_send![&*button, setEnabled: enabled];
            objc2_app_kit::NSView::setNeedsDisplay(&button, true);
        }
    }
    stage.finish_layout();
    Ok(())
}
