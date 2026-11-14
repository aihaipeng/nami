//! Offline history icon renderer; not bundled with the Nami application.
#[cfg(target_os = "macos")]
mod macos {
    use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
    use std::{
        error::Error,
        path::{Path, PathBuf},
    };

    fn reference_svg(source: &str, filled: bool, svg: &str) -> String {
        let svg = svg.replace("currentColor", "black");
        if !filled {
            return svg;
        }
        let prefix = if source == "star" {
            "<path d=\"M11.525"
        } else {
            "<path d=\"M9 10.76"
        };
        svg.replace(prefix, &prefix.replacen("<path", "<path fill=\"black\"", 1))
    }

    unsafe fn render(image: &AnyObject, fit: bool, output: &Path) -> Result<(), Box<dyn Error>> {
        let allocated: *mut AnyObject = msg_send![class!(NSBitmapImageRep), alloc];
        let bitmap: *mut AnyObject = msg_send![allocated,
            initWithBitmapDataPlanes: std::ptr::null_mut::<*mut u8>(),
            pixelsWide: 48isize, pixelsHigh: 48isize, bitsPerSample: 8isize,
            samplesPerPixel: 4isize, hasAlpha: true, isPlanar: false,
            colorSpaceName: &*NSString::from_str("NSDeviceRGBColorSpace"),
            bytesPerRow: 0isize, bitsPerPixel: 0isize];
        let bitmap = Retained::from_raw(bitmap).ok_or("Cannot allocate icon bitmap")?;
        let context: Option<Retained<AnyObject>> =
            msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: &*bitmap];
        let context = context.ok_or("Cannot create bitmap graphics context")?;
        let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
        let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: &*context];
        let size: NSSize = msg_send![image, size];
        let scale = 48.0 / size.width.max(size.height).max(1.0);
        let size = if fit {
            NSSize::new(size.width * scale, size.height * scale)
        } else {
            NSSize::new(48.0, 48.0)
        };
        let rect = NSRect::new(
            NSPoint::new((48.0 - size.width) / 2.0, (48.0 - size.height) / 2.0),
            size,
        );
        let _: () = msg_send![image, drawInRect: rect];
        let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
        let properties: *mut AnyObject = msg_send![class!(NSDictionary), dictionary];
        let png: Option<Retained<AnyObject>> =
            msg_send![&*bitmap, representationUsingType: 4usize, properties: properties];
        let png = png.ok_or("Cannot encode icon PNG")?;
        let bytes: *const u8 = msg_send![&*png, bytes];
        let length: usize = msg_send![&*png, length];
        std::fs::write(output, std::slice::from_raw_parts(bytes, length))?;
        Ok(())
    }

    pub fn run() -> Result<(), Box<dyn Error>> {
        let assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/assets/history");
        let mut output = assets.clone();
        let mut symbols = false;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--symbols" => symbols = true,
                "--reference" => symbols = false,
                "--output" => {
                    output = PathBuf::from(args.next().ok_or("--output requires a directory")?)
                }
                _ => {
                    return Err(format!(
                        "Unknown argument: {arg}; use --reference|--symbols [--output DIR]"
                    )
                    .into())
                }
            }
        }
        std::fs::create_dir_all(&output)?;
        unsafe {
            // Keep the AppKit framework linked for dynamically resolved classes.
            let _ = std::mem::size_of::<objc2_app_kit::NSView>();
            if symbols {
                for name in [
                    "list.bullet",
                    "star",
                    "star.fill",
                    "pin",
                    "pin.fill",
                    "arrow.clockwise",
                    "doc.on.doc",
                    "trash",
                    "magnifyingglass",
                    "xmark.circle.fill",
                ] {
                    let image: Option<Retained<AnyObject>> = msg_send![class!(NSImage), imageWithSystemSymbolName: &*NSString::from_str(name), accessibilityDescription: std::ptr::null::<NSString>()];
                    let image = image.ok_or_else(|| format!("Cannot load SF Symbol: {name}"))?;
                    let config: *mut AnyObject = msg_send![class!(NSImageSymbolConfiguration), configurationWithPointSize: 16.0f64, weight: 0.0f64];
                    let image: Retained<AnyObject> =
                        msg_send![&*image, imageWithSymbolConfiguration: config];
                    let mono: *mut AnyObject = msg_send![
                        class!(NSImageSymbolConfiguration),
                        configurationPreferringMonochrome
                    ];
                    let image: Retained<AnyObject> =
                        msg_send![&*image, imageWithSymbolConfiguration: mono];
                    render(&image, true, &output.join(format!("{name}.png")))?;
                }
            } else {
                for (name, source, filled) in [
                    ("reference-clock", "clock", false),
                    ("reference-keyboard", "keyboard", false),
                    ("reference-list", "list", false),
                    ("reference-refresh", "refresh-cw", false),
                    ("reference-copy", "copy", false),
                    ("reference-trash", "trash", false),
                    ("reference-search", "search", false),
                    ("reference-clear", "clear", false),
                    ("reference-star", "star", false),
                    ("reference-star-filled", "star", true),
                    ("reference-pin", "pin", false),
                    ("reference-pin-filled", "pin", true),
                ] {
                    let svg = std::fs::read_to_string(
                        assets.join("reference").join(format!("{source}.svg")),
                    )?;
                    let svg = reference_svg(source, filled, &svg);
                    let data: *mut AnyObject =
                        msg_send![class!(NSData), dataWithBytes: svg.as_ptr(), length: svg.len()];
                    let allocated: *mut AnyObject = msg_send![class!(NSImage), alloc];
                    let image: *mut AnyObject = msg_send![allocated, initWithData: data];
                    let image = Retained::from_raw(image)
                        .ok_or_else(|| format!("Cannot load reference SVG: {source}"))?;
                    render(&image, false, &output.join(format!("{name}.png")))?;
                }
            }
        }
        println!("History icons exported to {}", output.display());
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn reference_color_and_fill_preserve_paths() {
            let source = "<svg stroke=\"currentColor\"><path d=\"M11.525 2\"/></svg>";
            assert_eq!(
                reference_svg("star", false, source),
                source.replace("currentColor", "black")
            );
            assert!(
                reference_svg("star", true, source).contains("<path fill=\"black\" d=\"M11.525")
            );
            assert!(reference_svg("pin", true, "<path d=\"M9 10.76 0\"/>")
                .contains("<path fill=\"black\""));
        }
    }
}

fn main() {
    #[cfg(target_os = "macos")]
    if let Err(error) = macos::run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
    #[cfg(not(target_os = "macos"))]
    {
        eprintln!("History icon export requires macOS.");
        std::process::exit(1);
    }
}
