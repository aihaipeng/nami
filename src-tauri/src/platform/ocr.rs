// Cargo shares macOS framework dependencies across hosts; portable UI never calls Vision.
#[cfg(all(target_os = "macos", feature = "portable-ui"))]
use objc2_vision as _;

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
const MAX_PIXELS: u64 = 20_000_000;
#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
const MAX_DECODED_BYTES: u64 = 80 * 1024 * 1024;

#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
// Vision performs synchronously; callers must use their bounded background worker.
pub fn recognize_image_text(bytes: &[u8]) -> Result<String, String> {
    use objc2::{rc::autoreleasepool, AnyThread};
    use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_foundation::{NSArray, NSDictionary};
    use objc2_image_io::{
        kCGImagePropertyDepth, kCGImagePropertyOrientation, kCGImagePropertyPixelHeight,
        kCGImagePropertyPixelWidth, CGImagePropertyOrientation, CGImageSource, CGImageSourceStatus,
    };
    use objc2_vision::{
        VNImageRequestHandler, VNRecognizeTextRequest, VNRequest, VNRequestTextRecognitionLevel,
    };

    if bytes.len() > MAX_ENCODED_BYTES {
        return Err("ocr_image_limit".into());
    }
    autoreleasepool(|_| {
        let data = CFData::from_bytes(bytes);
        // ImageIO metadata inspection does not decode the pixel buffer.
        let source = unsafe { CGImageSource::with_data(&data, None) }.ok_or("ocr_invalid_image")?;
        if unsafe { source.status() } != CGImageSourceStatus::StatusComplete
            || unsafe { source.count() } == 0
        {
            return Err("ocr_invalid_image".into());
        }
        let properties =
            unsafe { source.properties_at_index(0, None) }.ok_or("ocr_invalid_image")?;
        // ImageIO property dictionaries have CFString keys and CFType values.
        let properties: CFRetained<CFDictionary<CFString, CFType>> =
            unsafe { CFRetained::cast_unchecked(properties) };
        let number = |key: &CFString| {
            properties
                .get(key)
                .and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64))
        };
        let width = number(unsafe { kCGImagePropertyPixelWidth }).ok_or("ocr_invalid_image")?;
        let height = number(unsafe { kCGImagePropertyPixelHeight }).ok_or("ocr_invalid_image")?;
        let depth = number(unsafe { kCGImagePropertyDepth }).unwrap_or(8);
        validate_dimensions(width, height, depth)?;
        let orientation = number(unsafe { kCGImagePropertyOrientation }).unwrap_or(1);
        let orientation = match orientation {
            1 => CGImagePropertyOrientation::Up,
            2 => CGImagePropertyOrientation::UpMirrored,
            3 => CGImagePropertyOrientation::Down,
            4 => CGImagePropertyOrientation::DownMirrored,
            5 => CGImagePropertyOrientation::LeftMirrored,
            6 => CGImagePropertyOrientation::Right,
            7 => CGImagePropertyOrientation::RightMirrored,
            8 => CGImagePropertyOrientation::Left,
            _ => return Err("ocr_invalid_image".into()),
        };
        let handler = unsafe {
            VNImageRequestHandler::initWithData_orientation_options(
                VNImageRequestHandler::alloc(),
                data.as_ref(),
                orientation,
                &NSDictionary::new(),
            )
        };
        let request = VNRecognizeTextRequest::new();
        request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
        request.setAutomaticallyDetectsLanguage(true);
        request.setMinimumTextHeight(0.0);
        let requests = NSArray::from_slice(&[request.as_ref() as &VNRequest]);
        handler
            .performRequests_error(&requests)
            .map_err(|_| "ocr_recognition_failed")?;
        let observations = request.results().ok_or("ocr_recognition_failed")?;
        Ok(observations
            .iter()
            .filter_map(|observation| observation.topCandidates(1).firstObject())
            .map(|candidate| candidate.string().to_string())
            .filter(|line| !line.trim().is_empty())
            .collect::<Vec<_>>()
            .join("\n"))
    })
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub fn recognize_image_text(_bytes: &[u8]) -> Result<String, String> {
    Err("platform_unavailable".into())
}

#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
fn validate_dimensions(width: i64, height: i64, depth: i64) -> Result<(), String> {
    if width <= 0 || height <= 0 || depth <= 0 {
        return Err("ocr_invalid_image".into());
    }
    let pixels = (width as u64).checked_mul(height as u64);
    let bytes = pixels.and_then(|pixels| pixels.checked_mul((depth as u64).div_ceil(8) * 4));
    if pixels.is_none_or(|pixels| pixels > MAX_PIXELS)
        || bytes.is_none_or(|bytes| bytes > MAX_DECODED_BYTES)
    {
        return Err("ocr_image_limit".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_reject_invalid_dimensions_overflow_and_large_high_depth_images() {
        assert_eq!(
            validate_dimensions(0, 12, 8),
            Err("ocr_invalid_image".into())
        );
        assert_eq!(
            validate_dimensions(12, -1, 8),
            Err("ocr_invalid_image".into())
        );
        assert_eq!(
            validate_dimensions(12, 12, 0),
            Err("ocr_invalid_image".into())
        );
        for size in [(5000, 5000, 8), (4000, 4000, 16), (i64::MAX, i64::MAX, 8)] {
            assert_eq!(
                validate_dimensions(size.0, size.1, size.2),
                Err("ocr_image_limit".into())
            );
        }
        assert!(validate_dimensions(4000, 4000, 8).is_ok());
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn vision_recognizes_synthetic_text_and_distinguishes_blank_corrupt_and_oversized_images() {
        let blank = bitmap(false);
        assert_eq!(recognize_image_text(&blank), Ok(String::new()));
        let text = recognize_image_text(&bitmap(true)).expect("synthetic text OCR succeeds");
        assert!(text.contains("HELLO") && text.contains("NAMI"), "{text:?}");
        assert_eq!(
            recognize_image_text(b"invalid image"),
            Err("ocr_invalid_image".into())
        );
        assert_eq!(recognize_image_text(&[]), Err("ocr_invalid_image".into()));
        assert_eq!(
            recognize_image_text(&vec![0; MAX_ENCODED_BYTES + 1]),
            Err("ocr_image_limit".into())
        );
        let mut oversized = blank;
        oversized[18..22].copy_from_slice(&5000i32.to_le_bytes());
        oversized[22..26].copy_from_slice(&5000i32.to_le_bytes());
        assert_eq!(
            recognize_image_text(&oversized),
            Err("ocr_image_limit".into())
        );
    }

    #[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
    #[test]
    fn unsupported_host_never_attempts_recognition() {
        assert_eq!(
            recognize_image_text(b"image"),
            Err("platform_unavailable".into())
        );
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    fn bitmap(text: bool) -> Vec<u8> {
        let (width, height) = (640usize, 160usize);
        let pixel_bytes = width * height * 3;
        let mut bytes = vec![255; 54 + pixel_bytes];
        bytes[..54].fill(0);
        bytes[..2].copy_from_slice(b"BM");
        bytes[2..6].copy_from_slice(&((54 + pixel_bytes) as u32).to_le_bytes());
        bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
        bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
        bytes[18..22].copy_from_slice(&(width as i32).to_le_bytes());
        bytes[22..26].copy_from_slice(&(height as i32).to_le_bytes());
        bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
        bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
        bytes[34..38].copy_from_slice(&(pixel_bytes as u32).to_le_bytes());
        if text {
            use objc2::{class, msg_send, rc::Retained, runtime::AnyObject};
            use objc2_foundation::{ns_string, NSData, NSPoint, NSString};
            unsafe {
                let data = NSData::with_bytes(&bytes);
                let bitmap: Retained<AnyObject> =
                    msg_send![class!(NSBitmapImageRep), imageRepWithData: &*data];
                let context: Retained<AnyObject> = msg_send![class!(NSGraphicsContext), graphicsContextWithBitmapImageRep: &*bitmap];
                let _: () = msg_send![class!(NSGraphicsContext), saveGraphicsState];
                let _: () = msg_send![class!(NSGraphicsContext), setCurrentContext: &*context];
                let attributes: Retained<AnyObject> =
                    msg_send![class!(NSMutableDictionary), dictionary];
                let font: Retained<AnyObject> =
                    msg_send![class!(NSFont), boldSystemFontOfSize: 48f64];
                let color: Retained<AnyObject> = msg_send![class!(NSColor), blackColor];
                let _: () =
                    msg_send![&*attributes, setObject: &*font, forKey: ns_string!("NSFont")];
                let _: () =
                    msg_send![&*attributes, setObject: &*color, forKey: ns_string!("NSColor")];
                let label = NSString::from_str("HELLO NAMI");
                let _: () = msg_send![&*label, drawAtPoint: NSPoint::new(48.0, 48.0), withAttributes: &*attributes];
                let _: () = msg_send![class!(NSGraphicsContext), restoreGraphicsState];
                let properties: Retained<AnyObject> = msg_send![class!(NSDictionary), dictionary];
                let png: Retained<NSData> =
                    msg_send![&*bitmap, representationUsingType: 4usize, properties: &*properties];
                return png.to_vec();
            }
        }
        bytes
    }
}
