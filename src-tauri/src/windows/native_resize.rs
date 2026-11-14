use super::*;
use objc2_foundation::{NSPoint, NSRect, NSSize};

const TAG: isize = 301;
pub(super) const EDGE: f64 = 8.0;
const CORNER: f64 = 16.0;
const LEFT: u32 = 1;
const RIGHT: u32 = 2;
const BOTTOM: u32 = 4;
const TOP: u32 = 8;

fn direction(point: NSPoint, size: NSSize) -> u32 {
    if point.x < 0.0 || point.y < 0.0 || point.x >= size.width || point.y >= size.height {
        return 0;
    }
    let horizontal = if point.x < EDGE {
        LEFT
    } else if point.x >= size.width - EDGE {
        RIGHT
    } else {
        0
    };
    let vertical = if point.y < EDGE {
        BOTTOM
    } else if point.y >= size.height - EDGE {
        TOP
    } else {
        0
    };
    if point.x < CORNER && point.y < CORNER {
        LEFT | BOTTOM
    } else if point.x < CORNER && point.y >= size.height - CORNER {
        LEFT | TOP
    } else if point.x >= size.width - CORNER && point.y < CORNER {
        RIGHT | BOTTOM
    } else if point.x >= size.width - CORNER && point.y >= size.height - CORNER {
        RIGHT | TOP
    } else {
        horizontal | vertical
    }
}

#[derive(Clone, Copy)]
struct Drag {
    direction: u32,
    mouse: NSPoint,
    frame: NSRect,
}

fn dragged_frame(drag: Drag, mouse: NSPoint, minimum: NSSize, maximum: NSSize) -> NSRect {
    let dx = mouse.x - drag.mouse.x;
    let dy = mouse.y - drag.mouse.y;
    let mut frame = drag.frame;
    let width = (frame.size.width
        + if drag.direction & LEFT != 0 {
            -dx
        } else if drag.direction & RIGHT != 0 {
            dx
        } else {
            0.0
        })
    .clamp(minimum.width, maximum.width.max(minimum.width));
    let height = (frame.size.height
        + if drag.direction & BOTTOM != 0 {
            -dy
        } else if drag.direction & TOP != 0 {
            dy
        } else {
            0.0
        })
    .clamp(minimum.height, maximum.height.max(minimum.height));
    if drag.direction & LEFT != 0 {
        frame.origin.x += frame.size.width - width;
    }
    if drag.direction & BOTTOM != 0 {
        frame.origin.y += frame.size.height - height;
    }
    frame.size = NSSize::new(width, height);
    frame
}

struct ResizeState {
    app: AppHandle,
    drag: Cell<Option<Drag>>,
}

objc2::define_class!(
    #[unsafe(super = objc2_app_kit::NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = ResizeState]
    struct ResizeTarget;
    impl ResizeTarget {
        #[unsafe(method(tag))]
        fn tag(&self) -> isize { TAG }
        #[unsafe(method(acceptsFirstMouse:))]
        fn first_mouse(&self, _: *mut AnyObject) -> bool { true }
        #[unsafe(method(resizeDirectionAtPoint:))]
        fn direction_at_point(&self, point: NSPoint) -> u32 {
            self.edge_at(point)
        }
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut AnyObject {
            unsafe {
                let parent: *mut AnyObject = objc2::msg_send![self, superview];
                let local: NSPoint = objc2::msg_send![self, convertPoint: point, fromView: parent];
                if self.edge_at(local) != 0 { (self as *const Self).cast_mut().cast() } else { std::ptr::null_mut() }
            }
        }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: *mut AnyObject) {
            unsafe {
                let location: NSPoint = objc2::msg_send![event, locationInWindow];
                let local: NSPoint = objc2::msg_send![self, convertPoint: location, fromView: std::ptr::null::<AnyObject>()];
                let mouse: NSPoint = objc2::msg_send![objc2::class!(NSEvent), mouseLocation];
                let _ = self.begin_drag(local, mouse);
            }
        }
        #[unsafe(method(mouseDragged:))]
        fn dragged(&self, _: *mut AnyObject) {
            let mouse: NSPoint = unsafe { objc2::msg_send![objc2::class!(NSEvent), mouseLocation] };
            self.drag_to(mouse);
        }
        #[unsafe(method(mouseUp:))]
        fn up(&self, _: *mut AnyObject) {
            let _ = self.end_drag();
        }
        #[unsafe(method(resetCursorRects))]
        fn cursors(&self) {
            unsafe {
                let bounds: NSRect = objc2::msg_send![self, bounds];
                let w = bounds.size.width;
                let h = bounds.size.height;
                for (area, edge) in [
                    (NSRect::new(NSPoint::new(0.0,CORNER),NSSize::new(EDGE,h-2.0*CORNER)),LEFT),
                    (NSRect::new(NSPoint::new(w-EDGE,CORNER),NSSize::new(EDGE,h-2.0*CORNER)),RIGHT),
                    (NSRect::new(NSPoint::new(CORNER,0.0),NSSize::new(w-2.0*CORNER,EDGE)),BOTTOM),
                    (NSRect::new(NSPoint::new(CORNER,h-EDGE),NSSize::new(w-2.0*CORNER,EDGE)),TOP),
                    (NSRect::new(NSPoint::new(0.0,0.0),NSSize::new(CORNER,CORNER)),LEFT|BOTTOM),
                    (NSRect::new(NSPoint::new(0.0,h-CORNER),NSSize::new(CORNER,CORNER)),LEFT|TOP),
                    (NSRect::new(NSPoint::new(w-CORNER,0.0),NSSize::new(CORNER,CORNER)),RIGHT|BOTTOM),
                    (NSRect::new(NSPoint::new(w-CORNER,h-CORNER),NSSize::new(CORNER,CORNER)),RIGHT|TOP),
                ] {
                    let class = objc2::class!(NSCursor);
                    let modern: bool = objc2::msg_send![class, respondsToSelector: objc2::sel!(frameResizeCursorFromPosition:inDirections:)];
                    let cursor: *mut AnyObject = if modern {
                        let position = (if edge & TOP != 0 { 1usize } else { 0 }) | (if edge & LEFT != 0 { 2 } else { 0 }) | (if edge & BOTTOM != 0 { 4 } else { 0 }) | (if edge & RIGHT != 0 { 8 } else { 0 });
                        objc2::msg_send![class, frameResizeCursorFromPosition: position, inDirections: 3usize]
                    } else if edge & (LEFT|RIGHT) != 0 { objc2::msg_send![class, resizeLeftRightCursor] } else { objc2::msg_send![class, resizeUpDownCursor] };
                    let _: () = objc2::msg_send![self, addCursorRect: area, cursor: cursor];
                }
            }
        }
    }
);

impl ResizeTarget {
    fn edge_at(&self, point: NSPoint) -> u32 {
        let bounds: NSRect = unsafe { objc2::msg_send![self, bounds] };
        direction(point, bounds.size)
    }

    fn begin_drag(&self, point: NSPoint, mouse: NSPoint) -> Result<(), String> {
        let direction = self.edge_at(point);
        if direction == 0 {
            return Err("resize_target_missing".into());
        }
        begin_translation_resize(&self.ivars().app)?;
        let window: *mut AnyObject = unsafe { objc2::msg_send![self, window] };
        self.ivars().drag.set(Some(Drag {
            direction,
            mouse,
            frame: unsafe { objc2::msg_send![window, frame] },
        }));
        Ok(())
    }

    fn drag_to(&self, mouse: NSPoint) {
        let Some(drag) = self.ivars().drag.get() else {
            return;
        };
        unsafe {
            let window: *mut AnyObject = objc2::msg_send![self, window];
            let minimum: NSSize = objc2::msg_send![window, contentMinSize];
            let maximum: NSSize = objc2::msg_send![window, contentMaxSize];
            let frame = dragged_frame(drag, mouse, minimum, maximum);
            let _: () = objc2::msg_send![window, setFrame: frame, display: true];
        }
    }

    fn end_drag(&self) -> Result<(), String> {
        if self.ivars().drag.take().is_some() {
            if let Some(window) = self.ivars().app.get_window("translation") {
                end_translation_resize(&self.ivars().app, &window)?;
            }
        }
        Ok(())
    }
}

#[cfg(feature = "ui-tests")]
pub(super) fn test_drag(window: &Window, point: NSPoint, delta: NSPoint) -> Result<(), String> {
    let stage = translation_stage(window)?;
    unsafe {
        let target: *mut ResizeTarget = objc2::msg_send![&*stage, viewWithTag: TAG];
        if target.is_null() {
            return Err("resize_target_missing".into());
        }
        let target = &*target;
        target.begin_drag(point, NSPoint::new(0.0, 0.0))?;
        target.drag_to(delta);
        target.end_drag()
    }
}

pub(super) fn install(app: &AppHandle, stage: &TranslationStage) -> Result<(), String> {
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    unsafe {
        let bounds: NSRect = objc2::msg_send![stage, bounds];
        let target = ResizeTarget::alloc(marker).set_ivars(ResizeState {
            app: app.clone(),
            drag: Cell::new(None),
        });
        let target: Retained<ResizeTarget> = objc2::msg_send![super(target), initWithFrame: bounds];
        let _: () = objc2::msg_send![&*target, setAutoresizingMask: 18usize];
        let _: () = objc2::msg_send![stage, addSubview: &*target];
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hit_bands_cover_edges_and_corners_but_leave_content_alone() {
        let size = NSSize::new(1000.0, 600.0);
        for (point, expected) in [
            (NSPoint::new(7.0, 100.0), LEFT),
            (NSPoint::new(993.0, 100.0), RIGHT),
            (NSPoint::new(100.0, 7.0), BOTTOM),
            (NSPoint::new(100.0, 593.0), TOP),
            (NSPoint::new(12.0, 12.0), LEFT | BOTTOM),
            (NSPoint::new(988.0, 588.0), RIGHT | TOP),
            (NSPoint::new(20.0, 20.0), 0),
            (NSPoint::new(-1.0, 10.0), 0),
        ] {
            assert_eq!(direction(point, size), expected);
        }
    }
    #[test]
    fn resize_uses_original_mouse_and_frame_and_locks_opposite_edges_at_limits() {
        let drag = Drag {
            direction: LEFT | BOTTOM,
            mouse: NSPoint::new(100.0, 100.0),
            frame: NSRect::new(NSPoint::new(50.0, 60.0), NSSize::new(1000.0, 600.0)),
        };
        let min = NSSize::new(700.0, 440.0);
        let max = NSSize::new(1600.0, 1000.0);
        for point in [
            NSPoint::new(200.0, 200.0),
            NSPoint::new(900.0, 900.0),
            NSPoint::new(-900.0, -900.0),
            drag.mouse,
        ] {
            let frame = dragged_frame(drag, point, min, max);
            assert_eq!(frame.origin.x + frame.size.width, 1050.0);
            assert_eq!(frame.origin.y + frame.size.height, 660.0);
            assert!((700.0..=1600.0).contains(&frame.size.width));
            assert!((440.0..=1000.0).contains(&frame.size.height));
        }
        assert_eq!(dragged_frame(drag, drag.mouse, min, max), drag.frame);
    }
}
