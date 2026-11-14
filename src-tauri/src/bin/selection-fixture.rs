#[cfg(target_os = "macos")]
mod fixture {
    use objc2::{
        class, msg_send,
        rc::Retained,
        runtime::{AnyObject, Bool},
        DefinedClass, MainThreadOnly,
    };
    use objc2_app_kit::NSWindow;
    use objc2_foundation::{ns_string, NSPoint, NSRect, NSSize, NSString};
    use std::cell::Cell;

    const TEXT: &str = "Nami synthetic copy fallback test.";
    const MODES: [&str; 4] = ["Text", "Image", "File", "Empty"];

    fn payload(mode: usize) -> Vec<(&'static str, &'static [u8])> {
        if mode == 3 {
            return Vec::new();
        }
        let mut values = vec![("public.utf8-plain-text", TEXT.as_bytes())];
        match mode {
            1 => values.push(("public.png", &[0, 1, 2])),
            2 => values.push(("public.file-url", b"file:///tmp/nami-synthetic-file.txt")),
            _ => {}
        }
        values
    }

    // No AXSelectedText provider: Nami must exercise the copy fallback.
    objc2::define_class!(
        #[unsafe(super = NSWindow)]
        #[thread_kind = objc2::MainThreadOnly]
        #[ivars = Cell<usize>]
        struct FixtureWindow;

        impl FixtureWindow {
            #[unsafe(method(performKeyEquivalent:))]
            fn perform_key_equivalent(&self, event: &AnyObject) -> Bool {
                unsafe {
                    let flags: usize = msg_send![event, modifierFlags];
                    let characters: Option<Retained<NSString>> = msg_send![event, charactersIgnoringModifiers];
                    let key = characters.map(|value| value.to_string()).unwrap_or_default();
                    if flags & (1 << 20) != 0 {
                        if key == "c" {
                            let mode = self.ivars().get();
                            self.set_title(&format!("Copied {}", MODES[mode]));
                            let values = payload(mode);
                            if !values.is_empty() {
                                let item: Retained<AnyObject> = msg_send![class!(NSPasteboardItem), new];
                                for (kind, bytes) in values {
                                    let data: Retained<AnyObject> = msg_send![class!(NSData), dataWithBytes: bytes.as_ptr(), length: bytes.len()];
                                    let _: bool = msg_send![&*item, setData: &*data, forType: &*NSString::from_str(kind)];
                                }
                                let board: *mut AnyObject = msg_send![class!(NSPasteboard), generalPasteboard];
                                let _: isize = msg_send![board, clearContents];
                                let items: Retained<AnyObject> = msg_send![class!(NSArray), arrayWithObject: &*item];
                                let _: bool = msg_send![board, writeObjects: &*items];
                            }
                            return Bool::YES;
                        }
                        if key == "t" {
                            self.set_title("T reached source");
                            return Bool::YES;
                        }
                    }
                    msg_send![super(self), performKeyEquivalent: event]
                }
            }

            #[unsafe(method(choose:))]
            fn choose(&self, sender: &AnyObject) {
                unsafe {
                    let mode: isize = msg_send![sender, tag];
                    self.ivars().set(mode as usize);
                    self.set_title(MODES[mode as usize]);
                    let content: *mut AnyObject = msg_send![self, contentView];
                    let _: bool = msg_send![self, makeFirstResponder: content];
                }
            }
        }
    );

    impl FixtureWindow {
        fn set_title(&self, suffix: &str) {
            unsafe {
                let title = NSString::from_str(&format!("Nami Synthetic Selection — {suffix}"));
                let _: () = msg_send![self, setTitle: &*title];
            }
        }
    }

    fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
    }

    pub fn run() {
        if std::env::args().any(|arg| arg == "--self-check") {
            assert_eq!(payload(0), [("public.utf8-plain-text", TEXT.as_bytes())]);
            assert_eq!(payload(1)[1], ("public.png", &[0, 1, 2][..]));
            assert_eq!(
                payload(2)[1],
                (
                    "public.file-url",
                    &b"file:///tmp/nami-synthetic-file.txt"[..]
                )
            );
            assert!(payload(3).is_empty());
            println!("PASS selection fixture Text/Image/File/Empty payloads; clipboard untouched");
            return;
        }
        let mtm = objc2::MainThreadMarker::new().expect("fixture must run on the main thread");
        unsafe {
            let app: *mut AnyObject = msg_send![class!(NSApplication), sharedApplication];
            let _: bool = msg_send![app, setActivationPolicy: 0isize];
            let window = FixtureWindow::alloc(mtm).set_ivars(Cell::new(0));
            let window: Retained<FixtureWindow> = msg_send![super(window),
                initWithContentRect: rect(120.0, 200.0, 540.0, 220.0),
                styleMask: 3usize, backing: 2usize, defer: false];
            let _: () = msg_send![&*window, setReleasedWhenClosed: false];
            window.set_title("Text");
            let content: *mut AnyObject = msg_send![&*window, contentView];
            let label: Retained<AnyObject> = msg_send![class!(NSTextField), labelWithString: ns_string!("Synthetic selection: Nami synthetic copy fallback test.")];
            let _: () = msg_send![&*label, setFrame: rect(20.0, 145.0, 500.0, 40.0)];
            let _: () = msg_send![content, addSubview: &*label];
            for (index, mode) in MODES.iter().enumerate() {
                let button: Retained<AnyObject> = msg_send![class!(NSButton),
                    buttonWithTitle: &*NSString::from_str(mode), target: &*window,
                    action: objc2::sel!(choose:)];
                let _: () = msg_send![&*button, setTag: index as isize];
                let _: () = msg_send![&*button, setFrame: rect(20.0 + index as f64 * 125.0, 80.0, 110.0, 30.0)];
                let _: () = msg_send![content, addSubview: &*button];
            }
            let _: () = msg_send![&*window, makeKeyAndOrderFront: std::ptr::null::<AnyObject>()];
            let _: () = msg_send![app, activateIgnoringOtherApps: true];
            let _: () = msg_send![app, run];
        }
    }
}

#[cfg(target_os = "macos")]
fn main() {
    fixture::run();
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("Selection fixture requires macOS.");
    std::process::exit(1);
}
