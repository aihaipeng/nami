#[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
pub fn image_dimensions(bytes: &[u8]) -> Result<(u32, u32), String> {
    use objc2_core_foundation::{CFData, CFDictionary, CFNumber, CFRetained, CFString, CFType};
    use objc2_image_io::{
        kCGImagePropertyOrientation, kCGImagePropertyPixelHeight, kCGImagePropertyPixelWidth,
        CGImageSource, CGImageSourceStatus,
    };
    if bytes.is_empty() || bytes.len() > nami::clipboard::MAX_IMAGE_BYTES {
        return Err("clipboard_image_invalid".into());
    }
    let data = CFData::from_bytes(bytes);
    let source =
        unsafe { CGImageSource::with_data(&data, None) }.ok_or("clipboard_image_invalid")?;
    if unsafe { source.status() } != CGImageSourceStatus::StatusComplete {
        return Err("clipboard_image_invalid".into());
    }
    let properties =
        unsafe { source.properties_at_index(0, None) }.ok_or("clipboard_image_invalid")?;
    let properties: CFRetained<CFDictionary<CFString, CFType>> =
        unsafe { CFRetained::cast_unchecked(properties) };
    let number = |key: &CFString| {
        properties
            .get(key)
            .and_then(|value| value.downcast_ref::<CFNumber>().and_then(CFNumber::as_i64))
    };
    let width = number(unsafe { kCGImagePropertyPixelWidth }).ok_or("clipboard_image_invalid")?;
    let height = number(unsafe { kCGImagePropertyPixelHeight }).ok_or("clipboard_image_invalid")?;
    let orientation = number(unsafe { kCGImagePropertyOrientation }).unwrap_or(1);
    oriented_dimensions(width, height, orientation)
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
pub fn image_dimensions(_: &[u8]) -> Result<(u32, u32), String> {
    Err("platform_unavailable".into())
}

#[cfg(any(all(target_os = "macos", not(feature = "portable-ui")), test))]
fn oriented_dimensions(width: i64, height: i64, orientation: i64) -> Result<(u32, u32), String> {
    if width <= 0 || height <= 0 || !(1..=8).contains(&orientation) {
        return Err("clipboard_image_invalid".into());
    }
    let width = u32::try_from(width).map_err(|_| "clipboard_image_invalid")?;
    let height = u32::try_from(height).map_err(|_| "clipboard_image_invalid")?;
    Ok(if orientation >= 5 {
        (height, width)
    } else {
        (width, height)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dimensions_follow_exif_orientation_and_reject_invalid_metadata() {
        for orientation in 1..=8 {
            assert_eq!(
                oriented_dimensions(640, 160, orientation).unwrap(),
                if orientation >= 5 {
                    (160, 640)
                } else {
                    (640, 160)
                }
            );
        }
        for (width, height, orientation) in
            [(0, 160, 1), (640, -1, 1), (640, 160, 0), (i64::MAX, 160, 1)]
        {
            assert!(oriented_dimensions(width, height, orientation).is_err());
        }
    }
    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn imageio_reads_original_dimensions_without_rendering_and_rejects_bad_data() {
        let png = include_bytes!("../../../tests/fixtures/clipboard-wide.png");
        assert_eq!(
            image_dimensions(png).unwrap(),
            (
                u32::from_be_bytes(png[16..20].try_into().unwrap()),
                u32::from_be_bytes(png[20..24].try_into().unwrap())
            )
        );
        assert!(image_dimensions(b"invalid image").is_err());
        assert!(image_dimensions(&[]).is_err());
    }
}
