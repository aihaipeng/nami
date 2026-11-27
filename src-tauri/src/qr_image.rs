use std::path::{Path, PathBuf};

const MIN_QR_SIZE: u32 = 21;
const MAX_QR_SIZE: u32 = 177;
const QUIET_ZONE_MODULES: u32 = 4;

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

fn adler32(bytes: &[u8]) -> u32 {
    const MOD: u32 = 65_521;
    let mut s1 = 1u32;
    let mut s2 = 0u32;
    for &byte in bytes {
        s1 = (s1 + u32::from(byte)) % MOD;
        s2 = (s2 + s1) % MOD;
    }
    (s2 << 16) | s1
}

fn push_chunk(out: &mut Vec<u8>, chunk_type: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let crc_start = out.len();
    out.extend_from_slice(chunk_type);
    out.extend_from_slice(data);
    let checksum = crc32(&out[crc_start..]);
    out.extend_from_slice(&checksum.to_be_bytes());
}

fn zlib_stored(raw: &[u8]) -> Vec<u8> {
    const MAX_BLOCK: usize = 65_535;
    let block_count = raw.len().div_ceil(MAX_BLOCK).max(1);
    let mut out = Vec::with_capacity(raw.len() + block_count * 5 + 6);
    // Zlib header: CMF=0x78 (deflate, 32K window), FLG=0x01 (no dict, FCHECK).
    out.push(0x78);
    out.push(0x01);
    if raw.is_empty() {
        out.push(0x01);
        out.extend_from_slice(&0u16.to_le_bytes());
        out.extend_from_slice(&0xffffu16.to_le_bytes());
    } else {
        let mut offset = 0;
        while offset < raw.len() {
            let end = (offset + MAX_BLOCK).min(raw.len());
            let len = (end - offset) as u16;
            let is_final = end == raw.len();
            out.push(if is_final { 0x01 } else { 0x00 });
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(&raw[offset..end]);
            offset = end;
        }
    }
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

pub(crate) fn validate_matrix(size: u32, packed_modules: &[u8]) -> Result<(), String> {
    if !(MIN_QR_SIZE..=MAX_QR_SIZE).contains(&size) || !(size - 17).is_multiple_of(4) {
        return Err("invalid_argument".into());
    }
    let expected = (size as usize * size as usize).div_ceil(8);
    if packed_modules.len() != expected {
        return Err("invalid_argument".into());
    }
    Ok(())
}

/// Renders a validated QR bit matrix into a crisp lossless 8-bit grayscale PNG
/// with a 4-module white quiet zone and integer pixel scaling.
pub(crate) fn render_qr_png(
    size: u32,
    packed_modules: &[u8],
    target_px: u32,
) -> Result<Vec<u8>, String> {
    validate_matrix(size, packed_modules)?;
    let view_modules = size + QUIET_ZONE_MODULES * 2;
    let scale = (target_px / view_modules).clamp(4, 16);
    let dim = (view_modules * scale) as usize;
    let row_stride = dim + 1;
    let mut scanlines = vec![0xffu8; row_stride * dim];
    for y in 0..dim {
        scanlines[y * row_stride] = 0x00; // PNG filter type 0 (None)
    }

    let size_usize = size as usize;
    let quiet_usize = QUIET_ZONE_MODULES as usize;
    let scale_usize = scale as usize;
    for r in 0..size_usize {
        for c in 0..size_usize {
            let idx = r * size_usize + c;
            let dark = (packed_modules[idx >> 3] >> (7 - (idx & 7))) & 1 == 1;
            if !dark {
                continue;
            }
            let x0 = (c + quiet_usize) * scale_usize;
            let y0 = (r + quiet_usize) * scale_usize;
            for dy in 0..scale_usize {
                let row_start = (y0 + dy) * row_stride + 1 + x0;
                scanlines[row_start..row_start + scale_usize].fill(0x00);
            }
        }
    }

    let mut png = Vec::with_capacity(scanlines.len() + 128);
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&(dim as u32).to_be_bytes());
    ihdr[4..8].copy_from_slice(&(dim as u32).to_be_bytes());
    ihdr[8] = 8; // 8-bit depth
    ihdr[9] = 0; // grayscale
    ihdr[10] = 0; // deflate
    ihdr[11] = 0; // adaptive filter
    ihdr[12] = 0; // no interlace
    push_chunk(&mut png, b"IHDR", &ihdr);
    let idat = zlib_stored(&scanlines);
    push_chunk(&mut png, b"IDAT", &idat);
    push_chunk(&mut png, b"IEND", &[]);

    Ok(crate::platform::compress_png(&png).unwrap_or(png))
}

pub(crate) fn validate_save_destination(
    app_data: &Path,
    destination: &Path,
) -> Result<PathBuf, String> {
    let app_data = app_data.canonicalize().map_err(|_| "storage_unavailable")?;
    let parent = destination.parent().ok_or("qr_save_failed")?;
    let parent = parent.canonicalize().map_err(|_| "qr_save_failed")?;
    if parent.starts_with(&app_data) {
        return Err("qr_save_failed".into());
    }
    let file_name = destination.file_name().ok_or("qr_save_failed")?;
    let resolved = parent.join(file_name);
    match resolved.symlink_metadata() {
        Ok(metadata) if metadata.is_file() => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        _ => return Err("qr_save_failed".into()),
    }
    Ok(resolved)
}

pub(crate) fn atomic_write_png(destination: &Path, png: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let parent = destination.parent().ok_or("qr_save_failed")?;
    let temp = parent.join(format!(".nami-qr-{}.tmp", uuid::Uuid::new_v4().simple()));
    let write_result = (|| -> Result<(), String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|_| "qr_save_failed")?;
        file.write_all(png).map_err(|_| "qr_save_failed")?;
        file.sync_all().map_err(|_| "qr_save_failed")?;
        std::fs::rename(&temp, destination).map_err(|_| "qr_save_failed")?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    write_result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_qr_matrix_dimensions_and_packed_length() {
        assert!(validate_matrix(21, &[0u8; (21usize * 21).div_ceil(8)]).is_ok());
        assert!(validate_matrix(177, &[0u8; (177usize * 177).div_ceil(8)]).is_ok());
        assert!(validate_matrix(20, &[0u8; 50]).is_err());
        assert!(validate_matrix(22, &[0u8; 61]).is_err());
        assert!(validate_matrix(181, &[0u8; 4096]).is_err());
        assert!(validate_matrix(21, &[0u8; 10]).is_err());
    }

    #[test]
    fn renders_valid_png_with_expected_dimensions_and_signature() {
        let mut packed = vec![0u8; (21usize * 21).div_ceil(8)];
        packed[0] = 0b1010_0000;
        let png = render_qr_png(21, &packed, 768).unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        assert_eq!(&png[12..16], b"IHDR");
        let width = u32::from_be_bytes(png[16..20].try_into().unwrap());
        let height = u32::from_be_bytes(png[20..24].try_into().unwrap());
        assert_eq!(width, height);
        assert_eq!(width, (21 + 8) * 16);
    }

    #[test]
    fn rejects_saving_inside_app_data_and_writes_atomically_outside() {
        let root = std::env::temp_dir().join(format!("nami-qr-save-{}", uuid::Uuid::new_v4()));
        let app_data = root.join("app-data");
        let export_dir = root.join("export");
        std::fs::create_dir_all(&app_data).unwrap();
        std::fs::create_dir_all(&export_dir).unwrap();

        assert!(validate_save_destination(&app_data, &app_data.join("qr.png")).is_err());
        let dest = validate_save_destination(&app_data, &export_dir.join("qr.png")).unwrap();
        atomic_write_png(&dest, b"\x89PNG\r\n\x1a\n").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"\x89PNG\r\n\x1a\n");

        let _ = std::fs::remove_dir_all(&root);
    }
}
