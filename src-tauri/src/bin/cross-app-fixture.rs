//! Standalone AppKit target for Nami's isolated cross-app checks.
#[cfg(target_os = "macos")]
mod macos {
    use objc2::{
        class, msg_send,
        rc::Retained,
        runtime::{AnyObject, Sel},
        sel, ClassType, DefinedClass, MainThreadMarker, MainThreadOnly,
    };
    use objc2_app_kit::{
        NSApplication, NSBackingStoreType, NSTextView, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{ns_string, NSPoint, NSRange, NSRect, NSSize, NSString};
    use std::{
        cell::Cell,
        fmt,
        io::{self, Write},
        ptr::null_mut,
        time::{Duration, Instant},
    };

    const SELECTION: &str = "NAMI synthetic selection";

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum Mode {
        Ax,
        ManualAx,
        Copy,
        CopyDelayed,
        Paste,
        Empty,
    }

    impl Mode {
        fn parse(value: &str) -> Option<Self> {
            match value {
                "ax" => Some(Self::Ax),
                "manual-ax" => Some(Self::ManualAx),
                "copy" => Some(Self::Copy),
                "copy-delayed" => Some(Self::CopyDelayed),
                "paste" => Some(Self::Paste),
                "empty" => Some(Self::Empty),
                _ => None,
            }
        }

        fn initial(self) -> (&'static str, NSRange) {
            match self {
                Self::Ax | Self::ManualAx | Self::Copy | Self::CopyDelayed => {
                    (SELECTION, NSRange::new(0, SELECTION.len()))
                }
                Self::Paste => ("[]", NSRange::new(1, 0)),
                Self::Empty => (SELECTION, NSRange::new(0, 0)),
            }
        }
    }

    fn report(line: fmt::Arguments<'_>) {
        let mut out = io::stdout().lock();
        if let Err(error) = writeln!(out, "{line}").and_then(|()| out.flush()) {
            eprintln!("cross-app-fixture stdout: {error}");
            std::process::exit(1);
        }
    }

    thread_local! {
        static AX_READY: Cell<Option<Instant>> = const { Cell::new(None) };
    }

    objc2::define_class!(
        #[unsafe(super(NSApplication))]
        #[thread_kind = MainThreadOnly]
        struct ColdAxApplication;

        impl ColdAxApplication {
            #[unsafe(method_id(accessibilityAttributeNames))]
            fn ax_names(&self) -> Retained<AnyObject> {
                unsafe {
                    let names: Retained<AnyObject> = msg_send![super(self), accessibilityAttributeNames];
                    let names: Retained<AnyObject> = msg_send![&*names, mutableCopy];
                    let _: () = msg_send![&*names, addObject: ns_string!("AXManualAccessibility")];
                    names
                }
            }

            #[unsafe(method(accessibilityIsAttributeSettable:))]
            fn ax_settable(&self, name: &NSString) -> bool {
                name == ns_string!("AXManualAccessibility")
                    || unsafe { msg_send![super(self), accessibilityIsAttributeSettable: name] }
            }

            #[unsafe(method_id(accessibilityAttributeValue:))]
            fn ax_value(&self, name: &NSString) -> Option<Retained<AnyObject>> {
                unsafe {
                    if name == ns_string!("AXManualAccessibility") {
                        msg_send![class!(NSNumber), numberWithBool: AX_READY.get().is_some()]
                    } else {
                        msg_send![super(self), accessibilityAttributeValue: name]
                    }
                }
            }

            #[unsafe(method(accessibilitySetValue:forAttribute:))]
            fn ax_set_value(&self, value: &AnyObject, name: &NSString) {
                unsafe {
                    if name == ns_string!("AXManualAccessibility") {
                        let enabled: bool = msg_send![value, boolValue];
                        if enabled {
                            AX_READY.set(Some(Instant::now() + Duration::from_millis(120)));
                            report(format_args!("AX ENABLED"));
                        }
                    } else {
                        let _: () = msg_send![super(self), accessibilitySetValue: value, forAttribute: name];
                    }
                }
            }
        }
    );

    struct EditorState {
        board: Retained<AnyObject>,
        mode: Mode,
        smoke: bool,
        copies: Cell<usize>,
        pastes: Cell<usize>,
    }

    objc2::define_class!(
        #[unsafe(super(NSTextView))]
        #[thread_kind = MainThreadOnly]
        #[ivars = EditorState]
        struct FixtureEditor;

        impl FixtureEditor {
            #[unsafe(method(isAccessibilitySelectorAllowed:))]
            fn allows_ax_selector(&self, selector: Sel) -> bool {
                if self.hides_ax_selection() && matches_ax_selection(selector) {
                    false
                } else {
                    unsafe { msg_send![super(self), isAccessibilitySelectorAllowed: selector] }
                }
            }

            #[unsafe(method_id(accessibilityAttributeValue:))]
            fn ax_attribute(&self, attribute: &NSString) -> Option<Retained<AnyObject>> {
                // NSTextView also serves legacy attribute reads; missing is distinct from empty.
                if self.hides_ax_selection() && [
                    ns_string!("AXSelectedText"),
                    ns_string!("AXSelectedTextRange"),
                    ns_string!("AXSelectedTextRanges"),
                ].contains(&attribute) {
                    None
                } else {
                    unsafe { msg_send![super(self), accessibilityAttributeValue: attribute] }
                }
            }

            #[unsafe(method(copy:))]
            fn copy(&self, _: *mut AnyObject) {
                let range = self.selection();
                if range.length == 0 { return; }
                unsafe {
                    let text: Retained<NSString> = msg_send![&*self.text(), substringWithRange: range];
                    let board = &*self.ivars().board;
                    let _: isize = msg_send![board, clearContents];
                    if self.ivars().mode == Mode::CopyDelayed {
                        // Reproduce applications that advance changeCount on
                        // clearContents and write the text a frame later.
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    let written: bool = msg_send![board, setString: &*text, forType: text_type()];
                    if written {
                        self.ivars().copies.set(self.ivars().copies.get() + 1);
                        self.update_title();
                        if self.ivars().smoke {
                            report(format_args!("COPY {text}"));
                        }
                    }
                }
            }

            #[unsafe(method(paste:))]
            fn paste(&self, _: *mut AnyObject) { self.paste_from_board(); }

            #[unsafe(method(validateMenuItem:))]
            fn validate_menu_item(&self, item: &AnyObject) -> bool {
                // NSTextView's standard Paste validation consults the general pasteboard.
                let action: Option<Sel> = unsafe { msg_send![item, action] };
                if action == Some(sel!(copy:)) {
                    self.selection().length > 0
                } else {
                    action == Some(sel!(paste:)) && board_text(&self.ivars().board).is_some_and(|text| !text.is_empty())
                }
            }

            #[unsafe(method(cut:))]
            fn cut(&self, _: *mut AnyObject) {}

            #[unsafe(method(menuForEvent:))]
            fn menu_for_event(&self, _: &AnyObject) -> *mut AnyObject { null_mut() }

            #[unsafe(method(pasteAsPlainText:))]
            fn paste_as_text(&self, _: *mut AnyObject) { self.paste_from_board(); }

            #[unsafe(method(pasteAsRichText:))]
            fn paste_as_rich_text(&self, _: *mut AnyObject) { self.paste_from_board(); }
        }
    );

    fn matches_ax_selection(selector: Sel) -> bool {
        selector == sel!(accessibilitySelectedText)
            || selector == sel!(accessibilitySelectedTextRange)
            || selector == sel!(accessibilitySelectedTextRanges)
    }

    fn text_type() -> &'static NSString {
        ns_string!("public.utf8-plain-text")
    }

    fn board_text(board: &AnyObject) -> Option<Retained<NSString>> {
        unsafe { msg_send![board, stringForType: text_type()] }
    }

    impl FixtureEditor {
        fn new(
            mtm: MainThreadMarker,
            size: NSRect,
            board: Retained<AnyObject>,
            mode: Mode,
            smoke: bool,
        ) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(EditorState {
                board,
                mode,
                smoke,
                copies: Cell::new(0),
                pastes: Cell::new(0),
            });
            unsafe { msg_send![super(this), initWithFrame: size] }
        }

        fn hides_ax_selection(&self) -> bool {
            matches!(self.ivars().mode, Mode::Copy | Mode::CopyDelayed)
                || (self.ivars().mode == Mode::ManualAx
                    && AX_READY.get().is_none_or(|ready| Instant::now() < ready))
        }

        fn paste_from_board(&self) {
            if let Some(text) = board_text(&self.ivars().board).filter(|text| !text.is_empty()) {
                if self.ivars().smoke && text.to_string().contains(['\r', '\n']) {
                    return;
                }
                unsafe {
                    let _: () =
                        msg_send![self, insertText: &*text, replacementRange: self.selection()];
                }
                self.ivars().pastes.set(self.ivars().pastes.get() + 1);
                self.update_title();
                if self.ivars().smoke {
                    report(format_args!("PASTE {}", self.text()));
                }
            }
        }

        fn text(&self) -> Retained<NSString> {
            unsafe { msg_send![self, string] }
        }

        fn set_text(&self, text: &NSString) {
            unsafe {
                let _: () = msg_send![self, setString: text];
            }
        }

        fn selection(&self) -> NSRange {
            unsafe { msg_send![self, selectedRange] }
        }

        fn select(&self, range: NSRange) {
            unsafe {
                let _: () = msg_send![self, setSelectedRange: range];
            }
        }

        fn update_title(&self) {
            let state = self.ivars();
            let title = NSString::from_str(&format!(
                "Nami Cross-App Fixture — Copies: {} / Pastes: {}",
                state.copies.get(),
                state.pastes.get(),
            ));
            unsafe {
                let window: Option<Retained<NSWindow>> = msg_send![self, window];
                if let Some(window) = window {
                    window.setTitle(&title);
                }
            }
        }
    }

    fn self_test(mtm: MainThreadMarker) {
        unsafe {
            let board: Retained<AnyObject> =
                msg_send![class!(NSPasteboard), pasteboardWithUniqueName];
            let editor = FixtureEditor::new(mtm, NSRect::ZERO, board.clone(), Mode::Copy, false);
            let paste_item: Retained<AnyObject> = msg_send![class!(NSMenuItem), new];
            let _: () = msg_send![&*paste_item, setAction: sel!(paste:)];
            editor.set_text(ns_string!("A 🧪 sample Z"));
            editor.select(NSRange::new(2, 9));
            let _: () = msg_send![&*editor, copy: null_mut::<AnyObject>()];
            assert_eq!(board_text(&board).unwrap().to_string(), "🧪 sample");
            assert_eq!(editor.ivars().copies.get(), 1);
            editor.set_text(ns_string!("[replace]"));
            editor.select(NSRange::new(1, 7));
            let enabled: bool = msg_send![&*editor, validateMenuItem: &*paste_item];
            assert!(enabled);
            let _: () = msg_send![&*editor, paste: null_mut::<AnyObject>()];
            assert_eq!(editor.text().to_string(), "[🧪 sample]");
            assert_eq!(editor.ivars().pastes.get(), 1);
            editor.select(NSRange::new(0, 0));
            let version: isize = msg_send![&*board, changeCount];
            let _: () = msg_send![&*editor, copy: null_mut::<AnyObject>()];
            let current: isize = msg_send![&*board, changeCount];
            assert_eq!(current, version);
            let _: isize = msg_send![&*board, clearContents];
            let enabled: bool = msg_send![&*editor, validateMenuItem: &*paste_item];
            assert!(!enabled);
            let _: () = msg_send![&*editor, paste: null_mut::<AnyObject>()];
            assert_eq!(editor.text().to_string(), "[🧪 sample]");
            assert_eq!(editor.ivars().pastes.get(), 1);
            for (arg, body, range) in [
                ("ax", SELECTION, NSRange::new(0, 24)),
                ("copy", SELECTION, NSRange::new(0, 24)),
                ("paste", "[]", NSRange::new(1, 0)),
                ("empty", SELECTION, NSRange::new(0, 0)),
            ] {
                let mode = Mode::parse(arg).unwrap();
                let editor = FixtureEditor::new(mtm, NSRect::ZERO, board.clone(), mode, false);
                let (text, selection) = mode.initial();
                editor.set_text(&NSString::from_str(text));
                editor.select(selection);
                assert_eq!(editor.text().to_string(), body);
                assert_eq!(editor.selection(), range);
                for (selector, name) in [
                    (
                        sel!(accessibilitySelectedText),
                        ns_string!("AXSelectedText"),
                    ),
                    (
                        sel!(accessibilitySelectedTextRange),
                        ns_string!("AXSelectedTextRange"),
                    ),
                ] {
                    let allowed: bool =
                        msg_send![&*editor, isAccessibilitySelectorAllowed: selector];
                    let value: Option<Retained<AnyObject>> =
                        msg_send![&*editor, accessibilityAttributeValue: name];
                    assert_eq!(allowed, arg != "copy");
                    assert_eq!(value.is_some(), arg != "copy");
                }
                if arg == "ax" {
                    let text: Retained<NSString> = msg_send![&*editor, accessibilityAttributeValue: ns_string!("AXSelectedText")];
                    assert_eq!(text.to_string(), SELECTION);
                } else if arg == "copy" {
                    let _: () = msg_send![&*editor, copy: null_mut::<AnyObject>()];
                    assert_eq!(board_text(&board).unwrap().to_string(), SELECTION);
                } else if arg == "paste" {
                    let _: () = msg_send![&*editor, paste: null_mut::<AnyObject>()];
                    assert_eq!(editor.text().to_string(), format!("[{SELECTION}]"));
                }
            }
            assert!(Mode::parse("unknown").is_none());
            let _: () = msg_send![&*board, releaseGlobally];
        }
        println!("PASS: isolated copy, replacement paste, empty copy/paste, and AX selector gate");
    }

    unsafe fn submenu(main: &AnyObject, title: &NSString) -> Retained<AnyObject> {
        let item: Retained<AnyObject> = msg_send![class!(NSMenuItem), new];
        let _: () = msg_send![main, addItem: &*item];
        let menu: Retained<AnyObject> = msg_send![class!(NSMenu), new];
        let _: () = msg_send![&*menu, setTitle: title];
        let _: () = msg_send![&*item, setSubmenu: &*menu];
        menu
    }

    unsafe fn show(app: &AnyObject, mtm: MainThreadMarker, mode: Mode, smoke: bool) {
        let _: bool = msg_send![app, setActivationPolicy: 0isize]; // NSApplicationActivationPolicyRegular
        let menu: Retained<AnyObject> = msg_send![class!(NSMenu), new];
        let application = submenu(&menu, ns_string!("Fixture"));
        let _: *mut AnyObject = msg_send![&*application, addItemWithTitle: ns_string!("Quit Fixture"), action: sel!(terminate:), keyEquivalent: ns_string!("q")];
        let edit = submenu(&menu, ns_string!("Edit"));
        let _: *mut AnyObject = msg_send![&*edit, addItemWithTitle: ns_string!("Copy"), action: sel!(copy:), keyEquivalent: ns_string!("c")];
        let _: *mut AnyObject = msg_send![&*edit, addItemWithTitle: ns_string!("Paste"), action: sel!(paste:), keyEquivalent: ns_string!("v")];
        let _: () = msg_send![app, setMainMenu: &*menu];
        let _: () = msg_send![app, finishLaunching];

        let size = NSRect::new(NSPoint::ZERO, NSSize::new(660.0, 220.0));
        let window: Retained<NSWindow> = msg_send![NSWindow::alloc(mtm), initWithContentRect: size, styleMask: NSWindowStyleMask::Titled | NSWindowStyleMask::Closable, backing: NSBackingStoreType::Buffered, defer: false];
        let _: () = msg_send![&*window, setReleasedWhenClosed: false];
        let board = msg_send![class!(NSPasteboard), pasteboardWithName: ns_string!("dev.local.nami.ui-tests.runtime")];
        let editor = FixtureEditor::new(mtm, size, board, mode, smoke);
        let manual_body;
        let (body, range) = if smoke {
            mode.initial()
        } else {
            let label = if mode == Mode::Copy {
                "Copy fallback"
            } else {
                "AX"
            };
            manual_body =
                format!("Mode: {label}\n\n{SELECTION}\nNAMI baseline marker\nPaste here: []");
            let range = NSRange::new(manual_body.find(SELECTION).unwrap(), SELECTION.len());
            (manual_body.as_str(), range)
        };
        let _: () = msg_send![&*editor, setRichText: false];
        let _: () = msg_send![&*editor, setTextContainerInset: NSSize::new(16.0, 16.0)];
        editor.set_text(&NSString::from_str(body));
        let _: () = msg_send![&*window, setContentView: &*editor];
        editor.update_title();
        let _: () = msg_send![&*window, center];
        let _: () = msg_send![&*window, makeKeyAndOrderFront: null_mut::<AnyObject>()];
        let focused: bool = msg_send![&*window, makeFirstResponder: &*editor];
        assert!(focused, "fixture editor refused first responder");
        editor.select(range);
        let _: () = msg_send![app, activateIgnoringOtherApps: true];
        if smoke {
            report(format_args!("READY {}", std::process::id()));
        }
        let _: () = msg_send![app, run];
    }

    fn usage() -> ! {
        eprintln!("Usage: cross-app-fixture [--copy-fallback | --self-test | --smoke {{ax|manual-ax|copy|paste|empty}}]");
        std::process::exit(2);
    }

    pub fn run() {
        let mtm = MainThreadMarker::new().expect("AppKit requires the main thread");
        let args = std::env::args().skip(1).collect::<Vec<_>>();
        objc2::rc::autoreleasepool(|_| unsafe {
            let class = if matches!(args.as_slice(), [flag, mode] if flag == "--smoke" && mode == "manual-ax")
            {
                ColdAxApplication::class()
            } else {
                class!(NSApplication)
            };
            let app: Retained<AnyObject> = msg_send![class, sharedApplication];
            match args.as_slice() {
                [] => show(&app, mtm, Mode::Ax, false),
                [arg] if arg == "--copy-fallback" => show(&app, mtm, Mode::Copy, false),
                [arg] if arg == "--self-test" => self_test(mtm),
                [flag, mode] if flag == "--smoke" => {
                    show(
                        &app,
                        mtm,
                        Mode::parse(mode).unwrap_or_else(|| usage()),
                        true,
                    );
                }
                _ => usage(),
            }
        });
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    macos::run();
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("cross-app-fixture requires macOS AppKit");
        std::process::exit(1);
    }
}
