use super::*;
use objc2_app_kit::{NSScrollView, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
};

const ROOT_TAG: isize = 201;
const SEARCH_TAG: isize = 205;
const DIVIDER_TAG: isize = 206;
const SHORTCUTS_TAG: isize = 207;
const WORDMARK_TAG: isize = 208;
const CLEAR_SEARCH_TAG: isize = 209;
const SEARCH_ICON_TAG: isize = 210;
const RECORD_TAG: isize = 1000;

use crate::model::HistoryRecord as Record;

struct HistoryState {
    app: AppHandle,
    records: RefCell<Vec<Record>>,
    order: RefCell<HashMap<String, usize>>,
    selected: RefCell<Option<String>>,
    generation: Cell<u64>,
    total_records: Cell<usize>,
    has_more: Cell<bool>,
    query_pending: Cell<bool>,
    load_pending: Cell<bool>,
    query_serial: Cell<u64>,
    requested_query: RefCell<Option<(String, bool)>>,
    suppress_load: Cell<bool>,
    favorites_only: Cell<bool>,
    favorite_icon_filled: Cell<bool>,
    pin_icon_filled: Cell<bool>,
    search: Cell<usize>,
    search_focused: Cell<bool>,
    scroll: Cell<usize>,
    document: Cell<usize>,
    pinned_scroll: Cell<usize>,
    pinned_document: Cell<usize>,
    record_count: Cell<usize>,
    progress: Cell<usize>,
    shortcuts: Cell<usize>,
    rows: RefCell<Vec<Retained<HistoryRow>>>,
    size: Cell<NSSize>,
    divider: Cell<usize>,
    empty_label: Cell<usize>,
    layout_count: Cell<usize>,
}

struct RowState {
    root: usize,
    index: usize,
    hovered: Cell<bool>,
    tracking: RefCell<Option<Retained<AnyObject>>>,
    heading: Cell<usize>,
    time: Cell<usize>,
    preview: Cell<usize>,
    actions: RefCell<Vec<Retained<AnyObject>>>,
}

struct DividerState {
    app: AppHandle,
    last_x: Cell<f64>,
}

struct HistoryScrollState {
    top_inset: Cell<f64>,
}

objc2::define_class!(
    #[unsafe(super = NSTextField)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = usize]
    struct HistorySearchField;
    impl HistorySearchField {
        #[unsafe(method(becomeFirstResponder))]
        fn focus(&self) -> objc2::runtime::Bool {
            let accepted: objc2::runtime::Bool = unsafe { objc2::msg_send![super(self), becomeFirstResponder] };
            if accepted.as_bool() { self.sync_focus(true); }
            accepted
        }
        #[unsafe(method(selectText:))]
        fn select_text(&self, sender: *mut AnyObject) {
            unsafe { let _: () = objc2::msg_send![super(self), selectText: sender]; }
            let editor: *mut AnyObject = unsafe { objc2::msg_send![self, currentEditor] };
            self.sync_focus(!editor.is_null());
        }
        #[unsafe(method(textDidEndEditing:))]
        fn end_editing(&self, notification: *mut AnyObject) {
            unsafe { let _: () = objc2::msg_send![super(self), textDidEndEditing: notification]; }
            self.sync_focus(false);
        }
    }
);

impl HistorySearchField {
    fn sync_focus(&self, focused: bool) {
        let root = unsafe { &*(*self.ivars() as *const NativeHistory) };
        root.ivars().search_focused.set(focused);
        root.update_search_appearance();
    }
}

objc2::define_class!(
    #[unsafe(super = NSScrollView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = HistoryScrollState]
    struct HistoryScrollView;
    impl HistoryScrollView {
        #[unsafe(method(tile))]
        fn tile(&self) {
            unsafe {
                let _: () = objc2::msg_send![super(self), tile];
                let clip: *mut AnyObject = objc2::msg_send![self, contentView];
                if clip.is_null() { return; }
                let mut bounds: NSRect = objc2::msg_send![clip, frame];
                let inset = self.ivars().top_inset.get().min(bounds.size.height);
                let flipped: bool = objc2::msg_send![self, isFlipped];
                if flipped { bounds.origin.y += inset; }
                bounds.size.height -= inset;
                frame(clip, bounds);
            }
            self.update_edge_fade();
        }
        #[unsafe(method(layout))]
        fn layout(&self) {
            unsafe { let _: () = objc2::msg_send![super(self), layout]; }
            self.update_edge_fade();
        }
        #[unsafe(method(reflectScrolledClipView:))]
        fn reflect_scrolled_clip_view(&self, clip: *mut AnyObject) {
            unsafe { let _: () = objc2::msg_send![super(self), reflectScrolledClipView: clip]; }
            self.update_edge_fade();
            self.request_more_if_needed();
        }
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut AnyObject {
            unsafe {
                let parent: *mut AnyObject = objc2::msg_send![self, superview];
                let local: NSPoint = objc2::msg_send![self, convertPoint: point, fromView: parent];
                let bounds: NSRect = objc2::msg_send![self, bounds];
                let flipped: bool = objc2::msg_send![self, isFlipped];
                let in_pinned = if flipped { local.y < self.ivars().top_inset.get() } else { local.y > bounds.size.height - self.ivars().top_inset.get() };
                if in_pinned {
                    let scroller: *mut AnyObject = objc2::msg_send![self, verticalScroller];
                    if scroller.is_null() { return std::ptr::null_mut(); }
                    let scroll_frame: NSRect = objc2::msg_send![scroller, frame];
                    if local.x < scroll_frame.origin.x || local.x >= scroll_frame.origin.x + scroll_frame.size.width || local.y < scroll_frame.origin.y || local.y >= scroll_frame.origin.y + scroll_frame.size.height {
                        return std::ptr::null_mut();
                    }
                }
                objc2::msg_send![super(self), hitTest: point]
            }
        }
    }
);

impl HistoryScrollView {
    fn request_more_if_needed(&self) {
        unsafe {
            let parent: *mut AnyObject = objc2::msg_send![self, superview];
            if parent.is_null() {
                return;
            }
            let tag: isize = objc2::msg_send![parent, tag];
            if tag != ROOT_TAG {
                return;
            }
            let hidden: bool = objc2::msg_send![self, isHiddenOrHasHiddenAncestor];
            if hidden {
                return;
            }
            let clip: *mut AnyObject = objc2::msg_send![self, contentView];
            let document: *mut AnyObject = objc2::msg_send![self, documentView];
            if clip.is_null() || document.is_null() {
                return;
            }
            let viewport: NSRect = objc2::msg_send![clip, bounds];
            let content: NSRect = objc2::msg_send![document, bounds];
            if viewport.size.height > 0.0
                && viewport.origin.y + viewport.size.height >= content.size.height - 180.0
            {
                (&*(parent as *const NativeHistory)).request_more();
            }
        }
    }

    fn update_edge_fade(&self) {
        unsafe {
            let clip: *mut AnyObject = objc2::msg_send![self, contentView];
            let layer: *mut AnyObject = objc2::msg_send![clip, layer];
            if layer.is_null() {
                return;
            }
            let mask: *mut AnyObject = objc2::msg_send![layer, mask];
            if mask.is_null() {
                return;
            }
            let bounds: NSRect = objc2::msg_send![clip, bounds];
            let fade = (8.0 / bounds.size.height.max(1.0)).min(0.5);
            let locations: Retained<AnyObject> =
                objc2::msg_send![objc2::class!(NSMutableArray), new];
            for value in [0.0, fade, 1.0 - fade, 1.0] {
                let number: *mut AnyObject =
                    objc2::msg_send![objc2::class!(NSNumber), numberWithDouble: value];
                let _: () = objc2::msg_send![&*locations, addObject: number];
            }
            let _: () = objc2::msg_send![objc2::class!(CATransaction), begin];
            let _: () = objc2::msg_send![objc2::class!(CATransaction), setDisableActions: true];
            frame(mask, bounds);
            let _: () = objc2::msg_send![mask, setLocations: &*locations];
            let _: () = objc2::msg_send![objc2::class!(CATransaction), commit];
        }
    }
}

objc2::define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = HistoryState]
    struct NativeHistory;
    impl NativeHistory {
        #[unsafe(method(tag))]
        fn tag(&self) -> isize { ROOT_TAG }
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstResponder))]
        fn responder(&self) -> bool { true }
        #[unsafe(method(acceptsFirstMouse:))]
        fn first_mouse(&self, _: *mut AnyObject) -> bool { true }
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut AnyObject {
            unsafe {
                let hit: *mut AnyObject = objc2::msg_send![super(self), hitTest: point];
                let wordmark: *mut AnyObject = objc2::msg_send![self, viewWithTag: WORDMARK_TAG];
                let hit = if !hit.is_null() && hit == wordmark { (self as *const Self).cast_mut().cast() } else { hit };
                let popup = self.ivars().shortcuts.get() as *mut AnyObject;
                if popup.is_null() || hit.is_null() { return hit; }
                let app: *mut AnyObject = objc2::msg_send![objc2::class!(NSApplication), sharedApplication];
                let event: *mut AnyObject = objc2::msg_send![app, currentEvent];
                let event_type: usize = if event.is_null() { 0 } else { objc2::msg_send![event, type] };
                let button: *mut AnyObject = objc2::msg_send![self, viewWithTag: SHORTCUTS_TAG];
                let inside: bool = objc2::msg_send![hit, isDescendantOf: popup];
                if (event_type == 1 || event_type == 3) && hit != button && hit != popup && !inside {
                    let _: () = objc2::msg_send![popup, setHidden: true];
                }
                hit
            }
        }
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance(&self) { unsafe { let _: ()=objc2::msg_send![super(self),viewDidChangeEffectiveAppearance]; } self.update_search_appearance(); self.setNeedsDisplay(true); }
        #[unsafe(method(layout))]
        fn layout(&self) { self.layout_content(); unsafe { let _: () = objc2::msg_send![super(self), layout]; } }
        #[unsafe(method(resizeSubviewsWithOldSize:))]
        fn resize(&self, _: NSSize) { self.layout_content(); }
        #[unsafe(method(pressed:))]
        fn pressed(&self, sender: *mut AnyObject) { unsafe { self.perform(objc2::msg_send![sender, tag]); } }
        #[unsafe(method(mouseDown:))]
        fn down(&self, event: *mut AnyObject) {
            unsafe { let window: *mut AnyObject = objc2::msg_send![self, window]; let _: () = objc2::msg_send![window, performWindowDragWithEvent: event]; }
        }
        #[unsafe(method(controlTextDidBeginEditing:))]
        fn search_editing(&self, _: *mut AnyObject) {
            self.ivars().search_focused.set(true);
            self.update_search_appearance();
            unsafe {
                let search = self.ivars().search.get() as *mut AnyObject;
                let editor: *mut AnyObject = objc2::msg_send![search, currentEditor];
                if !editor.is_null() {
                    let gray: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), grayColor];
                    let _: () = objc2::msg_send![editor, setInsertionPointColor: gray];
                }
            }
        }
        #[unsafe(method(controlTextDidEndEditing:))]
        fn search_ended(&self, _: *mut AnyObject) {
            self.ivars().search_focused.set(false);
            self.update_search_appearance();
        }
        #[unsafe(method(controlTextDidChange:))]
        fn search_changed(&self, _: *mut AnyObject) { self.request_query(); }
        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: *mut AnyObject) {
            unsafe {
                let code: u16 = objc2::msg_send![event, keyCode];
                let flags: usize = objc2::msg_send![event, modifierFlags];
                let shortcuts = self.ivars().shortcuts.get() as *mut AnyObject;
                let hidden: bool = objc2::msg_send![shortcuts, isHidden];
                if code == 53 && !hidden {
                    let _: () = objc2::msg_send![shortcuts, setHidden: true];
                } else if code == 53 || (code == 13 && flags & (1 << 20) != 0) {
                    let _ = super::hide(&self.ivars().app, "translation");
                } else if code == 125 || code == 126 {
                    self.move_selection(if code == 125 { 1 } else { -1 });
                } else if code == 36 {
                    if let Some(id) = self.ivars().selected.borrow().clone() {
                        let _ = crate::commands::select_history_sample(&self.ivars().app, Some(id), false);
                    }
                } else { let _: () = objc2::msg_send![super(self), keyDown: event]; }
            }
        }
    }
);

objc2::define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    struct HistoryDocument;
    impl HistoryDocument {
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
    }
);

objc2::define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = RowState]
    struct HistoryRow;
    impl HistoryRow {
        #[unsafe(method(tag))]
        fn tag(&self) -> isize { 3000 + self.ivars().index as isize }
        #[unsafe(method(isFlipped))]
        fn flipped(&self) -> bool { true }
        #[unsafe(method(acceptsFirstMouse:))]
        fn first_mouse(&self, _: *mut AnyObject) -> bool { true }
        #[unsafe(method(hitTest:))]
        fn hit_test(&self, point: NSPoint) -> *mut AnyObject {
            unsafe {
                let hit: *mut AnyObject = objc2::msg_send![super(self), hitTest: point];
                if hit.is_null() { return hit; }
                let button: bool = objc2::msg_send![hit, isKindOfClass: objc2::class!(NSButton)];
                let enabled = button && objc2::msg_send![hit, isEnabled];
                if enabled { hit } else { self as *const Self as *mut AnyObject }
            }
        }
        #[unsafe(method(accessibilityPerformPress))]
        fn accessible_press(&self) -> bool {
            let root = unsafe { &*(self.ivars().root as *const NativeHistory) };
            root.select(self.ivars().index, false); true
        }
        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, _: *mut AnyObject) {
            let root = unsafe { &*(self.ivars().root as *const NativeHistory) };
            root.select(self.ivars().index, false);
            unsafe {
                let window: *mut AnyObject = objc2::msg_send![self, window];
                let _: bool = objc2::msg_send![window, makeFirstResponder: root];
            }
        }
        #[unsafe(method(updateTrackingAreas))]
        fn tracking(&self) {
            unsafe {
                let _: () = objc2::msg_send![super(self), updateTrackingAreas];
                if self.ivars().tracking.borrow().is_none() {
                    let allocated: *mut AnyObject = objc2::msg_send![objc2::class!(NSTrackingArea), alloc];
                    let area: *mut AnyObject = objc2::msg_send![allocated, initWithRect: NSRect::ZERO, options: 641usize, owner: self, userInfo: std::ptr::null::<AnyObject>()];
                    if let Some(area) = Retained::from_raw(area) {
                        let _: () = objc2::msg_send![self, addTrackingArea: &*area];
                        *self.ivars().tracking.borrow_mut() = Some(area);
                    }
                }
            }
        }
        #[unsafe(method(mouseEntered:))]
        fn entered(&self, _: *mut AnyObject) { self.set_hover(true); }
        #[unsafe(method(mouseExited:))]
        fn exited(&self, _: *mut AnyObject) { self.set_hover(false); }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) { unsafe { self.draw_row(); } }
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn appearance(&self) { unsafe { let _: () = objc2::msg_send![super(self), viewDidChangeEffectiveAppearance]; } self.setNeedsDisplay(true); }
    }
);

objc2::define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = objc2::MainThreadOnly]
    #[ivars = DividerState]
    struct HistoryDivider;
    impl HistoryDivider {
        #[unsafe(method(tag))]
        fn tag(&self) -> isize { DIVIDER_TAG }
        #[unsafe(method(drawRect:))]
        fn draw(&self, _: NSRect) {
            unsafe {
                let bounds: NSRect = objc2::msg_send![self, bounds];
                let base: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
                let colors: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSMutableArray), new];
                for alpha in [0.0_f64, 0.04, 0.12, 0.04, 0.0] {
                    let color: *mut AnyObject = objc2::msg_send![base, colorWithAlphaComponent: alpha];
                    let _: () = objc2::msg_send![&*colors, addObject: color];
                }
                let positions = [0.0_f64, 0.25, 0.5, 0.75, 1.0];
                let space: *mut AnyObject = objc2::msg_send![objc2::class!(NSColorSpace), sRGBColorSpace];
                let gradient: *mut AnyObject = objc2::msg_send![objc2::class!(NSGradient), alloc];
                let gradient: *mut AnyObject = objc2::msg_send![gradient, initWithColors: &*colors, atLocations: positions.as_ptr(), colorSpace: space];
                if let Some(gradient) = Retained::from_raw(gradient) {
                    let _: () = objc2::msg_send![&*gradient, drawInRect: rect(bounds.size.width - 1.0, 0.0, 1.0, bounds.size.height), angle: 90.0_f64];
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
                let _ = super::resize_history(&self.ivars().app, width + delta, false);
            }
        }
        #[unsafe(method(resetCursorRects))]
        fn cursor(&self) { unsafe { let bounds: NSRect = objc2::msg_send![self, bounds]; let cursor: *mut AnyObject = objc2::msg_send![objc2::class!(NSCursor), resizeLeftRightCursor]; let _: () = objc2::msg_send![self, addCursorRect: bounds, cursor: cursor]; } }
        #[unsafe(method(accessibilityPerformIncrement))]
        fn increment(&self) -> bool { let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().history_width; super::resize_history(&self.ivars().app, width + 24.0, false).is_ok() }
        #[unsafe(method(accessibilityPerformDecrement))]
        fn decrement(&self) -> bool { let width = self.ivars().app.state::<AppState>().windows.lock().unwrap().history_width; super::resize_history(&self.ivars().app, width - 24.0, false).is_ok() }
    }
);

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(
        NSPoint::new(x, y),
        NSSize::new(width.max(0.0), height.max(0.0)),
    )
}
unsafe fn frame(view: *mut AnyObject, value: NSRect) {
    let _: () = objc2::msg_send![view, setFrame: value];
}
unsafe fn label(value: &str, size: f64, secondary: bool) -> Retained<AnyObject> {
    let value = NSString::from_str(value);
    let field: Retained<AnyObject> =
        objc2::msg_send![objc2::class!(NSTextField), labelWithString: &*value];
    let font: *mut AnyObject = objc2::msg_send![objc2::class!(NSFont), systemFontOfSize: size];
    let color: *mut AnyObject = if secondary {
        objc2::msg_send![objc2::class!(NSColor), secondaryLabelColor]
    } else {
        objc2::msg_send![objc2::class!(NSColor), labelColor]
    };
    let _: () = objc2::msg_send![&*field, setFont: font];
    let _: () = objc2::msg_send![&*field, setTextColor: color];
    let _: () = objc2::msg_send![&*field, setLineBreakMode: 4usize];
    field
}
unsafe fn symbol_button(
    root: &NativeHistory,
    tag: isize,
    symbol: &str,
    title: &str,
) -> Retained<AnyObject> {
    let button: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSButton), new];
    let _: () = objc2::msg_send![&*button, setTag: tag];
    let _: () = objc2::msg_send![&*button, setBordered: false];
    let _: () = objc2::msg_send![&*button, setButtonType: 0usize];
    let _: () = objc2::msg_send![&*button, setImagePosition: 1usize];
    let _: () = objc2::msg_send![&*button, setImageScaling: 2usize];
    let _: () = objc2::msg_send![&*button, setTarget: root];
    let _: () = objc2::msg_send![&*button, setAction: objc2::sel!(pressed:)];
    let _: () = objc2::msg_send![&*button, setFocusRingType: 1usize];
    let title = NSString::from_str(title);
    let _: () = objc2::msg_send![&*button, setAccessibilityLabel: &*title];
    let _: () = objc2::msg_send![&*button, setToolTip: &*title];
    set_symbol(&button, symbol);
    button
}
unsafe fn set_symbol(button: &AnyObject, symbol: &str) {
    let reference: Option<&[u8]> = match symbol {
        "list.bullet" => Some(include_bytes!(
            "../../../src/assets/history/reference-list.png"
        )),
        "trash" => Some(include_bytes!(
            "../../../src/assets/history/reference-trash.png"
        )),
        "keyboard" => Some(include_bytes!(
            "../../../src/assets/history/reference-keyboard.png"
        )),
        "magnifyingglass" => Some(include_bytes!(
            "../../../src/assets/history/reference-search.png"
        )),
        "xmark.circle.fill" => Some(include_bytes!(
            "../../../src/assets/history/reference-clear.png"
        )),
        "languages" => Some(include_bytes!(
            "../../../src/assets/history/reference-languages.png"
        )),
        "doc.on.doc" => Some(include_bytes!(
            "../../../src/assets/history/reference-copy.png"
        )),
        "star" => Some(include_bytes!(
            "../../../src/assets/history/reference-star.png"
        )),
        "star.fill" => Some(include_bytes!(
            "../../../src/assets/history/reference-star-filled.png"
        )),
        "pin" => Some(include_bytes!(
            "../../../src/assets/history/reference-pin.png"
        )),
        "pin.fill" => Some(include_bytes!(
            "../../../src/assets/history/reference-pin-filled.png"
        )),
        _ => None,
    };
    if let Some(reference) = reference {
        let data: *mut AnyObject = objc2::msg_send![objc2::class!(NSData), dataWithBytes: reference.as_ptr(), length: reference.len()];
        let allocated: *mut AnyObject = objc2::msg_send![objc2::class!(NSImage), alloc];
        let image: *mut AnyObject = objc2::msg_send![allocated, initWithData: data];
        let image = Retained::from_raw(image).expect("reference history icon");
        let _: () = objc2::msg_send![&*image, setSize: NSSize::new(14.0, 14.0)];
        let _: () = objc2::msg_send![&*image, setTemplate: true];
        let _: () = objc2::msg_send![button, setImage: &*image];
        return;
    }
    let symbol = NSString::from_str(symbol);
    let image: Option<Retained<AnyObject>> = objc2::msg_send![objc2::class!(NSImage), imageWithSystemSymbolName: &*symbol, accessibilityDescription: std::ptr::null::<NSString>()];
    if let Some(image) = image {
        let configuration: *mut AnyObject = objc2::msg_send![objc2::class!(NSImageSymbolConfiguration), configurationWithPointSize: 14.0_f64, weight: 0.0_f64];
        let configured: Retained<AnyObject> =
            objc2::msg_send![&*image, imageWithSymbolConfiguration: configuration];
        let _: () = objc2::msg_send![&*configured, setTemplate: true];
        // Raster templates prevent NSButton from reconfiguring SF Symbol geometry.
        let extent = 14.0_f64;
        let size: NSSize = objc2::msg_send![&*configured, size];
        let scale = extent / size.width.max(size.height).max(1.0);
        let width = size.width * scale;
        let height = size.height * scale;
        let allocated: *mut AnyObject = objc2::msg_send![objc2::class!(NSImage), alloc];
        let pointer: *mut AnyObject =
            objc2::msg_send![allocated, initWithSize: NSSize::new(extent, extent)];
        let template = Retained::from_raw(pointer).expect("history symbol template");
        let _: () = objc2::msg_send![&*template, lockFocus];
        let _: () = objc2::msg_send![&*configured, drawInRect: rect((extent-width)/2.0, (extent-height)/2.0, width, height)];
        let _: () = objc2::msg_send![&*template, unlockFocus];
        let _: () = objc2::msg_send![&*template, setTemplate: true];
        let _: () = objc2::msg_send![button, setImage: &*template];
    }
}

unsafe fn source_icon(record: &Record) -> Option<Retained<AnyObject>> {
    let url = record.source_app.as_ref()?.get("iconDataUrl")?.as_str()?;
    let (mime, encoded) = url.split_once(",")?;
    if !mime.starts_with("data:image/") || !mime.ends_with(";base64") {
        return None;
    }
    let allocated: objc2::rc::Allocated<AnyObject> = objc2::msg_send![objc2::class!(NSData), alloc];
    let data: Option<Retained<AnyObject>> = objc2::msg_send![allocated, initWithBase64EncodedString: &*NSString::from_str(encoded), options: 0usize];
    let image: objc2::rc::Allocated<AnyObject> = objc2::msg_send![objc2::class!(NSImage), alloc];
    objc2::msg_send![image, initWithData: &*data?]
}

unsafe fn default_source_icon() -> Retained<AnyObject> {
    let bytes = include_bytes!("../../../src/assets/history/reference-nami.png");
    let data: *mut AnyObject =
        objc2::msg_send![objc2::class!(NSData), dataWithBytes: bytes.as_ptr(), length: bytes.len()];
    let allocated: objc2::rc::Allocated<AnyObject> =
        objc2::msg_send![objc2::class!(NSImage), alloc];
    let image: Retained<AnyObject> = objc2::msg_send![allocated, initWithData: data];
    let _: () = objc2::msg_send![&*image, setSize: NSSize::new(14.0, 14.0)];
    let _: () = objc2::msg_send![&*image, setTemplate: false];
    image
}

unsafe fn animate_actions(button: &AnyObject, showing: bool) {
    let layer: *mut AnyObject = objc2::msg_send![button, layer];
    let presentation: *mut AnyObject = objc2::msg_send![layer, presentationLayer];
    let current = if presentation.is_null() {
        layer
    } else {
        presentation
    };
    let workspace: *mut AnyObject = objc2::msg_send![objc2::class!(NSWorkspace), sharedWorkspace];
    let reduced: bool = objc2::msg_send![workspace, accessibilityDisplayShouldReduceMotion];
    let key = objc2_foundation::ns_string!("historyActions");
    let animations: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSMutableArray), new];
    let _: () = objc2::msg_send![objc2::class!(CATransaction), begin];
    let _: () = objc2::msg_send![objc2::class!(CATransaction), setDisableActions: true];
    for (path, target) in [
        ("transform.translation.x", if showing { 0.0 } else { 8.0 }),
        ("opacity", if showing { 1.0 } else { 0.0 }),
    ] {
        let path = NSString::from_str(path);
        let from: Retained<AnyObject> = objc2::msg_send![current, valueForKeyPath: &*path];
        let to: Retained<AnyObject> =
            objc2::msg_send![objc2::class!(NSNumber), numberWithDouble: target];
        let _: () = objc2::msg_send![layer, setValue: &*to, forKeyPath: &*path];
        let animation: Retained<AnyObject> =
            objc2::msg_send![objc2::class!(CABasicAnimation), animationWithKeyPath: &*path];
        let _: () = objc2::msg_send![&*animation, setFromValue: &*from];
        let _: () = objc2::msg_send![&*animation, setToValue: &*to];
        let _: () = objc2::msg_send![&*animations, addObject: &*animation];
    }
    let _: () = objc2::msg_send![objc2::class!(CATransaction), commit];
    if reduced {
        let _: () = objc2::msg_send![layer, removeAnimationForKey: key];
    } else {
        let group: Retained<AnyObject> =
            objc2::msg_send![objc2::class!(CAAnimationGroup), animation];
        let _: () = objc2::msg_send![&*group, setAnimations: &*animations];
        let _: () =
            objc2::msg_send![&*group, setDuration: if showing { 0.14_f64 } else { 0.1_f64 }];
        let timing: *mut AnyObject = objc2::msg_send![objc2::class!(CAMediaTimingFunction), functionWithName: objc2_foundation::ns_string!("easeOut")];
        let _: () = objc2::msg_send![&*group, setTimingFunction: timing];
        let _: () = objc2::msg_send![layer, addAnimation: &*group, forKey: key];
    }
}

fn language(code: &str) -> &str {
    match code {
        "auto" => "Auto-Detect",
        "en" => "English",
        "zh-CN" => "Chinese (Simplified)",
        "zh-TW" => "Chinese (Traditional)",
        "fr" => "French",
        "ja" => "Japanese",
        "de" => "German",
        "ko" => "Korean",
        "es" => "Spanish",
        "pt" => "Portuguese (Brazil)",
        "it" => "Italian",
        "ar" => "Arabic",
        "ru" => "Russian",
        "nl" => "Dutch",
        "id" => "Indonesian",
        "pl" => "Polish",
        "tr" => "Turkish",
        "vi" => "Vietnamese",
        other => other,
    }
}

impl HistoryRow {
    fn protected(&self) -> bool {
        let root = unsafe { &*(self.ivars().root as *const NativeHistory) };
        let records = root.ivars().records.borrow();
        records[self.ivars().index].is_favorite || records[self.ivars().index].is_pinned
    }
    fn set_hover(&self, value: bool) {
        if self.ivars().hovered.replace(value) == value {
            return;
        }
        unsafe {
            let bounds: NSRect = objc2::msg_send![self, bounds];
            self.resize_content(bounds.size.width);
            let protected = self.protected();
            for (index, button) in self.ivars().actions.borrow().iter().enumerate() {
                let allowed = index != 4 || !protected;
                let _: () = objc2::msg_send![&**button, setHidden: !allowed];
                let _: () = objc2::msg_send![&**button, setEnabled: value && allowed];
                let _: () = objc2::msg_send![&**button, setAccessibilityElement: value && allowed];
                animate_actions(button, value);
            }
            let _: () =
                objc2::msg_send![self.ivars().time.get() as *mut AnyObject, setHidden: value];
        }
        self.setNeedsDisplay(true);
    }
    unsafe fn draw_row(&self) {
        let root = &*(self.ivars().root as *const NativeHistory);
        let records = root.ivars().records.borrow();
        let record = &records[self.ivars().index];
        let selected = root.ivars().selected.borrow().as_deref() == Some(&record.id);
        let bounds: NSRect = objc2::msg_send![self, bounds];
        let shape: *mut AnyObject = objc2::msg_send![objc2::class!(NSBezierPath), bezierPathWithRoundedRect: rect(0.5,0.5,bounds.size.width-1.0,bounds.size.height-1.0), xRadius: 10.0_f64, yRadius: 10.0_f64];
        if selected || self.ivars().hovered.get() {
            let base: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
            let color: *mut AnyObject = objc2::msg_send![base, colorWithAlphaComponent: if selected {0.09_f64} else {0.04_f64}];
            let _: () = objc2::msg_send![color, setFill];
            let _: () = objc2::msg_send![shape, fill];
        }
        if record.is_pinned {
            let color: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
            let color: *mut AnyObject = objc2::msg_send![color, colorWithAlphaComponent: 0.3_f64];
            let _: () = objc2::msg_send![color, setStroke];
            let _: () = objc2::msg_send![shape, setLineWidth: 1.0_f64];
            let _: () = objc2::msg_send![shape, stroke];
        }
    }
    fn resize_content(&self, width: f64) -> f64 {
        unsafe {
            let preview = self.ivars().preview.get() as *mut AnyObject;
            let available = (width - 16.0).max(1.0);
            let cell: *mut AnyObject = objc2::msg_send![preview, cell];
            let measured: NSSize =
                objc2::msg_send![cell, cellSizeForBounds: rect(0.0,0.0,available,1000.0)];
            let text_height = measured.height.clamp(1.0, 60.0).ceil();
            let height = 30.0 + text_height + 8.0;
            frame(preview, rect(8.0, 30.0, available, text_height));
            let time = self.ivars().time.get() as *mut AnyObject;
            let time_cell: *mut AnyObject = objc2::msg_send![time, cell];
            let time_size: NSSize = objc2::msg_send![time_cell, cellSize];
            // Include the text cell's inset and round outward to avoid truncating HH:mm.
            let time_size = NSSize::new(time_size.width.ceil() + 4.0, time_size.height.ceil());
            frame(
                time,
                rect(
                    width - 8.0 - time_size.width,
                    16.0 - time_size.height / 2.0,
                    time_size.width,
                    time_size.height,
                ),
            );
            let reserved = if self.ivars().hovered.get() {
                if self.protected() {
                    100.0
                } else {
                    125.0
                }
            } else {
                time_size.width
            };
            let heading = self.ivars().heading.get() as *mut AnyObject;
            let heading_size: NSSize = objc2::msg_send![heading, intrinsicContentSize];
            frame(
                heading,
                rect(
                    29.0,
                    16.0 - heading_size.height / 2.0,
                    (width - 42.0 - reserved).max(0.0),
                    heading_size.height,
                ),
            );
            let count = if self.protected() { 4 } else { 5 };
            for (i, button) in self.ivars().actions.borrow().iter().enumerate() {
                let x = width - 5.0 - (count as f64 - i as f64) * 25.0;
                frame(
                    Retained::as_ptr(button).cast_mut(),
                    rect(x, 4.0, 24.0, 24.0),
                );
            }
            height
        }
    }
}

impl NativeHistory {
    fn update_search_appearance(&self) {
        let search = self.ivars().search.get() as *mut AnyObject;
        if search.is_null() {
            return;
        }
        unsafe {
            let container: *mut AnyObject = objc2::msg_send![search, superview];
            let layer: *mut AnyObject = objc2::msg_send![container, layer];
            let base: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), labelColor];
            let focused = self.ivars().search_focused.get();
            let border: *mut AnyObject = objc2::msg_send![base, colorWithAlphaComponent: if focused { 0.25_f64 } else { 0.12_f64 }];
            let background: *mut AnyObject =
                objc2::msg_send![base, colorWithAlphaComponent: 0.04_f64];
            let border: *mut AnyObject = objc2::msg_send![border, CGColor];
            let background: *mut AnyObject = objc2::msg_send![background, CGColor];
            let _: () = objc2::msg_send![objc2::class!(CATransaction), begin];
            let _: () = objc2::msg_send![objc2::class!(CATransaction), setDisableActions: true];
            let _: () = objc2::msg_send![layer, setBorderColor: border];
            let _: () =
                objc2::msg_send![layer, setBorderWidth: if focused { 2.0_f64 } else { 1.0_f64 }];
            let _: () = objc2::msg_send![layer, setBackgroundColor: background];
            let _: () = objc2::msg_send![objc2::class!(CATransaction), commit];
        }
    }
    unsafe fn add_row(&self, marker: objc2::MainThreadMarker, index: usize, record: &Record) {
        let row = HistoryRow::alloc(marker).set_ivars(RowState {
            root: self as *const Self as usize,
            index,
            hovered: Cell::new(false),
            tracking: RefCell::new(None),
            heading: Cell::new(0),
            time: Cell::new(0),
            preview: Cell::new(0),
            actions: RefCell::new(Vec::new()),
        });
        let row: Retained<HistoryRow> = objc2::msg_send![super(row),initWithFrame:NSRect::ZERO];
        let _: () = objc2::msg_send![&*row,setWantsLayer:true];
        let heading = format!(
            "{} - {}",
            language(&record.source_language),
            record
                .target_languages
                .iter()
                .map(|s| language(s))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let heading = label(&heading, 10.0, true);
        let _: () = objc2::msg_send![&*row,addSubview:&*heading];
        row.ivars().heading.set(Retained::as_ptr(&heading) as usize);
        let time = label(record.date.get(11..16).unwrap_or(""), 11.0, true);
        let _: () = objc2::msg_send![&*time, setAlignment: 1usize];
        let _: () = objc2::msg_send![&*row,addSubview:&*time];
        row.ivars().time.set(Retained::as_ptr(&time) as usize);
        let preview: String = record
            .text
            .replace("\r\n", "\n")
            .trim()
            .chars()
            .take(256)
            .collect();
        let preview = label(&preview, 13.0, false);
        let _: () = objc2::msg_send![&*preview,setMaximumNumberOfLines:3isize];
        let cell: *mut AnyObject = objc2::msg_send![&*preview, cell];
        let _: () = objc2::msg_send![cell,setWraps:true];
        let _: () = objc2::msg_send![cell,setScrollable:false];
        let _: () = objc2::msg_send![&*row,addSubview:&*preview];
        row.ivars().preview.set(Retained::as_ptr(&preview) as usize);
        let image: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSImageView), new];
        let icon = source_icon(record).unwrap_or_else(|| default_source_icon());
        let _: () = objc2::msg_send![&*image,setImage:&*icon];
        if let Some(name) = record
            .source_app
            .as_ref()
            .and_then(|app| app.get("name"))
            .and_then(|name| name.as_str())
        {
            let _: () = objc2::msg_send![&*image, setToolTip: &*NSString::from_str(name)];
        }
        let _: () = objc2::msg_send![&*image,setImageScaling:0usize];
        frame(
            Retained::as_ptr(&image).cast_mut(),
            rect(9.0, 9.0, 14.0, 14.0),
        );
        let _: () = objc2::msg_send![&*row,addSubview:&*image];
        for (action, symbol, title) in [
            (0, "languages", "Retranslate"),
            (1, "doc.on.doc", "Copy source text"),
            (2, "star", "Favorite"),
            (3, "pin", "Pin record"),
            (4, "trash", "Delete history record"),
        ] {
            let button = symbol_button(
                self,
                RECORD_TAG + index as isize * 10 + action,
                symbol,
                title,
            );
            if action == 4 {
                let red: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), systemRedColor];
                let _: () = objc2::msg_send![&*button, setContentTintColor: red];
            }
            let _: () = objc2::msg_send![&*button,setWantsLayer:true];
            let _: () = objc2::msg_send![&*button,setEnabled:false];
            let _: () = objc2::msg_send![&*button,setAccessibilityElement:false];
            let layer: *mut AnyObject = objc2::msg_send![&*button, layer];
            let _: () = objc2::msg_send![layer, setOpacity: 0.0_f32];
            let offset: *mut AnyObject =
                objc2::msg_send![objc2::class!(NSNumber), numberWithDouble: 8.0_f64];
            let _: () = objc2::msg_send![layer, setValue: offset, forKeyPath: objc2_foundation::ns_string!("transform.translation.x")];
            let _: () = objc2::msg_send![&*row,addSubview:&*button];
            row.ivars().actions.borrow_mut().push(button);
        }
        let accessible = NSString::from_str(&format!("Translation: {}", record.text));
        let _: () = objc2::msg_send![&*row,setAccessibilityElement:true];
        let _: () =
            objc2::msg_send![&*row,setAccessibilityRole:objc2_foundation::ns_string!("AXButton")];
        let _: () = objc2::msg_send![&*row,setAccessibilityLabel:&*accessible];
        self.ivars().rows.borrow_mut().push(row);
    }
    fn query(&self) -> String {
        unsafe {
            let value: Retained<NSString> =
                objc2::msg_send![self.ivars().search.get() as *mut AnyObject, stringValue];
            value.to_string().trim().to_lowercase()
        }
    }
    fn indices(&self, pinned: bool) -> Vec<usize> {
        let records = self.ivars().records.borrow();
        let mut indices: Vec<usize> = records
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                r.is_pinned == pinned
                    && (!self.ivars().favorites_only.get() || r.is_favorite)
                    && !r.id.is_empty()
            })
            .map(|(i, _)| i)
            .collect();
        let order = self.ivars().order.borrow();
        indices.sort_by(|&a, &b| {
            records[b]
                .date
                .cmp(&records[a].date)
                .then_with(|| order.get(&records[a].id).cmp(&order.get(&records[b].id)))
        });
        indices
    }

    fn select(&self, index: usize, retranslate: bool) {
        let id = self.ivars().records.borrow()[index].id.clone();
        if let Some(row) = self.ivars().rows.borrow().get(index) {
            unsafe {
                let frame: NSRect = objc2::msg_send![&**row, frame];
                let _: bool = objc2::msg_send![&**row, scrollRectToVisible: rect(0.0,0.0,frame.size.width,frame.size.height)];
            }
        }
        if crate::commands::select_history_sample(&self.ivars().app, Some(id), retranslate).is_err()
        {
            self.report_error();
        }
    }
    fn move_selection(&self, delta: isize) {
        let mut visible = self.indices(true);
        visible.extend(self.indices(false));
        if visible.is_empty() {
            return;
        }
        let selected = self.ivars().selected.borrow().clone();
        let records = self.ivars().records.borrow();
        let current = visible
            .iter()
            .position(|&i| Some(records[i].id.as_str()) == selected.as_deref());
        let next = current
            .map(|i| (i as isize + delta).clamp(0, visible.len() as isize - 1) as usize)
            .unwrap_or(if delta > 0 { 0 } else { visible.len() - 1 });
        drop(records);
        self.select(visible[next], false);
    }
    fn update_loading(&self) {
        let loading = self.ivars().query_pending.get() || self.ivars().load_pending.get();
        unsafe {
            let progress = self.ivars().progress.get() as *mut AnyObject;
            if !progress.is_null() {
                if loading {
                    let _: () =
                        objc2::msg_send![progress, startAnimation: std::ptr::null::<AnyObject>()];
                } else {
                    let _: () =
                        objc2::msg_send![progress, stopAnimation: std::ptr::null::<AnyObject>()];
                }
            }
            let label = self.ivars().empty_label.get() as *mut AnyObject;
            if !label.is_null() {
                let empty = self.indices(true).is_empty() && self.indices(false).is_empty();
                let _: () = objc2::msg_send![label, setHidden: loading || !empty];
            }
        }
    }

    fn request_query(&self) {
        let query = self.query();
        let favorites_only = self.ivars().favorites_only.get();
        if *self.ivars().requested_query.borrow() == Some((query.clone(), favorites_only)) {
            self.reload(true);
            return;
        }
        *self.ivars().requested_query.borrow_mut() = Some((query.clone(), favorites_only));
        let serial = self.ivars().query_serial.get().wrapping_add(1);
        self.ivars().query_serial.set(serial);
        self.ivars().query_pending.set(true);
        self.ivars().load_pending.set(false);
        self.ivars().has_more.set(false);
        self.reload(true);
        self.update_loading();
        let app = self.ivars().app.clone();
        tauri::async_runtime::spawn(async move {
            let result = crate::history_runtime::query_history(&app, query, favorites_only).await;
            let owned = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(window) = owned.get_window("translation") else {
                    return;
                };
                let Ok(root) = root(&window) else {
                    return;
                };
                if root.ivars().query_serial.get() != serial {
                    return;
                }
                root.ivars().query_pending.set(false);
                root.update_loading();
                if let Err(code) = result {
                    if code != "cancelled" {
                        *root.ivars().requested_query.borrow_mut() = None;
                        root.report_error();
                    }
                }
            });
        });
    }

    fn request_more(&self) {
        if self.ivars().suppress_load.get()
            || self.ivars().query_pending.get()
            || !self.ivars().has_more.get()
            || self.ivars().load_pending.replace(true)
        {
            return;
        }
        self.update_loading();
        let serial = self.ivars().query_serial.get();
        let app = self.ivars().app.clone();
        tauri::async_runtime::spawn(async move {
            let result = crate::history_runtime::load_more(&app).await;
            let owned = app.clone();
            let _ = app.run_on_main_thread(move || {
                let Some(window) = owned.get_window("translation") else {
                    return;
                };
                let Ok(root) = root(&window) else {
                    return;
                };
                if root.ivars().query_serial.get() != serial {
                    return;
                }
                root.ivars().load_pending.set(false);
                root.update_loading();
                if let Err(code) = result {
                    if code != "cancelled" {
                        root.report_error();
                    }
                }
            });
        });
    }

    fn report_error(&self) {
        if let Some(view) = self.ivars().app.get_webview("translation") {
            let _ = view.emit("layout_error", "native_history_action");
        }
    }
    fn perform(&self, tag: isize) {
        if tag == CLEAR_SEARCH_TAG {
            unsafe {
                let search = self.ivars().search.get() as *mut AnyObject;
                let empty = NSString::from_str("");
                let _: () = objc2::msg_send![search, setStringValue: &*empty];
                let window: *mut AnyObject = objc2::msg_send![self, window];
                let _: bool = objc2::msg_send![window, makeFirstResponder: search];
            }
            self.request_query();
            return;
        }
        if tag == SHORTCUTS_TAG {
            unsafe {
                let shortcuts = self.ivars().shortcuts.get() as *mut AnyObject;
                let hidden: bool = objc2::msg_send![shortcuts, isHidden];
                let _: () = objc2::msg_send![shortcuts, setHidden: !hidden];
                if hidden {
                    let window: *mut AnyObject = objc2::msg_send![self, window];
                    let _: bool = objc2::msg_send![window, makeFirstResponder: self];
                }
            }
            return;
        }
        if tag == 202 || tag == 203 {
            self.ivars().favorites_only.set(tag == 203);
            self.request_query();
            return;
        }
        if tag == 204 {
            {
                let app_state = self.ivars().app.state::<AppState>();
                let mut state = app_state.windows.lock().unwrap();
                state.history_pinned = !state.history_pinned;
            }
            if super::update_translation_level(&self.ivars().app).is_err() {
                self.report_error();
            }
            self.update_toolbar();
            return;
        }
        if tag < RECORD_TAG {
            return;
        }
        let index = ((tag - RECORD_TAG) / 10) as usize;
        let action = (tag - RECORD_TAG) % 10;
        if index >= self.ivars().records.borrow().len() {
            return;
        }
        if action == 0 {
            self.select(index, true);
            return;
        }
        if action == 1 {
            let id = self.ivars().records.borrow()[index].id.clone();
            let text = crate::history_runtime::history_record(&self.ivars().app, &id)
                .map(|record| record.text);
            match text.and_then(|text| platform::write_text(&text).ok()) {
                Some(version) => self
                    .ivars()
                    .app
                    .state::<AppState>()
                    .clipboard_version
                    .store(version, std::sync::atomic::Ordering::SeqCst),
                None => self.report_error(),
            }
            return;
        }
        let action = match action {
            2 => "favorite",
            3 => "pin",
            4 => "delete",
            _ => return,
        };
        let id = self.ivars().records.borrow()[index].id.clone();
        let app = self.ivars().app.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(code) = crate::commands::update_history_record(&app, &id, action).await {
                if code != "history_protected" {
                    if let Some(view) = app.get_webview("translation") {
                        let _ = view.emit("layout_error", &code);
                    }
                }
            }
        });
    }
    fn update_toolbar(&self) {
        self.setNeedsDisplay(true);
        unsafe {
            let pinned = self
                .ivars()
                .app
                .state::<AppState>()
                .windows
                .lock()
                .unwrap()
                .history_pinned;
            for (tag, active) in [
                (202, !self.ivars().favorites_only.get()),
                (203, self.ivars().favorites_only.get()),
                (204, pinned),
            ] {
                let button: *mut AnyObject = objc2::msg_send![self, viewWithTag: tag];
                let color: *mut AnyObject = if active {
                    objc2::msg_send![objc2::class!(NSColor), labelColor]
                } else {
                    objc2::msg_send![objc2::class!(NSColor), secondaryLabelColor]
                };
                if tag == 203 && self.ivars().favorite_icon_filled.replace(active) != active {
                    set_symbol(&*button, if active { "star.fill" } else { "star" });
                } else if tag == 204 && self.ivars().pin_icon_filled.replace(pinned) != pinned {
                    set_symbol(&*button, if pinned { "pin.fill" } else { "pin" });
                }
                let _: () = objc2::msg_send![button, setContentTintColor: color];
                let _: () = objc2::msg_send![button, setState: if active {1isize} else {0isize}];
                let label = NSString::from_str(if tag == 202 {
                    "All history"
                } else if tag == 203 {
                    "Favorite history"
                } else if pinned {
                    "Unpin window"
                } else {
                    "Pin history region"
                });
                let _: () = objc2::msg_send![button, setAccessibilityLabel: &*label];
            }
        }
    }
    fn reload(&self, top: bool) {
        self.ivars().suppress_load.set(true);
        unsafe {
            let clear: *mut AnyObject = objc2::msg_send![self, viewWithTag: CLEAR_SEARCH_TAG];
            let _: () = objc2::msg_send![clear, setHidden: self.query().is_empty()];
        }

        let pinned = self.indices(true);
        let ordinary = self.indices(false);
        unsafe {
            let count = self.ivars().total_records.get();
            let count_text = NSString::from_str(&format!(
                "{} {}",
                count,
                if count == 1 { "record" } else { "records" }
            ));
            let _: () = objc2::msg_send![self.ivars().record_count.get() as *mut AnyObject, setStringValue: &*count_text];
            for row in self.ivars().rows.borrow().iter() {
                let _: () = objc2::msg_send![&**row, removeFromSuperview];
            }
            for (indices, document) in [
                (&pinned, self.ivars().pinned_document.get()),
                (&ordinary, self.ivars().document.get()),
            ] {
                for &index in indices {
                    let row = self.ivars().rows.borrow()[index].clone();
                    let _: () = objc2::msg_send![document as *mut AnyObject, addSubview: &*row];
                }
            }
            for (index, row) in self.ivars().rows.borrow().iter().enumerate() {
                let record = self.ivars().records.borrow()[index].clone();
                let actions = row.ivars().actions.borrow();
                set_symbol(
                    &actions[2],
                    if record.is_favorite {
                        "star.fill"
                    } else {
                        "star"
                    },
                );
                set_symbol(
                    &actions[3],
                    if record.is_pinned { "pin.fill" } else { "pin" },
                );
                let _: () = objc2::msg_send![&*actions[4], setEnabled: row.ivars().hovered.get() && !record.is_favorite && !record.is_pinned];
                let _: () = objc2::msg_send![&*actions[4], setHidden: record.is_favorite || record.is_pinned];
                for (button, label) in [
                    (
                        &actions[2],
                        if record.is_favorite {
                            "Remove favorite"
                        } else {
                            "Favorite"
                        },
                    ),
                    (
                        &actions[3],
                        if record.is_pinned {
                            "Unpin record"
                        } else {
                            "Pin record"
                        },
                    ),
                ] {
                    let label = NSString::from_str(label);
                    let _: () = objc2::msg_send![&**button, setAccessibilityLabel: &*label];
                }
                row.setNeedsDisplay(true);
            }
            if top {
                for pointer in [self.ivars().pinned_scroll.get(), self.ivars().scroll.get()] {
                    let scroll = pointer as *mut AnyObject;
                    let clip: *mut AnyObject = objc2::msg_send![scroll, contentView];
                    let _: () = objc2::msg_send![clip, scrollToPoint: NSPoint::ZERO];
                    let _: () = objc2::msg_send![scroll, reflectScrolledClipView: clip];
                }
            }
        }
        unsafe {
            let empty = pinned.is_empty() && ordinary.is_empty();
            let label = self.ivars().empty_label.get() as *mut AnyObject;
            let message = if !self.query().is_empty() {
                "No matching translations"
            } else if self.ivars().favorites_only.get() {
                "No favorite translations"
            } else {
                "No translations yet"
            };
            let value = NSString::from_str(message);
            let _: () = objc2::msg_send![label, setStringValue: &*value];
            let loading = self.ivars().query_pending.get() || self.ivars().load_pending.get();
            let _: () = objc2::msg_send![label, setHidden: loading || !empty];
        }
        self.ivars().size.set(NSSize::ZERO);
        self.layout_content();
        self.update_toolbar();
        self.ivars().suppress_load.set(false);
    }
    fn layout_content(&self) {
        if self.ivars().divider.get() == 0 {
            return;
        }
        unsafe {
            let bounds: NSRect = objc2::msg_send![self, bounds];
            let size = bounds.size;
            if self.ivars().size.replace(size) == size {
                return;
            }
            self.ivars()
                .layout_count
                .set(self.ivars().layout_count.get() + 1);
            let wordmark: *mut AnyObject = objc2::msg_send![self, viewWithTag: WORDMARK_TAG];
            let wordmark_bounds: NSRect = objc2::msg_send![wordmark, frame];
            frame(
                wordmark,
                rect(
                    10.0,
                    24.0 - wordmark_bounds.size.height / 2.0,
                    wordmark_bounds.size.width,
                    wordmark_bounds.size.height,
                ),
            );
            let search_x = 10.0 + wordmark_bounds.size.width + 6.0;
            let search = self.ivars().search.get() as *mut AnyObject;
            let container: *mut AnyObject = objc2::msg_send![search, superview];
            let search_width = (size.width - search_x - 88.0).max(40.0);
            frame(container, rect(search_x, 10.0, search_width, 28.0));
            let cell: *mut AnyObject = objc2::msg_send![search, cell];
            let text_size: NSSize = objc2::msg_send![cell, cellSize];
            frame(
                search,
                rect(
                    28.0,
                    (28.0 - text_size.height) / 2.0,
                    (search_width - 52.0).max(1.0),
                    text_size.height,
                ),
            );
            let icon: *mut AnyObject = objc2::msg_send![self, viewWithTag: SEARCH_ICON_TAG];
            frame(icon, rect(8.0, 7.0, 14.0, 14.0));
            let clear: *mut AnyObject = objc2::msg_send![self, viewWithTag: CLEAR_SEARCH_TAG];
            frame(clear, rect(search_width - 24.0, 2.0, 24.0, 24.0));
            for (offset, tag) in [202isize, 203, 204].into_iter().enumerate() {
                let button: *mut AnyObject = objc2::msg_send![self, viewWithTag: tag];
                frame(
                    button,
                    rect(size.width - 86.0 + offset as f64 * 26.0, 12.0, 24.0, 24.0),
                );
            }
            let pinned = self.indices(true);
            let ordinary = self.indices(false);
            let width = (size.width - 20.0).max(1.0);
            let mut heights = [0.0_f64; 2];
            for (group, indices) in [&pinned, &ordinary].into_iter().enumerate() {
                let mut y = 4.0;
                for &index in indices {
                    let row = self.ivars().rows.borrow()[index].clone();
                    let height = row.resize_content(width);
                    frame(
                        Retained::as_ptr(&row).cast_mut().cast(),
                        rect(10.0, y, width, height),
                    );
                    y += height + 4.0;
                }
                heights[group] = if indices.is_empty() { 0.0 } else { y + 4.0 };
            }
            frame(
                self.ivars().empty_label.get() as *mut AnyObject,
                rect(24.0, size.height / 2.0 - 10.0, size.width - 48.0, 30.0),
            );
            let viewport_height = (size.height - 76.0).max(0.0);
            let pinned_limit = if ordinary.is_empty() {
                viewport_height
            } else {
                viewport_height * 0.45
            };
            let pinned_height = heights[0].min(pinned_limit);
            let gap = if pinned_height > 0.0 { 4.0 } else { 0.0 };
            let ordinary_height = (viewport_height - pinned_height - gap).max(0.0);
            frame(
                self.ivars().pinned_scroll.get() as *mut AnyObject,
                rect(0.0, 48.0, size.width, pinned_height),
            );
            frame(
                self.ivars().pinned_document.get() as *mut AnyObject,
                rect(0.0, 0.0, size.width, heights[0].max(pinned_height)),
            );
            let scroll = &*(self.ivars().scroll.get() as *const HistoryScrollView);
            let inset_changed =
                scroll.ivars().top_inset.replace(pinned_height + gap) != pinned_height + gap;
            let _: () = objc2::msg_send![scroll, setHidden: ordinary.is_empty()];
            frame(
                self.ivars().scroll.get() as *mut AnyObject,
                rect(0.0, 48.0, size.width, viewport_height),
            );
            if inset_changed {
                let _: () = objc2::msg_send![scroll, tile];
            }
            frame(
                self.ivars().document.get() as *mut AnyObject,
                rect(0.0, 0.0, size.width, heights[1].max(ordinary_height)),
            );
            frame(
                self.ivars().progress.get() as *mut AnyObject,
                rect(size.width - 52.0, size.height - 24.0, 16.0, 16.0),
            );
            let count = self.ivars().record_count.get() as *mut AnyObject;
            let _: () = objc2::msg_send![count, sizeToFit];
            let count_frame: NSRect = objc2::msg_send![count, frame];
            frame(
                count,
                rect(
                    12.0,
                    size.height - 16.0 - count_frame.size.height / 2.0,
                    size.width - 60.0,
                    count_frame.size.height,
                ),
            );
            let button: *mut AnyObject = objc2::msg_send![self, viewWithTag: SHORTCUTS_TAG];
            frame(
                button,
                rect(size.width - 36.0, size.height - 28.0, 24.0, 24.0),
            );
            frame(
                self.ivars().shortcuts.get() as *mut AnyObject,
                rect(size.width - 242.0, size.height - 176.0, 230.0, 136.0),
            );
            frame(
                self.ivars().divider.get() as *mut AnyObject,
                rect(
                    size.width - 6.0,
                    size.height * 0.07,
                    6.0,
                    size.height * 0.86,
                ),
            );
        }
    }
}

pub(super) fn create(app: &AppHandle) -> Result<Retained<NSView>, String> {
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    let snapshot = app.state::<AppState>().history.lock().unwrap().snapshot();
    let order = snapshot
        .records
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect();
    let root = NativeHistory::alloc(marker).set_ivars(HistoryState {
        app: app.clone(),
        records: RefCell::new(snapshot.records),
        order: RefCell::new(order),
        selected: RefCell::new(snapshot.selected_id),
        generation: Cell::new(snapshot.generation),
        total_records: Cell::new(snapshot.total_records),
        has_more: Cell::new(snapshot.has_more),
        query_pending: Cell::new(false),
        load_pending: Cell::new(false),
        query_serial: Cell::new(0),
        requested_query: RefCell::new(Some((String::new(), false))),
        suppress_load: Cell::new(false),
        favorites_only: Cell::new(false),
        favorite_icon_filled: Cell::new(false),
        pin_icon_filled: Cell::new(false),
        search: Cell::new(0),
        search_focused: Cell::new(false),
        scroll: Cell::new(0),
        document: Cell::new(0),
        pinned_scroll: Cell::new(0),
        pinned_document: Cell::new(0),
        record_count: Cell::new(0),
        progress: Cell::new(0),
        shortcuts: Cell::new(0),
        rows: RefCell::new(Vec::new()),
        size: Cell::new(NSSize::ZERO),
        divider: Cell::new(0),
        empty_label: Cell::new(0),
        layout_count: Cell::new(0),
    });
    let root: Retained<NativeHistory> =
        unsafe { objc2::msg_send![super(root), initWithFrame: rect(0.0,0.0,360.0,620.0)] };
    unsafe {
        let _: () = objc2::msg_send![&*root, setAutoresizingMask: 0usize];
        let _: () = objc2::msg_send![&*root, setWantsLayer: true];
        let layer: *mut AnyObject = objc2::msg_send![&*root, layer];
        let _: () = objc2::msg_send![layer, setMasksToBounds: true];
        let wordmark: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSImageView), new];
        let bytes = include_bytes!("../../../src/assets/history/reference/nami-wordmark.svg");
        let data: *mut AnyObject = objc2::msg_send![objc2::class!(NSData), dataWithBytes: bytes.as_ptr(), length: bytes.len()];
        let allocated: objc2::rc::Allocated<AnyObject> =
            objc2::msg_send![objc2::class!(NSImage), alloc];
        let mark: Retained<AnyObject> = objc2::msg_send![allocated, initWithData: data];
        let _: () = objc2::msg_send![&*mark, setSize: NSSize::new(44.0, 44.0 * 280.0 / 680.0)];
        let _: () = objc2::msg_send![&*mark, setTemplate: false];
        let _: () = objc2::msg_send![&*wordmark, setImage: &*mark];
        let white: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), whiteColor];
        let _: () = objc2::msg_send![&*wordmark, setContentTintColor: white];
        let _: () = objc2::msg_send![&*wordmark, setImageScaling: 0usize];
        let _: () = objc2::msg_send![&*wordmark, setAccessibilityLabel: objc2_foundation::ns_string!("Nami")];
        let _: () = objc2::msg_send![&*wordmark, setTag: WORDMARK_TAG];
        frame(
            Retained::as_ptr(&wordmark).cast_mut(),
            rect(10.0, 14.0, 44.0, 20.0),
        );
        let _: () = objc2::msg_send![&*root, addSubview: &*wordmark];
        let search = HistorySearchField::alloc(marker).set_ivars(Retained::as_ptr(&root) as usize);
        let search: Retained<HistorySearchField> = objc2::msg_send![super(search), init];
        let container: Retained<NSView> = objc2::msg_send![objc2::class!(NSView), new];
        let _: () = objc2::msg_send![&*container, setWantsLayer: true];
        let _: () = objc2::msg_send![&*container, addSubview: &*search];
        let _: () = objc2::msg_send![&*root, addSubview: &*container];
        let _: () = objc2::msg_send![&*search, setEditable: true];
        let _: () = objc2::msg_send![&*search, setSelectable: true];
        let _: () = objc2::msg_send![&*search, setUsesSingleLineMode: true];
        let placeholder = NSString::from_str("Search source or translation");
        let _: () = objc2::msg_send![&*search, setTag: SEARCH_TAG];
        let _: () = objc2::msg_send![&*search, setBezeled: false];
        let _: () = objc2::msg_send![&*search, setDrawsBackground: false];
        let _: () = objc2::msg_send![&*search, setWantsLayer: true];
        let search_layer: *mut AnyObject = objc2::msg_send![&*container, layer];
        let _: () = objc2::msg_send![search_layer, setCornerRadius: 10.0_f64];
        let _: () = objc2::msg_send![search_layer, setBorderWidth: 1.0_f64];
        let _: () = objc2::msg_send![&*search, setPlaceholderString: &*placeholder];
        let font: *mut AnyObject =
            objc2::msg_send![objc2::class!(NSFont), systemFontOfSize: 12.0_f64];
        let _: () = objc2::msg_send![&*search, setFont: font];
        let _: () = objc2::msg_send![&*search, setFocusRingType:1usize];
        let _: () = objc2::msg_send![&*search, setDelegate: &*root];

        let _: () = objc2::msg_send![&*search, setAccessibilityLabel:&*placeholder];
        let icon: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSImageView), new];
        let _: () = objc2::msg_send![&*icon, setTag: SEARCH_ICON_TAG];
        set_symbol(&icon, "magnifyingglass");
        let _: () = objc2::msg_send![&*icon, setImageScaling: 2usize];
        let _: () = objc2::msg_send![&*container, addSubview: &*icon];
        let clear = symbol_button(&root, CLEAR_SEARCH_TAG, "xmark.circle.fill", "Clear search");
        let _: () = objc2::msg_send![&*clear, setHidden: true];
        let _: () = objc2::msg_send![&*container, addSubview: &*clear];
        root.ivars().search.set(Retained::as_ptr(&search) as usize);
        root.update_search_appearance();
        for (tag, symbol, title) in [
            (202, "list.bullet", "All history"),
            (203, "star", "Favorite history"),
            (204, "pin", "Pin history region"),
        ] {
            let button = symbol_button(&root, tag, symbol, title);
            let _: () = objc2::msg_send![&*root,addSubview:&*button];
        }
        for pinned in [true, false] {
            let allocated = HistoryScrollView::alloc(marker).set_ivars(HistoryScrollState {
                top_inset: Cell::new(0.0),
            });
            let scroll: Retained<HistoryScrollView> =
                objc2::msg_send![super(allocated), initWithFrame:NSRect::ZERO];
            let clip: *mut AnyObject = objc2::msg_send![&*scroll, contentView];
            let _: () = objc2::msg_send![clip, setWantsLayer: true];
            let layer: *mut AnyObject = objc2::msg_send![clip, layer];
            let mask: *mut AnyObject = objc2::msg_send![objc2::class!(CAGradientLayer), layer];
            let colors: Retained<AnyObject> = objc2::msg_send![objc2::class!(NSMutableArray), new];
            for alpha in [0.0_f64, 1.0, 1.0, 0.0] {
                let color: *mut AnyObject =
                    objc2::msg_send![objc2::class!(NSColor), colorWithWhite: 0.0_f64, alpha: alpha];
                let cg_color: *mut AnyObject = objc2::msg_send![color, CGColor];
                let _: () = objc2::msg_send![&*colors, addObject: cg_color];
            }
            let _: () = objc2::msg_send![mask, setColors: &*colors];
            let _: () = objc2::msg_send![mask, setStartPoint: NSPoint::new(0.5, 0.0)];
            let _: () = objc2::msg_send![mask, setEndPoint: NSPoint::new(0.5, 1.0)];
            let _: () = objc2::msg_send![layer, setMask: mask];
            let _: () = objc2::msg_send![&*scroll, setDrawsBackground:false];
            let _: () = objc2::msg_send![&*scroll, setBorderType:0usize];
            let _: () = objc2::msg_send![&*scroll, setHasVerticalScroller:!pinned];
            let _: () = objc2::msg_send![&*scroll, setAutohidesScrollers:true];
            let _: () = objc2::msg_send![&*scroll, setScrollerStyle:1usize];
            let document = HistoryDocument::alloc(marker).set_ivars(());
            let document: Retained<HistoryDocument> =
                objc2::msg_send![super(document), initWithFrame:NSRect::ZERO];
            let _: () = objc2::msg_send![&*scroll, setDocumentView:&*document];
            let _: () = objc2::msg_send![&*root, addSubview:&*scroll];
            if pinned {
                root.ivars()
                    .pinned_scroll
                    .set(Retained::as_ptr(&scroll) as usize);
                root.ivars()
                    .pinned_document
                    .set(Retained::as_ptr(&document) as usize);
            } else {
                root.ivars().scroll.set(Retained::as_ptr(&scroll) as usize);
                root.ivars()
                    .document
                    .set(Retained::as_ptr(&document) as usize);
            }
        }
        let empty = label("No translations yet", 12.0, true);
        let _: () = objc2::msg_send![&*empty, setAlignment: 1usize];
        let _: () = objc2::msg_send![&*root, addSubview: &*empty];
        root.ivars()
            .empty_label
            .set(Retained::as_ptr(&empty) as usize);
        for (index, record) in root.ivars().records.borrow().iter().enumerate() {
            root.add_row(marker, index, record);
        }
        let divider = HistoryDivider::alloc(marker).set_ivars(DividerState {
            app: app.clone(),
            last_x: Cell::new(0.0),
        });
        let divider: Retained<HistoryDivider> =
            objc2::msg_send![super(divider),initWithFrame:NSRect::ZERO];
        let _: () = objc2::msg_send![&*divider,setAccessibilityElement:true];
        let _: () = objc2::msg_send![&*divider,setAccessibilityRole:objc2_foundation::ns_string!("AXSplitter")];
        let _: () = objc2::msg_send![&*divider,setAccessibilityLabel:objc2_foundation::ns_string!("Resize translation history")];
        let _: () = objc2::msg_send![&*root,addSubview:&*divider];
        root.ivars()
            .divider
            .set(Retained::as_ptr(&divider) as usize);
        let count = label("", 11.0, true);
        let _: () = objc2::msg_send![&*root, addSubview: &*count];
        root.ivars()
            .record_count
            .set(Retained::as_ptr(&count) as usize);
        let progress: Retained<AnyObject> =
            objc2::msg_send![objc2::class!(NSProgressIndicator), new];
        let _: () = objc2::msg_send![&*progress, setStyle: 1usize];
        let _: () = objc2::msg_send![&*progress, setControlSize: 1usize];
        let _: () = objc2::msg_send![&*progress, setIndeterminate: true];
        let _: () = objc2::msg_send![&*progress, setDisplayedWhenStopped: false];
        let _: () = objc2::msg_send![&*progress, setAccessibilityLabel: objc2_foundation::ns_string!("Loading history")];
        let _: () = objc2::msg_send![&*root, addSubview: &*progress];
        root.ivars()
            .progress
            .set(Retained::as_ptr(&progress) as usize);
        let keyboard = symbol_button(&root, SHORTCUTS_TAG, "keyboard", "Keyboard shortcuts");
        let color: *mut AnyObject = objc2::msg_send![objc2::class!(NSColor), secondaryLabelColor];
        let _: () = objc2::msg_send![&*keyboard, setContentTintColor: color];
        let _: () = objc2::msg_send![&*root, addSubview: &*keyboard];

        let shortcuts: Retained<AnyObject> =
            objc2::msg_send![objc2::class!(NSVisualEffectView), new];
        let _: () = objc2::msg_send![&*shortcuts, setMaterial: 6isize];
        let _: () = objc2::msg_send![&*shortcuts, setBlendingMode: 1isize];
        let _: () = objc2::msg_send![&*shortcuts, setWantsLayer: true];
        let layer: *mut AnyObject = objc2::msg_send![&*shortcuts, layer];
        let _: () = objc2::msg_send![layer, setCornerRadius: 10.0_f64];
        let _: () = objc2::msg_send![layer, setMasksToBounds: true];
        let title = label("Keyboard shortcuts", 12.0, false);
        let bold: *mut AnyObject =
            objc2::msg_send![objc2::class!(NSFont), boldSystemFontOfSize: 12.0_f64];
        let _: () = objc2::msg_send![&*title, setFont: bold];
        frame(
            Retained::as_ptr(&title).cast_mut(),
            rect(14.0, 107.0, 202.0, 18.0),
        );
        let _: () = objc2::msg_send![&*shortcuts, addSubview: &*title];
        for (y, description, keys) in [
            (81.0, "Navigate history", "↑ / ↓"),
            (57.0, "Open selection", "Return"),
            (33.0, "Hide window", "Esc / ⌘ W"),
        ] {
            let description = label(description, 11.0, false);
            frame(
                Retained::as_ptr(&description).cast_mut(),
                rect(14.0, y, 126.0, 17.0),
            );
            let _: () = objc2::msg_send![&*shortcuts, addSubview: &*description];
            let keys = label(keys, 11.0, true);
            let _: () = objc2::msg_send![&*keys, setAlignment: 1usize];
            frame(
                Retained::as_ptr(&keys).cast_mut(),
                rect(140.0, y, 76.0, 17.0),
            );
            let _: () = objc2::msg_send![&*shortcuts, addSubview: &*keys];
        }
        let hint = label("Select a record to navigate", 10.0, true);
        frame(
            Retained::as_ptr(&hint).cast_mut(),
            rect(14.0, 10.0, 202.0, 16.0),
        );
        let _: () = objc2::msg_send![&*shortcuts, addSubview: &*hint];
        let _: () = objc2::msg_send![&*shortcuts, setHidden: true];
        let _: () = objc2::msg_send![&*root, addSubview: &*shortcuts];
        root.ivars()
            .shortcuts
            .set(Retained::as_ptr(&shortcuts) as usize);
    }
    root.reload(false);
    Ok(Retained::into_super(root))
}

fn root(window: &Window) -> Result<Retained<NativeHistory>, String> {
    let stage = translation_stage(window)?;
    let pointer = unsafe { objc2::msg_send![&*stage,viewWithTag:ROOT_TAG] };
    unsafe { Retained::retain(pointer).ok_or("window_geometry".into()) }
}

pub(super) fn apply_snapshot(
    window: &Window,
    snapshot: &crate::history::HistorySnapshot,
) -> Result<(), String> {
    let marker = objc2::MainThreadMarker::new().ok_or("window_geometry")?;
    let root = root(window)?;
    *root.ivars().order.borrow_mut() = snapshot
        .records
        .iter()
        .enumerate()
        .map(|(i, r)| (r.id.clone(), i))
        .collect();
    let reset = root.ivars().generation.replace(snapshot.generation) != snapshot.generation;
    let count_changed =
        root.ivars().total_records.replace(snapshot.total_records) != snapshot.total_records;
    root.ivars().has_more.set(snapshot.has_more);
    root.ivars().suppress_load.set(true);
    if reset {
        for row in root.ivars().rows.borrow().iter() {
            unsafe {
                let _: () = objc2::msg_send![&**row, removeFromSuperview];
            }
        }
        root.ivars().rows.borrow_mut().clear();
        root.ivars().records.borrow_mut().clear();
    }
    let incoming: HashMap<&str, &Record> = snapshot
        .records
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect();
    let mut changed = reset || count_changed;
    let mut known = HashSet::new();
    {
        let mut records = root.ivars().records.borrow_mut();
        for projected in records.iter_mut().filter(|record| !record.id.is_empty()) {
            if let Some(record) = incoming.get(projected.id.as_str()) {
                changed |= projected.is_favorite != record.is_favorite
                    || projected.is_pinned != record.is_pinned;
                // Translation content is immutable; flag/selection updates do not clone its text or icon.
                projected.is_favorite = record.is_favorite;
                projected.is_pinned = record.is_pinned;
                known.insert(projected.id.clone());
            } else {
                // Preserve row indices and native instances within the loaded query.
                projected.id.clear();
                changed = true;
            }
        }
    }
    for record in &snapshot.records {
        if known.insert(record.id.clone()) {
            let index = root.ivars().records.borrow().len();
            root.ivars().records.borrow_mut().push(record.clone());
            unsafe {
                root.add_row(marker, index, record);
            }
            changed = true;
        }
    }
    let previous = root.ivars().selected.replace(snapshot.selected_id.clone());
    if changed {
        root.reload(reset);
    } else if previous != snapshot.selected_id {
        let records = root.ivars().records.borrow();
        for (record, row) in records.iter().zip(root.ivars().rows.borrow().iter()) {
            if previous.as_deref() == Some(record.id.as_str())
                || snapshot.selected_id.as_deref() == Some(record.id.as_str())
            {
                row.setNeedsDisplay(true);
            }
        }
    }
    root.ivars().suppress_load.set(false);
    Ok(())
}

pub(super) fn sync(window: &Window, history_visible: bool) -> Result<(), String> {
    let root = root(window)?;
    let app_state = window.app_handle().state::<AppState>();
    let state = app_state.windows.lock().unwrap().clone();
    unsafe {
        let _: () =
            objc2::msg_send![&*root,setHidden:!history_visible && state.history_extent<=0.0];
        let _: () = objc2::msg_send![root.ivars().divider.get() as *mut AnyObject,setHidden:!state.translation_visible || !state.history_visible];
        if !history_visible {
            let _: () =
                objc2::msg_send![root.ivars().shortcuts.get() as *mut AnyObject, setHidden: true];
        }
    }
    root.update_toolbar();
    Ok(())
}

#[cfg(feature = "ui-tests")]
pub(super) fn snapshot(window: &Window) -> Result<serde_json::Value, String> {
    let root = root(window)?;
    let pinned = root.indices(true);
    let ordinary = root.indices(false);
    let records = root.ivars().records.borrow();
    let mut rows = Vec::new();
    for (index, row) in root.ivars().rows.borrow().iter().enumerate() {
        unsafe {
            let f: NSRect = objc2::msg_send![&**row, frame];
            let parent: *mut AnyObject = objc2::msg_send![&**row, superview];
            let bounds: NSRect = objc2::msg_send![&**row, bounds];
            let viewport: NSRect = objc2::msg_send![&**row, convertRect: bounds, toView: &*root];
            let actions = row.ivars().actions.borrow().iter().map(|button| {
                let frame: NSRect = objc2::msg_send![&**button, frame];
                let hidden: bool = objc2::msg_send![&**button, isHidden];
                let enabled: bool = objc2::msg_send![&**button, isEnabled];
                let image: *mut AnyObject = objc2::msg_send![&**button, image];
                let image_size: NSSize = objc2::msg_send![image, size];
                let cell: *mut AnyObject = objc2::msg_send![&**button, cell];
                let bounds: NSRect = objc2::msg_send![&**button, bounds];
                let drawn: NSRect = objc2::msg_send![cell, imageRectForBounds: bounds];
                let layer: *mut AnyObject = objc2::msg_send![&**button, layer];
                let opacity: f32 = objc2::msg_send![layer, opacity];
                let offset: *mut AnyObject = objc2::msg_send![layer, valueForKeyPath: objc2_foundation::ns_string!("transform.translation.x")];
                let offset: f64 = objc2::msg_send![offset, doubleValue];
                let animation: *mut AnyObject = objc2::msg_send![layer, animationForKey: objc2_foundation::ns_string!("historyActions")];
                let duration: f64 = if animation.is_null() { 0.0 } else { objc2::msg_send![animation, duration] };
                serde_json::json!({"x":frame.origin.x,"y":frame.origin.y,"width":frame.size.width,"height":frame.size.height,"hidden":hidden,"enabled":enabled,"opacity":opacity,"offset":offset,"animationDuration":duration,"drawnWidth":drawn.size.width,"drawnHeight":drawn.size.height,"imageWidth":image_size.width,"imageHeight":image_size.height})
            }).collect::<Vec<_>>();
            let time = row.ivars().time.get() as *mut AnyObject;
            let time_frame: NSRect = objc2::msg_send![time, frame];
            let heading_frame: NSRect =
                objc2::msg_send![row.ivars().heading.get() as *mut AnyObject, frame];
            let time_cell: *mut AnyObject = objc2::msg_send![time, cell];
            let time_size: NSSize = objc2::msg_send![time_cell, cellSize];
            rows.push(serde_json::json!({"id":records[index].id,"pinned":records[index].is_pinned,"favorite":records[index].is_favorite,"x":f.origin.x,"y":f.origin.y,"width":f.size.width,"height":f.size.height,"parentPointer":parent as usize,"viewportY":viewport.origin.y,"pointer":Retained::as_ptr(row) as usize,"hidden":parent.is_null(),"actions":actions,"headingCenterY":heading_frame.origin.y+heading_frame.size.height/2.0,"timeCenterY":time_frame.origin.y+time_frame.size.height/2.0,"timeWidth":time_frame.size.width,"timeRequiredWidth":time_size.width,"timeHeight":time_frame.size.height,"timeRequiredHeight":time_size.height}));
        }
    }
    let mut toolbar = Vec::new();
    unsafe {
        for tag in [202isize, 203, 204] {
            let button: *mut AnyObject = objc2::msg_send![&*root, viewWithTag: tag];
            let cell: *mut AnyObject = objc2::msg_send![button, cell];
            let bounds: NSRect = objc2::msg_send![button, bounds];
            let drawn: NSRect = objc2::msg_send![cell, imageRectForBounds: bounds];
            toolbar.push(serde_json::json!({"tag":tag,"drawnWidth":drawn.size.width,"drawnHeight":drawn.size.height}));
        }
    }
    let scroll_y;
    let scrollbar_y;
    let pinned_y;
    let count_text;
    let shortcuts_hidden: bool;
    unsafe {
        let scroll = root.ivars().scroll.get() as *mut AnyObject;
        let clip: *mut AnyObject = objc2::msg_send![scroll, contentView];
        let bounds: NSRect = objc2::msg_send![clip, bounds];
        scroll_y = bounds.origin.y;
        let scroller: *mut AnyObject = objc2::msg_send![scroll, verticalScroller];
        let bounds: NSRect = objc2::msg_send![scroller, bounds];
        let scroller_frame: NSRect =
            objc2::msg_send![scroller, convertRect: bounds, toView: &*root];
        scrollbar_y = scroller_frame.origin.y;
        let pinned_frame: NSRect =
            objc2::msg_send![root.ivars().pinned_scroll.get() as *mut AnyObject, frame];
        pinned_y = pinned_frame.origin.y;
        let text: Retained<NSString> = objc2::msg_send![
            root.ivars().record_count.get() as *mut AnyObject,
            stringValue
        ];
        count_text = text.to_string();
        shortcuts_hidden =
            objc2::msg_send![root.ivars().shortcuts.get() as *mut AnyObject, isHidden];
    }
    Ok(
        serde_json::json!({"generation":root.ivars().generation.get(),"hasMore":root.ivars().has_more.get(),"totalRecords":root.ivars().total_records.get(),"suppressLoad":root.ivars().suppress_load.get(),"queryPending":root.ivars().query_pending.get(),"loadPending":root.ivars().load_pending.get(),"toolbar":toolbar,"rootPointer":Retained::as_ptr(&root) as usize,"layoutCount":root.ivars().layout_count.get(),"scrollY":scroll_y,"scrollbarY":scrollbar_y,"pinnedY":pinned_y,"countText":count_text,"shortcutsHidden":shortcuts_hidden,"query":root.query(),"favoritesOnly":root.ivars().favorites_only.get(),"selected":root.ivars().selected.borrow().clone(),"pinnedCount":pinned.len(),"ordinaryCount":ordinary.len(),"rows":rows}),
    )
}
