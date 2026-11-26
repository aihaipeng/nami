use crate::{clipboard_runtime, platform, AppState};
use nami::clipboard::{ClipboardPage, MAX_IMAGE_BYTES};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};
use tauri::{AppHandle, Manager, UriSchemeContext, UriSchemeResponder};

pub(crate) const MEDIA_SCHEME: &str = "nami-media";
const MIN_PREVIEW_PIXELS: u32 = 128;
const MIN_PREVIEW_HEIGHT: u32 = 32;
const MAX_PREVIEW_PIXELS: u32 = 2048;
const DEFAULT_PREVIEW_PIXELS: u32 = 1024;
const MAX_THUMB_CACHE_FILES: usize = 256;
const MAX_THUMB_CACHE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_ICON_BYTES: usize = 4 * 1024 * 1024;
const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";

pub(crate) fn media_url(path: &str) -> String {
    #[cfg(target_os = "windows")]
    {
        format!("http://{MEDIA_SCHEME}.localhost{path}")
    }
    #[cfg(not(target_os = "windows"))]
    {
        format!("{MEDIA_SCHEME}://localhost{path}")
    }
}

pub(crate) fn rewrite_page_source_icons(page: &mut ClipboardPage) {
    for (id, value) in &mut page.sources {
        let Some(object) = value.as_object_mut() else {
            continue;
        };
        let has_inline_png = object
            .get("iconDataUrl")
            .and_then(|value| value.as_str())
            .is_some_and(|url| url.starts_with("data:image/png;base64,"));
        if has_inline_png {
            object.insert(
                "iconDataUrl".into(),
                serde_json::Value::String(media_url(&format!("/icon/{id}"))),
            );
        }
    }
}

fn thumb_cache_dir(app: &AppHandle) -> Option<PathBuf> {
    app.path()
        .app_cache_dir()
        .ok()
        .map(|root| root.join("clipboard-thumbs"))
}

fn ensure_thumb_dir(directory: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(directory)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700));
    }
    Ok(())
}

fn extract_stored_image_hash(relative: &str) -> Option<&str> {
    let filename = relative.rsplit('/').next()?;
    let (hash, _) = filename.rsplit_once('.')?;
    if hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Some(hash)
    } else {
        None
    }
}

fn fit_box_max_pixels(orig_w: u32, orig_h: u32, max_w: u32, max_h: Option<u32>) -> u32 {
    let Some(max_h) = max_h else {
        return max_w;
    };
    if orig_w == 0 || orig_h == 0 {
        return max_w;
    }
    let scale = (f64::from(max_w) / f64::from(orig_w))
        .min(f64::from(max_h) / f64::from(orig_h))
        .min(1.0);
    let target_w = (f64::from(orig_w) * scale).ceil().max(1.0) as u32;
    let target_h = (f64::from(orig_h) * scale).ceil().max(1.0) as u32;
    target_w.max(target_h).clamp(1, max_w)
}

fn cache_dim_suffix(max_w: u32, max_h: Option<u32>) -> String {
    match max_h {
        Some(h) => format!("{max_w}x{h}"),
        None => max_w.to_string(),
    }
}

fn stored_image_cache_key(relative: &str, max_w: u32, max_h: Option<u32>) -> Option<String> {
    let hash = extract_stored_image_hash(relative)?;
    let dims = cache_dim_suffix(max_w, max_h);
    Some(format!("img_{hash}_{dims}.png"))
}

fn file_image_cache_key(
    path: &Path,
    metadata: &std::fs::Metadata,
    max_w: u32,
    max_h: Option<u32>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"file-thumb\0");
    hasher.update(path.to_string_lossy().as_bytes());
    hasher.update([0]);
    hasher.update(metadata.len().to_be_bytes());
    if let Ok(modified) = metadata.modified() {
        if let Ok(duration) = modified.duration_since(SystemTime::UNIX_EPOCH) {
            hasher.update(duration.as_secs().to_be_bytes());
            hasher.update(duration.subsec_nanos().to_be_bytes());
        }
    }
    let hash = format!("{:x}", hasher.finalize());
    let dims = cache_dim_suffix(max_w, max_h);
    format!("file_{hash}_{dims}.png")
}

fn read_cached_png(directory: &Path, key: &str) -> Option<Vec<u8>> {
    let path = directory.join(key);
    let metadata = std::fs::symlink_metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(&path).ok()?;
    bytes.starts_with(PNG_SIGNATURE).then_some(bytes)
}

fn write_cached_png(directory: &Path, key: &str, bytes: &[u8]) {
    if !bytes.starts_with(PNG_SIGNATURE) || bytes.len() > MAX_IMAGE_BYTES {
        return;
    }
    if ensure_thumb_dir(directory).is_err() {
        return;
    }
    let target = directory.join(key);
    let temp = directory.join(format!(".{key}.{}.tmp", uuid::Uuid::new_v4().simple()));
    if std::fs::write(&temp, bytes).is_ok() {
        if std::fs::rename(&temp, &target).is_err() {
            let _ = std::fs::remove_file(&temp);
        } else {
            prune_thumb_cache(directory, MAX_THUMB_CACHE_FILES, MAX_THUMB_CACHE_BYTES);
        }
    } else {
        let _ = std::fs::remove_file(&temp);
    }
}

fn prune_thumb_cache(directory: &Path, max_files: usize, max_bytes: u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    let mut files = Vec::new();
    let mut total_bytes = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') && name.ends_with(".tmp") {
            let stale = metadata
                .modified()
                .ok()
                .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                .is_some_and(|age| age.as_secs() >= 60);
            if stale {
                let _ = std::fs::remove_file(path);
            }
            continue;
        }
        let len = metadata.len();
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        total_bytes = total_bytes.saturating_add(len);
        files.push((path, len, modified));
    }
    if files.len() <= max_files && total_bytes <= max_bytes {
        return;
    }
    files.sort_by_key(|(_, _, modified)| *modified);
    let mut remaining_files = files.len();
    for (path, len, _) in files {
        if remaining_files <= max_files && total_bytes <= max_bytes {
            break;
        }
        if std::fs::remove_file(path).is_ok() {
            remaining_files = remaining_files.saturating_sub(1);
            total_bytes = total_bytes.saturating_sub(len);
        }
    }
}

pub(crate) fn remove_thumbs_for_image_paths(app: &AppHandle, relative_paths: &[String]) {
    if relative_paths.is_empty() {
        return;
    }
    let Some(directory) = thumb_cache_dir(app) else {
        return;
    };
    remove_thumbs_for_image_paths_in(&directory, relative_paths);
}

fn remove_thumbs_for_image_paths_in(directory: &Path, relative_paths: &[String]) {
    let prefixes: Vec<String> = relative_paths
        .iter()
        .filter_map(|path| extract_stored_image_hash(path))
        .map(|hash| format!("img_{hash}_"))
        .collect();
    if prefixes.is_empty() {
        return;
    }
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if prefixes.iter().any(|prefix| name.starts_with(prefix)) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub(crate) fn remove_file_thumbs(app: &AppHandle) {
    let Some(directory) = thumb_cache_dir(app) else {
        return;
    };
    remove_file_thumbs_in(&directory);
}

fn remove_file_thumbs_in(directory: &Path) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name.to_string_lossy().starts_with("file_") {
            let _ = std::fs::remove_file(entry.path());
        }
    }
}

pub(crate) fn prune_stale_caches(
    app: &AppHandle,
    referenced_images: &std::collections::HashSet<String>,
    drag_max_age: std::time::Duration,
) -> (usize, u64) {
    let Ok(cache_root) = app.path().app_cache_dir() else {
        return (0, 0);
    };
    prune_stale_caches_in(
        &cache_root,
        referenced_images,
        std::time::Duration::from_secs(60),
        drag_max_age,
    )
}

fn prune_stale_caches_in(
    cache_root: &Path,
    referenced_images: &std::collections::HashSet<String>,
    min_tmp_age: std::time::Duration,
    drag_max_age: std::time::Duration,
) -> (usize, u64) {
    let referenced_hashes: std::collections::HashSet<&str> = referenced_images
        .iter()
        .filter_map(|rel| extract_stored_image_hash(rel))
        .collect();
    let (thumb_removed, thumb_bytes) = prune_unreferenced_thumbs(
        &cache_root.join("clipboard-thumbs"),
        &referenced_hashes,
        min_tmp_age,
    );
    let (drag_removed, drag_bytes) =
        prune_expired_files(&cache_root.join("drag-exports"), drag_max_age);
    (
        thumb_removed + drag_removed,
        thumb_bytes.saturating_add(drag_bytes),
    )
}

fn prune_unreferenced_thumbs(
    directory: &Path,
    referenced_hashes: &std::collections::HashSet<&str>,
    min_tmp_age: std::time::Duration,
) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return (0, 0);
    };
    let now = SystemTime::now();
    let mut removed = 0usize;
    let mut bytes = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let should_remove = if name.starts_with('.') && name.ends_with(".tmp") {
            min_tmp_age.is_zero()
                || metadata
                    .modified()
                    .ok()
                    .and_then(|modified| now.duration_since(modified).ok())
                    .is_some_and(|age| age >= min_tmp_age)
        } else if let Some(rest) = name.strip_prefix("img_") {
            rest.split_once('_')
                .map(|(hash, _)| !referenced_hashes.contains(hash))
                .unwrap_or(false)
        } else {
            false
        };
        if !should_remove {
            continue;
        }
        let len = metadata.len();
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
            bytes = bytes.saturating_add(len);
        }
    }
    (removed, bytes)
}

fn prune_expired_files(directory: &Path, max_age: std::time::Duration) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(directory) else {
        return (0, 0);
    };
    let now = SystemTime::now();
    let mut removed = 0usize;
    let mut bytes = 0u64;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(metadata) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let expired = max_age.is_zero()
            || metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age >= max_age);
        if !expired {
            continue;
        }
        let len = metadata.len();
        if std::fs::remove_file(&path).is_ok() {
            removed += 1;
            bytes = bytes.saturating_add(len);
        }
    }
    (removed, bytes)
}

enum PreviewSource {
    StoredImage { relative: String },
    LocalFile { path: PathBuf },
}

pub(crate) async fn load_image_preview(
    app: &AppHandle,
    record_id: String,
    max_pixels: u32,
    max_height: Option<u32>,
) -> Result<Vec<u8>, String> {
    if !(MIN_PREVIEW_PIXELS..=MAX_PREVIEW_PIXELS).contains(&max_pixels)
        || max_height.is_some_and(|h| !(MIN_PREVIEW_HEIGHT..=MAX_PREVIEW_PIXELS).contains(&h))
    {
        return Err("invalid_argument".into());
    }
    let root = clipboard_runtime::image_root(app)?;
    let cache_dir = thumb_cache_dir(app);
    let source = {
        let state = app.state::<AppState>();
        let _write = state.clipboard_writes.lock().await;
        let (record, representation) = clipboard_runtime::database(app, move |database| {
            let record = database.clipboard_record(&record_id)?;
            let representation = if record.kind != "text" {
                database.clipboard_file_representation(&record_id)?
            } else {
                None
            };
            Ok((record, representation))
        })
        .await?;
        match record.kind.as_str() {
            "image" => {
                let relative = representation
                    .map(|item| item.image_path)
                    .or(record.image_path)
                    .ok_or("storage_corrupt")?;
                PreviewSource::StoredImage { relative }
            }
            "files" => {
                let paths: Vec<String> = match representation {
                    Some(item) => item.paths,
                    None => serde_json::from_str(&record.text).map_err(|_| "storage_corrupt")?,
                };
                if paths.len() != 1 {
                    return Err("invalid_argument".into());
                }
                let path = PathBuf::from(&paths[0]);
                if !path.is_absolute() || paths[0].contains('\0') {
                    return Err("storage_corrupt".into());
                }
                PreviewSource::LocalFile { path }
            }
            _ => return Err("invalid_argument".into()),
        }
    };
    tauri::async_runtime::spawn_blocking(move || {
        load_preview_with_cache(&root, cache_dir.as_deref(), source, max_pixels, max_height)
    })
    .await
    .map_err(|_| "storage_worker")?
}

fn load_preview_with_cache(
    root: &Path,
    cache_dir: Option<&Path>,
    source: PreviewSource,
    max_pixels: u32,
    max_height: Option<u32>,
) -> Result<Vec<u8>, String> {
    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    {
        let target_pixels_for_path = |path: &Path| match max_height {
            Some(max_h) => match platform::image_dimensions_from_path(path) {
                Ok((w, h)) => fit_box_max_pixels(w, h, max_pixels, Some(max_h)),
                Err(_) => max_pixels,
            },
            None => max_pixels,
        };
        match source {
            PreviewSource::StoredImage { relative } => {
                let cache_key = stored_image_cache_key(&relative, max_pixels, max_height);
                if let (Some(dir), Some(key)) = (cache_dir, cache_key.as_deref()) {
                    if let Some(cached) = read_cached_png(dir, key) {
                        return Ok(cached);
                    }
                }
                let path = nami::clipboard::verified_image_path(root, &relative)?;
                let effective = target_pixels_for_path(&path);
                let preview = platform::clipboard_image_preview_from_path(&path, effective)
                    .map_err(str::to_owned)?;
                if let (Some(dir), Some(key)) = (cache_dir, cache_key.as_deref()) {
                    write_cached_png(dir, key, &preview);
                }
                Ok(preview)
            }
            PreviewSource::LocalFile { path } => {
                const MAX_LOCAL_IMAGE_PIXELS: u64 = 64 * 1024 * 1024;
                let metadata = std::fs::metadata(&path).map_err(|_| "clipboard_file_missing")?;
                if !metadata.is_file() {
                    return Err("invalid_argument".into());
                }
                if metadata.len() > MAX_IMAGE_BYTES as u64 {
                    return Err("clipboard_image_limit".into());
                }
                let cache_key = file_image_cache_key(&path, &metadata, max_pixels, max_height);
                if let Some(dir) = cache_dir {
                    if let Some(cached) = read_cached_png(dir, &cache_key) {
                        return Ok(cached);
                    }
                }
                let (w, h) = platform::image_dimensions_from_path(&path)?;
                if u64::from(w) * u64::from(h) > MAX_LOCAL_IMAGE_PIXELS {
                    return Err("clipboard_image_limit".into());
                }
                let effective = match max_height {
                    Some(max_h) => fit_box_max_pixels(w, h, max_pixels, Some(max_h)),
                    None => max_pixels,
                };
                let preview = platform::clipboard_image_preview_from_path(&path, effective)
                    .map_err(str::to_owned)?;
                if let Some(dir) = cache_dir {
                    write_cached_png(dir, &cache_key, &preview);
                }
                Ok(preview)
            }
        }
    }
    #[cfg(any(not(target_os = "macos"), feature = "portable-ui"))]
    {
        load_preview_with_renderer(
            root,
            cache_dir,
            source,
            max_pixels,
            max_height,
            platform::clipboard_image_preview,
        )
    }
}

#[cfg(any(not(target_os = "macos"), feature = "portable-ui", test))]
fn load_preview_with_renderer(
    root: &Path,
    cache_dir: Option<&Path>,
    source: PreviewSource,
    max_pixels: u32,
    max_height: Option<u32>,
    render: impl FnOnce(&[u8], u32) -> Result<Vec<u8>, &'static str>,
) -> Result<Vec<u8>, String> {
    let target_pixels = |bytes: &[u8]| match max_height {
        Some(max_h) => match platform::image_dimensions(bytes) {
            Ok((w, h)) => fit_box_max_pixels(w, h, max_pixels, Some(max_h)),
            Err(_) => max_pixels,
        },
        None => max_pixels,
    };
    match source {
        PreviewSource::StoredImage { relative } => {
            let cache_key = stored_image_cache_key(&relative, max_pixels, max_height);
            if let (Some(dir), Some(key)) = (cache_dir, cache_key.as_deref()) {
                if let Some(cached) = read_cached_png(dir, key) {
                    return Ok(cached);
                }
            }
            let bytes = nami::clipboard::read_image(root, &relative)?;
            let effective = target_pixels(&bytes);
            let preview = render(&bytes, effective).map_err(str::to_owned)?;
            if let (Some(dir), Some(key)) = (cache_dir, cache_key.as_deref()) {
                write_cached_png(dir, key, &preview);
            }
            Ok(preview)
        }
        PreviewSource::LocalFile { path } => {
            let metadata = std::fs::metadata(&path).map_err(|_| "clipboard_file_missing")?;
            if !metadata.is_file() {
                return Err("invalid_argument".into());
            }
            if metadata.len() > MAX_IMAGE_BYTES as u64 {
                return Err("clipboard_image_limit".into());
            }
            let cache_key = file_image_cache_key(&path, &metadata, max_pixels, max_height);
            if let Some(dir) = cache_dir {
                if let Some(cached) = read_cached_png(dir, &cache_key) {
                    return Ok(cached);
                }
            }
            let bytes = nami::clipboard::read_file_preview(&path)?;
            let effective = target_pixels(&bytes);
            let preview = render(&bytes, effective).map_err(str::to_owned)?;
            if let Some(dir) = cache_dir {
                write_cached_png(dir, &cache_key, &preview);
            }
            Ok(preview)
        }
    }
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(input.len() / 4 * 3);
    let mut buf = 0u32;
    let mut bits = 0u8;
    let mut padding = 0u8;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            padding += 1;
            if padding > 2 {
                return None;
            }
            continue;
        }
        if padding > 0 {
            return None;
        }
        let val = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        };
        buf = (buf << 6) | u32::from(val);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buf >> bits) & 0xff) as u8);
            if out.len() > MAX_ICON_BYTES {
                return None;
            }
        }
    }
    if bits >= 6 || (bits > 0 && (buf & ((1u32 << bits) - 1)) != 0) {
        return None;
    }
    Some(out)
}

fn decode_png_data_url(url: &str) -> Option<Vec<u8>> {
    let encoded = url.strip_prefix("data:image/png;base64,")?;
    let bytes = decode_base64(encoded)?;
    bytes.starts_with(PNG_SIGNATURE).then_some(bytes)
}

pub(crate) async fn load_source_icon(
    app: &AppHandle,
    source_app_id: i64,
) -> Result<Vec<u8>, String> {
    if source_app_id <= 0 {
        return Err("invalid_argument".into());
    }
    let payload = clipboard_runtime::database(app, move |database| {
        database.source_app_payload(source_app_id)
    })
    .await?
    .ok_or("not_found")?;
    let value: serde_json::Value = serde_json::from_str(&payload).map_err(|_| "storage_corrupt")?;
    let url = value
        .get("iconDataUrl")
        .and_then(|value| value.as_str())
        .ok_or("not_found")?;
    decode_png_data_url(url).ok_or_else(|| "not_found".into())
}

#[derive(Debug, PartialEq, Eq)]
enum MediaRoute {
    Image {
        record_id: String,
        max_pixels: u32,
        max_height: Option<u32>,
    },
    Icon {
        source_app_id: i64,
    },
}

fn percent_decode(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok()?;
            let value = u8::from_str_radix(hex, 16).ok()?;
            out.push(value);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn parse_media_route(uri: &tauri::http::Uri) -> Result<MediaRoute, u16> {
    let path = uri.path();
    if let Some(raw_id) = path.strip_prefix("/image/") {
        if raw_id.is_empty() || raw_id.contains('/') {
            return Err(400);
        }
        let record_id = percent_decode(raw_id).ok_or(400u16)?;
        if record_id.is_empty()
            || record_id.len() > 256
            || record_id.contains('/')
            || record_id.contains('\\')
            || record_id.contains('\0')
            || record_id == "."
            || record_id == ".."
        {
            return Err(400);
        }
        let mut max_pixels = DEFAULT_PREVIEW_PIXELS;
        let mut max_height = None;
        if let Some(query) = uri.query() {
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                if let Some((key, value)) = pair.split_once('=') {
                    if key == "w" {
                        max_pixels = value.parse::<u32>().map_err(|_| 400u16)?;
                    } else if key == "h" {
                        max_height = Some(value.parse::<u32>().map_err(|_| 400u16)?);
                    }
                }
            }
        }
        if !(MIN_PREVIEW_PIXELS..=MAX_PREVIEW_PIXELS).contains(&max_pixels)
            || max_height.is_some_and(|h| !(MIN_PREVIEW_HEIGHT..=MAX_PREVIEW_PIXELS).contains(&h))
        {
            return Err(400);
        }
        return Ok(MediaRoute::Image {
            record_id,
            max_pixels,
            max_height,
        });
    }
    if let Some(raw_id) = path.strip_prefix("/icon/") {
        if raw_id.is_empty() || raw_id.contains('/') {
            return Err(400);
        }
        let source_app_id = raw_id.parse::<i64>().map_err(|_| 400u16)?;
        if source_app_id <= 0 {
            return Err(400);
        }
        return Ok(MediaRoute::Icon { source_app_id });
    }
    Err(404)
}

fn authorize_media_request(webview_label: &str, route: &MediaRoute) -> bool {
    match route {
        MediaRoute::Image { .. } => matches!(webview_label, "clipboard" | "translation"),
        MediaRoute::Icon { .. } => {
            matches!(webview_label, "clipboard" | "translation" | "settings")
        }
    }
}

fn empty_response(status: u16) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(status)
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff")
        .body(Vec::new())
        .unwrap()
}

fn png_response(bytes: Vec<u8>, cache_control: &str) -> tauri::http::Response<Vec<u8>> {
    tauri::http::Response::builder()
        .status(200)
        .header("Content-Type", "image/png")
        .header("Cache-Control", cache_control)
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff")
        .body(bytes)
        .unwrap()
}

pub(crate) fn handle_uri_scheme_request(
    ctx: UriSchemeContext<'_, tauri::Wry>,
    request: tauri::http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let method = request.method();
    if method == tauri::http::Method::OPTIONS {
        responder.respond(empty_response(204));
        return;
    }
    if method != tauri::http::Method::GET {
        responder.respond(empty_response(405));
        return;
    }
    let route = match parse_media_route(request.uri()) {
        Ok(route) => route,
        Err(status) => {
            responder.respond(empty_response(status));
            return;
        }
    };
    if !authorize_media_request(ctx.webview_label(), &route) {
        responder.respond(empty_response(403));
        return;
    }
    let app = ctx.app_handle().clone();
    tauri::async_runtime::spawn(async move {
        let response = match route {
            MediaRoute::Image {
                record_id,
                max_pixels,
                max_height,
            } => match load_image_preview(&app, record_id, max_pixels, max_height).await {
                Ok(bytes) => png_response(bytes, "no-store"),
                Err(code) => empty_response(match code.as_str() {
                    "invalid_argument" => 400,
                    "not_found" | "clipboard_image_missing" | "clipboard_file_missing" => 404,
                    _ => 500,
                }),
            },
            MediaRoute::Icon { source_app_id } => match load_source_icon(&app, source_app_id).await
            {
                Ok(bytes) => png_response(bytes, "private, max-age=86400, immutable"),
                Err(code) => empty_response(match code.as_str() {
                    "invalid_argument" => 400,
                    "not_found" => 404,
                    _ => 500,
                }),
            },
        };
        responder.respond(response);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn parses_media_routes_and_rejects_invalid_or_out_of_budget_requests() {
        let uri: tauri::http::Uri = "nami-media://localhost/image/rec-123?w=768&h=128"
            .parse()
            .unwrap();
        assert_eq!(
            parse_media_route(&uri),
            Ok(MediaRoute::Image {
                record_id: "rec-123".into(),
                max_pixels: 768,
                max_height: Some(128),
            })
        );
        let default_uri: tauri::http::Uri = "nami-media://localhost/image/rec%201".parse().unwrap();
        assert_eq!(
            parse_media_route(&default_uri),
            Ok(MediaRoute::Image {
                record_id: "rec 1".into(),
                max_pixels: DEFAULT_PREVIEW_PIXELS,
                max_height: None,
            })
        );
        let icon_uri: tauri::http::Uri = "nami-media://localhost/icon/42".parse().unwrap();
        assert_eq!(
            parse_media_route(&icon_uri),
            Ok(MediaRoute::Icon { source_app_id: 42 })
        );
        for bad in [
            "nami-media://localhost/image/rec-1?w=0",
            "nami-media://localhost/image/rec-1?w=127",
            "nami-media://localhost/image/rec-1?w=2049",
            "nami-media://localhost/image/rec-1?w=768&h=31",
            "nami-media://localhost/image/rec-1?w=768&h=2049",
            "nami-media://localhost/image/..%2Fsecret?w=512",
            "nami-media://localhost/image/..",
            "nami-media://localhost/icon/0",
            "nami-media://localhost/icon/-1",
        ] {
            let uri: tauri::http::Uri = bad.parse().unwrap();
            assert_eq!(parse_media_route(&uri), Err(400), "must reject {bad}");
        }
        let unknown: tauri::http::Uri = "nami-media://localhost/other/1".parse().unwrap();
        assert_eq!(parse_media_route(&unknown), Err(404));
        assert!(authorize_media_request(
            "clipboard",
            &MediaRoute::Image {
                record_id: "a".into(),
                max_pixels: 512,
                max_height: Some(128),
            }
        ));
        assert!(!authorize_media_request(
            "settings",
            &MediaRoute::Image {
                record_id: "a".into(),
                max_pixels: 512,
                max_height: None,
            }
        ));
        assert!(authorize_media_request(
            "settings",
            &MediaRoute::Icon { source_app_id: 1 }
        ));
        assert_eq!(fit_box_max_pixels(1920, 1080, 768, Some(128)), 228);
        assert_eq!(fit_box_max_pixels(1000, 2000, 768, Some(128)), 128);
        assert_eq!(fit_box_max_pixels(2400, 300, 768, Some(128)), 768);
        assert_eq!(fit_box_max_pixels(1920, 1080, 1536, None), 1536);
    }

    #[test]
    fn decodes_valid_png_data_url_and_rewrites_page_source_icons() {
        let valid_data_url = "data:image/png;base64,iVBORw0KGgo=";
        assert_eq!(
            decode_png_data_url(valid_data_url).as_deref(),
            Some(PNG_SIGNATURE.as_slice())
        );
        assert_eq!(decode_png_data_url("data:image/png;base64,AAAA"), None);
        assert_eq!(decode_png_data_url("https://example.com/icon.png"), None);

        let mut sources = BTreeMap::new();
        sources.insert(
            7,
            serde_json::json!({ "name": "Finder", "iconDataUrl": valid_data_url }),
        );
        sources.insert(8, serde_json::json!({ "name": "Nami" }));
        let mut page = ClipboardPage {
            records: Vec::new(),
            sources,
            cursor: None,
            has_more: false,
            total_records: 0,
            pin_paste_keys: Vec::new(),
        };
        rewrite_page_source_icons(&mut page);
        assert_eq!(
            page.sources[&7]["iconDataUrl"].as_str(),
            Some(media_url("/icon/7").as_str())
        );
        assert!(page.sources[&8].get("iconDataUrl").is_none());
    }

    #[test]
    fn caches_and_cleans_stored_and_file_image_thumbnails() {
        let base = std::env::temp_dir().join(format!("nami-media-cache-{}", uuid::Uuid::new_v4()));
        let root = base.join("images");
        let cache = base.join("thumbs");
        std::fs::create_dir_all(&root).unwrap();
        let original = b"\x89PNG\r\n\x1a\noriginal-image-payload";
        let relative = nami::clipboard::store_image(&root, original, "png").unwrap();

        let rendered = b"\x89PNG\r\n\x1a\nresized-thumb-bytes".to_vec();
        let mut render_calls = 0usize;
        let first = load_preview_with_renderer(
            &root,
            Some(&cache),
            PreviewSource::StoredImage {
                relative: relative.clone(),
            },
            512,
            None,
            |bytes, max_pixels| {
                render_calls += 1;
                assert_eq!(bytes, original);
                assert_eq!(max_pixels, 512);
                Ok(rendered.clone())
            },
        )
        .unwrap();
        assert_eq!(first, rendered);
        assert_eq!(render_calls, 1);

        let second = load_preview_with_renderer(
            &root,
            Some(&cache),
            PreviewSource::StoredImage {
                relative: relative.clone(),
            },
            512,
            None,
            |_, _| {
                render_calls += 1;
                Err("should_hit_cache")
            },
        )
        .unwrap();
        assert_eq!(second, rendered);
        assert_eq!(render_calls, 1);

        remove_thumbs_for_image_paths_in(&cache, std::slice::from_ref(&relative));
        let cache_key = stored_image_cache_key(&relative, 512, None).unwrap();
        assert!(!cache.join(cache_key).exists());

        let local = base.join("local.png");
        std::fs::write(&local, original).unwrap();
        let file_thumb = load_preview_with_renderer(
            &root,
            Some(&cache),
            PreviewSource::LocalFile {
                path: local.clone(),
            },
            256,
            Some(128),
            |_, _| Ok(rendered.clone()),
        )
        .unwrap();
        assert_eq!(file_thumb, rendered);
        assert!(std::fs::read_dir(&cache).unwrap().count() == 1);
        remove_file_thumbs_in(&cache);
        assert!(std::fs::read_dir(&cache).unwrap().count() == 0);

        // Verify cache pruning enforces file budget.
        for index in 0..5 {
            write_cached_png(&cache, &format!("img_{index:064x}_256.png"), &rendered);
        }
        prune_thumb_cache(&cache, 2, MAX_THUMB_CACHE_BYTES);
        assert_eq!(std::fs::read_dir(&cache).unwrap().count(), 2);

        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    #[ignore = "explicit synthetic image preview cold/warm and serialization benchmark"]
    fn benchmark_image_preview_cache() {
        let base = std::env::temp_dir().join(format!("nami-media-bench-{}", uuid::Uuid::new_v4()));
        let root = base.join("images");
        let cache = base.join("thumbs");
        std::fs::create_dir_all(&root).unwrap();
        let fixture = include_bytes!("../icons/icon.png");
        let relative = nami::clipboard::store_image(&root, fixture, "png").unwrap();

        let cold_iters = 20u32;
        let cold_start = std::time::Instant::now();
        let mut preview = Vec::new();
        for _ in 0..cold_iters {
            preview = platform::clipboard_image_preview(fixture, 768).unwrap();
        }
        let cold_avg_ms = cold_start.elapsed().as_secs_f64() * 1000.0 / f64::from(cold_iters);

        // Prime disk cache once.
        let primed = load_preview_with_cache(
            &root,
            Some(&cache),
            PreviewSource::StoredImage {
                relative: relative.clone(),
            },
            768,
            None,
        )
        .unwrap();
        assert_eq!(primed, preview);

        let warm_iters = 200u32;
        let warm_start = std::time::Instant::now();
        for _ in 0..warm_iters {
            let hit = load_preview_with_cache(
                &root,
                Some(&cache),
                PreviewSource::StoredImage {
                    relative: relative.clone(),
                },
                768,
                None,
            )
            .unwrap();
            assert_eq!(hit.len(), preview.len());
        }
        let warm_avg_ms = warm_start.elapsed().as_secs_f64() * 1000.0 / f64::from(warm_iters);

        let json_iters = 20u32;
        let json_start = std::time::Instant::now();
        let mut json_len = 0usize;
        for _ in 0..json_iters {
            let encoded = serde_json::to_vec(&serde_json::json!({
                "bytes": &preview,
                "mime": "image/png",
            }))
            .unwrap();
            json_len = encoded.len();
        }
        let json_avg_ms = json_start.elapsed().as_secs_f64() * 1000.0 / f64::from(json_iters);

        eprintln!(
            "BENCH_MEDIA source_bytes={} preview_bytes={} json_bytes={} json_ratio={:.2}x cold_ms={:.3} warm_ms={:.3} speedup={:.1}x json_ser_ms={:.3}",
            fixture.len(),
            preview.len(),
            json_len,
            json_len as f64 / preview.len() as f64,
            cold_avg_ms,
            warm_avg_ms,
            cold_avg_ms / warm_avg_ms.max(0.0001),
            json_avg_ms,
        );

        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn prune_stale_caches_keeps_live_thumbs_and_prunes_stale_entries() {
        let base = std::env::temp_dir().join(format!("nami-prune-cache-{}", uuid::Uuid::new_v4()));
        let thumbs = base.join("clipboard-thumbs");
        let drags = base.join("drag-exports");
        std::fs::create_dir_all(&thumbs).unwrap();
        std::fs::create_dir_all(&drags).unwrap();

        let live_hash = "a".repeat(64);
        let orphan_hash = "b".repeat(64);
        let live_thumb = thumbs.join(format!("img_{live_hash}_512.png"));
        let orphan_thumb = thumbs.join(format!("img_{orphan_hash}_512.png"));
        let stale_tmp = thumbs.join(".partial.tmp");
        let file_thumb = thumbs.join("file_1234_512.png");
        let drag_file = drags.join("Nami_export.png");

        std::fs::write(&live_thumb, b"live-thumb").unwrap();
        std::fs::write(&orphan_thumb, b"orphan-thumb").unwrap();
        std::fs::write(&stale_tmp, b"tmp").unwrap();
        std::fs::write(&file_thumb, b"file-thumb").unwrap();
        std::fs::write(&drag_file, b"drag-export").unwrap();

        let mut referenced = std::collections::HashSet::new();
        referenced.insert(format!("origin/aa/{live_hash}.png"));

        // With 30-min drag max age, a freshly created drag export is preserved while orphan thumb & tmp are pruned.
        let (removed, bytes) = prune_stale_caches_in(
            &base,
            &referenced,
            std::time::Duration::ZERO,
            std::time::Duration::from_secs(30 * 60),
        );
        assert_eq!(removed, 2);
        assert_eq!(bytes, (b"orphan-thumb".len() + b"tmp".len()) as u64);
        assert!(live_thumb.exists());
        assert!(file_thumb.exists());
        assert!(drag_file.exists());
        assert!(!orphan_thumb.exists());
        assert!(!stale_tmp.exists());

        // Once expired (ZERO max age), drag export is pruned while live thumb stays intact.
        let (removed_expired, _) = prune_stale_caches_in(
            &base,
            &referenced,
            std::time::Duration::ZERO,
            std::time::Duration::ZERO,
        );
        assert_eq!(removed_expired, 1);
        assert!(live_thumb.exists());
        assert!(!drag_file.exists());

        std::fs::remove_dir_all(base).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn local_file_preview_caches_valid_images_and_rejects_pixel_bombs_and_missing_files() {
        let base = std::env::temp_dir().join(format!("nami-local-media-{}", uuid::Uuid::new_v4()));
        let cache = base.join("thumbs");
        std::fs::create_dir_all(&base).unwrap();

        let valid = base.join("photo.jpg");
        std::fs::write(
            &valid,
            include_bytes!("../../tests/fixtures/clipboard-wide.jpg"),
        )
        .unwrap();
        let first = load_preview_with_cache(
            &base,
            Some(&cache),
            PreviewSource::LocalFile {
                path: valid.clone(),
            },
            512,
            Some(128),
        )
        .unwrap();
        assert!(first.starts_with(PNG_SIGNATURE));
        let second = load_preview_with_cache(
            &base,
            Some(&cache),
            PreviewSource::LocalFile {
                path: valid.clone(),
            },
            512,
            Some(128),
        )
        .unwrap();
        assert_eq!(first, second);

        // Removing the local file immediately fails with clipboard_file_missing.
        std::fs::remove_file(&valid).unwrap();
        assert_eq!(
            load_preview_with_cache(
                &base,
                Some(&cache),
                PreviewSource::LocalFile { path: valid },
                512,
                Some(128),
            )
            .unwrap_err(),
            "clipboard_file_missing"
        );

        // A compressed pixel bomb (8193x8193 > 64M pixels) is rejected before decoding.
        let bomb = base.join("bomb.png");
        std::fs::write(
            &bomb,
            include_bytes!("../../tests/fixtures/clipboard-oversized.png"),
        )
        .unwrap();
        assert_eq!(
            load_preview_with_cache(
                &base,
                Some(&cache),
                PreviewSource::LocalFile { path: bomb },
                512,
                Some(128),
            )
            .unwrap_err(),
            "clipboard_image_limit"
        );

        std::fs::remove_dir_all(base).unwrap();
    }
}
