pub mod open_target;
pub mod rich_text;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Write, path::Path};

pub const MAX_FILE_COUNT: usize = 4096;
pub const MAX_RICH_TEXT_BYTES: usize = 1024 * 1024;
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_IGNORED_PASTEBOARD_TYPES: usize = 256;
pub const MAX_IGNORED_REGEXPS: usize = 128;
pub const MAX_IGNORED_REGEX_BYTES: usize = 512;
pub const MAX_IGNORED_REGEX_COMPILED_BYTES: usize = 256 * 1024;
pub const UNIVERSAL_CLIPBOARD_PASTEBOARD_TYPE: &str = "com.apple.is-remote-clipboard";
pub const DEFAULT_IGNORED_PASTEBOARD_TYPES: [&str; 5] = [
    "Pasteboard generator type",
    "com.agilebits.onepassword",
    "com.typeit4me.clipping",
    "de.petermaurer.TransientPasteboardType",
    "net.antelle.keeweb",
];

pub fn valid_pasteboard_type(kind: &str) -> bool {
    !kind.is_empty()
        && kind.len() <= 255
        && kind == kind.trim()
        && kind
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-_:/ ".contains(&byte))
}

pub fn matches_secret_pattern(text: &str) -> bool {
    use std::sync::OnceLock;
    static PATTERNS: OnceLock<Vec<regex::Regex>> = OnceLock::new();
    let patterns = PATTERNS.get_or_init(|| {
        [
            r"-----BEGIN (?:RSA |DSA |EC |OPENSSH |PGP |ENCRYPTED )?PRIVATE KEY-----",
            r"(?:^|[^A-Za-z0-9_-])eyJ[A-Za-z0-9_-]{8,}\.eyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{10,}(?:$|[^A-Za-z0-9_-])",
            r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b",
            r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36}\b|\bgithub_pat_[A-Za-z0-9_]{22,}\b",
            r"\bsk-(?:proj-|ant-api\d{2}-|live_|test_)?[A-Za-z0-9_-]{20,}\b",
            r"\bAIza[0-9A-Za-z_-]{35}\b",
            r"\bxox[baprs]-[0-9A-Za-z-]{10,}\b",
            r"\bglpat-[A-Za-z0-9_-]{20,}\b",
            r"\bnpm_[A-Za-z0-9]{36}\b",
            r"\bhf_[A-Za-z0-9]{20,}\b",
        ]
        .into_iter()
        .map(|pattern| regex::Regex::new(pattern).expect("built-in secret regex must compile"))
        .collect()
    });
    const HEAD_SCAN_BYTES: usize = 64 * 1024;
    const TAIL_SCAN_BYTES: usize = 16 * 1024;
    if text.len() <= HEAD_SCAN_BYTES + TAIL_SCAN_BYTES {
        return patterns.iter().any(|re| re.is_match(text));
    }
    let mut head_end = HEAD_SCAN_BYTES;
    while head_end > 0 && !text.is_char_boundary(head_end) {
        head_end -= 1;
    }
    let mut tail_start = text.len() - TAIL_SCAN_BYTES;
    while tail_start < text.len() && !text.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let head = &text[..head_end];
    let tail = &text[tail_start..];
    patterns
        .iter()
        .any(|re| re.is_match(head) || re.is_match(tail))
}

pub fn compile_ignored_regex(pattern: &str) -> Result<regex::Regex, String> {
    if pattern.trim().is_empty() || pattern.len() > MAX_IGNORED_REGEX_BYTES {
        return Err("clipboard_invalid_regex".into());
    }
    regex::RegexBuilder::new(pattern)
        .size_limit(MAX_IGNORED_REGEX_COMPILED_BYTES)
        .build()
        .map_err(|_| "clipboard_invalid_regex".into())
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RecordingKind {
    Text,
    Image,
    Files,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct RecordingTypes {
    pub text: bool,
    pub image: bool,
    pub files: bool,
}

impl Default for RecordingTypes {
    fn default() -> Self {
        Self {
            text: true,
            image: true,
            files: true,
        }
    }
}

impl RecordingTypes {
    pub fn set(&mut self, kind: RecordingKind, enabled: bool) {
        *match kind {
            RecordingKind::Text => &mut self.text,
            RecordingKind::Image => &mut self.image,
            RecordingKind::Files => &mut self.files,
        } = enabled;
    }

    pub fn any(self) -> bool {
        self.text || self.image || self.files
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct HistoryPolicy {
    pub limit: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_age_days: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_storage_mb: Option<u32>,
}

impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            limit: 500,
            max_age_days: None,
            max_storage_mb: None,
        }
    }
}

#[derive(Serialize)]
pub struct CleanupPreview {
    pub count: usize,
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub categories: Option<[usize; 3]>,
}

impl HistoryPolicy {
    pub fn valid(self) -> bool {
        (1..=1_000_000).contains(&self.limit)
            && self
                .max_age_days
                .is_none_or(|days| [1, 7, 30].contains(&days))
            && self
                .max_storage_mb
                .is_none_or(|mb| [500, 1024, 2048].contains(&mb))
    }

    pub fn max_storage_bytes(self) -> Option<u64> {
        self.max_storage_mb.map(|mb| u64::from(mb) * 1024 * 1024)
    }

    pub fn from_store(value: Option<String>) -> Self {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct Saved {
            limit: Option<u32>,
            max_age_days: Option<u32>,
            max_storage_mb: Option<u32>,
        }
        value
            .and_then(|value| serde_json::from_str::<Saved>(&value).ok())
            .and_then(|value| {
                value.limit.map(|limit| Self {
                    limit,
                    max_age_days: value.max_age_days.filter(|days| [1, 7, 30].contains(days)),
                    max_storage_mb: value
                        .max_storage_mb
                        .filter(|mb| [500, 1024, 2048].contains(mb)),
                })
            })
            .filter(|policy| policy.valid())
            .unwrap_or_default()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardTextFormatting {
    pub html: Option<Vec<u8>>,
    pub rtf: Option<Vec<u8>>,
}

impl ClipboardTextFormatting {
    pub fn valid(&self) -> bool {
        (self.html.is_some() || self.rtf.is_some())
            && self
                .html
                .iter()
                .chain(&self.rtf)
                .all(|data| !data.is_empty() && data.len() <= MAX_RICH_TEXT_BYTES)
    }

    pub fn fingerprint(&self, text: &str) -> String {
        let mut hash = Sha256::new();
        hash.update(b"rich-text\0");
        for data in [
            text.as_bytes(),
            self.html.as_deref().unwrap_or_default(),
            self.rtf.as_deref().unwrap_or_default(),
        ] {
            hash.update((data.len() as u64).to_be_bytes());
            hash.update(data);
        }
        format!("{:x}", hash.finalize())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipboardFileRepresentation {
    pub paths: Vec<String>,
    pub image_path: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardRecord {
    pub id: String,
    pub kind: String,
    pub text: String,
    pub date: String,
    pub is_favorite: bool,
    pub is_pinned: bool,
    pub source_app: Option<serde_json::Value>,
    pub image_path: Option<String>,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardCursor {
    pub pinned: bool,
    pub created_at: i64,
    pub sequence: i64,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ClipboardSearchMode {
    #[default]
    Exact,
    Fuzzy,
    Regex,
    Mixed,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ClipboardSearchOptions {
    pub mode: ClipboardSearchMode,
    pub body: bool,
    pub note: bool,
    pub image_text: bool,
}

impl Default for ClipboardSearchOptions {
    fn default() -> Self {
        Self {
            mode: ClipboardSearchMode::Exact,
            body: true,
            note: true,
            image_text: true,
        }
    }
}

impl ClipboardSearchOptions {
    pub fn has_scope(self) -> bool {
        self.body || self.note || self.image_text
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardSummary {
    pub title: String,
    pub id: String,
    pub kind: String,
    pub text: String,
    pub date: String,
    pub is_favorite: bool,
    pub is_pinned: bool,
    pub source_app_id: Option<i64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub image_text_match: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_file_paths: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_rich_text: bool,
    /// Image OCR finished and stored an empty original. Omitted when false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub image_text_empty: bool,
    /// Saved Command letter for a pinned record. Omitted when the pin has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pin_paste_key: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PinPasteKey {
    pub id: String,
    pub key: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPage {
    pub records: Vec<ClipboardSummary>,
    pub sources: std::collections::BTreeMap<i64, serde_json::Value>,
    pub cursor: Option<ClipboardCursor>,
    pub has_more: bool,
    pub total_records: usize,
    /// Every pinned letter, including records outside this page or filter.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pin_paste_keys: Vec<PinPasteKey>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardQueryResult {
    pub page: ClipboardPage,
    pub alternate: Option<ClipboardPage>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardDetail {
    pub title: String,
    pub text: String,
    pub kind: String,
    pub id: String,
    pub date: String,
    pub is_favorite: bool,
    pub is_pinned: bool,
    pub source_app: Option<serde_json::Value>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub size: Option<u64>,
    pub character_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_text_format: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rich_text_html: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_index_status: Option<&'static str>,
    pub files: Vec<FileEntry>,
    #[serde(skip)]
    pub image_path: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub path: String,
    pub name: String,
    pub exists: bool,
    pub is_dir: bool,
    pub is_image: bool,
    pub icon_data_url: Option<String>,
}

pub fn file_entries(text: &str) -> Result<Vec<FileEntry>, String> {
    file_entries_with_limit(text, 3)
}

pub fn all_file_entries(text: &str) -> Result<Vec<FileEntry>, String> {
    file_entries_with_limit(text, MAX_FILE_COUNT)
}

pub fn file_entries_from_paths(paths: Vec<String>) -> Result<Vec<FileEntry>, String> {
    file_entries_from_paths_with_limit(paths, MAX_FILE_COUNT)
}

fn file_entries_with_limit(text: &str, limit: usize) -> Result<Vec<FileEntry>, String> {
    let paths: Vec<String> = serde_json::from_str(text).map_err(|_| "storage_corrupt")?;
    file_entries_from_paths_with_limit(paths, limit)
}

fn file_entries_from_paths_with_limit(
    paths: Vec<String>,
    limit: usize,
) -> Result<Vec<FileEntry>, String> {
    if paths.is_empty() || paths.len() > MAX_FILE_COUNT {
        return Err("storage_corrupt".into());
    }
    paths
        .into_iter()
        .take(limit)
        .map(|path| {
            let file = Path::new(&path);
            if !file.is_absolute() || path.contains('\0') {
                return Err("storage_corrupt".into());
            }
            let metadata = std::fs::metadata(file).ok();
            let is_dir = metadata.as_ref().is_some_and(|value| value.is_dir());
            let is_image = !is_dir
                && file
                    .extension()
                    .and_then(|value| value.to_str())
                    .is_some_and(|extension| {
                        [
                            "png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff", "ico",
                            "heic", "heif", "avif", "svg", "apng",
                        ]
                        .contains(&extension.to_ascii_lowercase().as_str())
                    });
            Ok(FileEntry {
                name: file
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or(&path)
                    .to_owned(),
                path,
                exists: metadata.is_some(),
                is_dir,
                is_image,
                icon_data_url: None,
            })
        })
        .collect()
}

pub fn open_file_preview(path: &Path) -> Result<File, String> {
    let metadata = std::fs::metadata(path).map_err(|_| "clipboard_file_missing")?;
    if !metadata.is_file() {
        return Err("invalid_argument".into());
    }
    if metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("clipboard_image_limit".into());
    }
    File::open(path).map_err(|_| "clipboard_file_missing".into())
}

pub fn read_file_preview(path: &Path) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = open_file_preview(path)?;
    let mut bytes = Vec::new();
    file.take(MAX_IMAGE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "clipboard_file_missing")?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("clipboard_image_limit".into());
    }
    Ok(bytes)
}

pub fn fingerprint(kind: &str, bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

pub(crate) fn valid_image_name(relative: &str) -> bool {
    let parts: Vec<_> = relative.split('/').collect();
    if parts.len() != 3 || parts[0] != "origin" || parts[1].len() != 2 {
        return false;
    }
    let Some((hash, extension)) = parts[2].rsplit_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        && &hash[..2] == parts[1]
        && ["png", "tiff"].contains(&extension)
}

fn reject_symlinks(root: &Path, relative: &str) -> Result<(), String> {
    let mut path = root.to_path_buf();
    for component in std::iter::once("").chain(relative.split('/')) {
        path.push(component);
        match std::fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err("storage_path".into());
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("storage_unavailable".into()),
        }
    }
    Ok(())
}

pub fn store_image(root: &Path, bytes: &[u8], extension: &str) -> Result<String, String> {
    store_image_with_status(root, bytes, extension).map(|(path, _)| path)
}

pub fn store_image_with_status(
    root: &Path,
    bytes: &[u8],
    extension: &str,
) -> Result<(String, bool), String> {
    let format = match extension {
        "png" if bytes.starts_with(b"\x89PNG\r\n\x1a\n") => "png",
        "tiff" if bytes.starts_with(b"II*\0") || bytes.starts_with(b"MM\0*") => "tiff",
        _ => return Err("clipboard_image_format".into()),
    };
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err("clipboard_image_limit".into());
    }
    let hash = fingerprint("image", bytes);
    let relative = format!("origin/{}/{}.{}", &hash[..2], hash, format);
    reject_symlinks(root, &relative)?;
    let path = root.join(&relative);
    if path.exists() {
        if read_image(root, &relative)? != bytes {
            return Err("storage_corrupt".into());
        }
        return Ok((relative, false));
    }
    std::fs::create_dir_all(path.parent().ok_or("storage_path")?)
        .map_err(|_| "storage_unavailable")?;
    let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| -> Result<bool, String> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|_| "storage_unavailable")?;
        file.write_all(bytes).map_err(|_| "storage_unavailable")?;
        file.sync_all().map_err(|_| "storage_unavailable")?;
        match std::fs::hard_link(&temporary, &path) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if read_image(root, &relative)? != bytes {
                    return Err("storage_corrupt".into());
                }
                Ok(false)
            }
            Err(_) => Err("storage_unavailable".into()),
        }
    })();
    let _ = std::fs::remove_file(&temporary);
    let created = result?;
    Ok((relative, created))
}

pub fn verified_image_path(root: &Path, relative: &str) -> Result<std::path::PathBuf, String> {
    open_verified_image(root, relative)?;
    Ok(root.join(relative))
}

/// Returns the rewound file and the byte count covered by the verified hash.
pub fn open_verified_image(root: &Path, relative: &str) -> Result<(File, u64), String> {
    use std::io::{Read, Seek};
    if !valid_image_name(relative) {
        return Err("storage_path".into());
    }
    reject_symlinks(root, relative)?;
    let path = root.join(relative);
    let metadata = std::fs::metadata(&path).map_err(|_| "clipboard_image_missing")?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("clipboard_image_limit".into());
    }
    let filename = relative.rsplit('/').next().ok_or("storage_path")?;
    let expected = filename.split('.').next().ok_or("storage_path")?;
    let mut file = File::open(&path).map_err(|_| "storage_unavailable")?;
    let mut hasher = Sha256::new();
    hasher.update(b"image");
    hasher.update([0]);
    let mut total = 0usize;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|_| "storage_unavailable")?;
        if count == 0 {
            break;
        }
        total = total.saturating_add(count);
        if total > MAX_IMAGE_BYTES {
            return Err("clipboard_image_limit".into());
        }
        hasher.update(&buffer[..count]);
    }
    if format!("{:x}", hasher.finalize()) != expected {
        return Err("storage_corrupt".into());
    }
    file.rewind().map_err(|_| "storage_unavailable")?;
    Ok((file, total as u64))
}

pub fn read_image(root: &Path, relative: &str) -> Result<Vec<u8>, String> {
    if !valid_image_name(relative) {
        return Err("storage_path".into());
    }
    reject_symlinks(root, relative)?;
    let path = root.join(relative);
    let metadata = std::fs::metadata(&path).map_err(|_| "clipboard_image_missing")?;
    if !metadata.is_file() || metadata.len() > MAX_IMAGE_BYTES as u64 {
        return Err("clipboard_image_limit".into());
    }
    let bytes = std::fs::read(path).map_err(|_| "storage_unavailable")?;
    let filename = relative.rsplit('/').next().ok_or("storage_path")?;
    let expected = filename.split('.').next().ok_or("storage_path")?;
    if fingerprint("image", &bytes) != expected {
        return Err("storage_corrupt".into());
    }
    Ok(bytes)
}

pub fn remove_image(root: &Path, relative: &str) -> Result<(), String> {
    if !valid_image_name(relative) {
        return Err("storage_path".into());
    }
    reject_symlinks(root, relative)?;
    match std::fs::remove_file(root.join(relative)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err("storage_unavailable".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_storage_is_deduplicated_and_validates_paths_and_bytes() {
        let directory = std::env::temp_dir().join(format!("nami-images-{}", uuid::Uuid::new_v4()));
        let bytes = b"\x89PNG\r\n\x1a\nsynthetic";
        let relative = store_image(&directory, bytes, "png").unwrap();
        assert_eq!(store_image(&directory, bytes, "png").unwrap(), relative);
        assert_eq!(
            verified_image_path(&directory, &relative).unwrap(),
            directory.join(&relative)
        );
        assert_eq!(read_image(&directory, &relative).unwrap(), bytes);
        assert!(read_image(&directory, "../private.png").is_err());
        assert!(verified_image_path(&directory, "../private.png").is_err());
        assert!(store_image(&directory, b"invalid", "png").is_err());
        std::fs::write(directory.join(&relative), b"corrupt").unwrap();
        assert_eq!(
            verified_image_path(&directory, &relative).unwrap_err(),
            "storage_corrupt"
        );
        assert_eq!(
            read_image(&directory, &relative).unwrap_err(),
            "storage_corrupt"
        );
        remove_image(&directory, &relative).unwrap();
        remove_image(&directory, &relative).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn image_store_reports_new_resources_without_claiming_existing_files() {
        let directory =
            std::env::temp_dir().join(format!("nami-image-status-{}", uuid::Uuid::new_v4()));
        let bytes = b"\x89PNG\r\n\x1a\nsynthetic status image";
        let (path, created) = store_image_with_status(&directory, bytes, "png").unwrap();
        assert!(created);
        let (existing, created) = store_image_with_status(&directory, bytes, "png").unwrap();
        assert!(!created);
        assert_eq!(existing, path);
        assert_eq!(read_image(&directory, &path).unwrap(), bytes);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_image_stores_report_one_new_resource() {
        let directory =
            std::env::temp_dir().join(format!("nami-image-race-{}", uuid::Uuid::new_v4()));
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let root = directory.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    store_image_with_status(
                        &root,
                        b"\x89PNG\r\n\x1a\nsynthetic concurrent image",
                        "png",
                    )
                    .unwrap()
                })
            })
            .collect();
        let results: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|(_, created)| *created).count(), 1);
        assert_eq!(results[0].0, results[1].0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn image_storage_rejects_symlinked_resources() {
        let directory = std::env::temp_dir().join(format!("nami-images-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        std::os::unix::fs::symlink(std::env::temp_dir(), directory.join("origin")).unwrap();
        assert_eq!(
            store_image(&directory, b"\x89PNG\r\n\x1a\nsynthetic", "png").unwrap_err(),
            "storage_path"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
}

#[cfg(test)]
mod file_display_tests {
    use super::*;
    #[test]
    fn file_list_limits_preserve_names_and_detect_missing_directories_and_images() {
        let root = std::env::temp_dir().join(format!("nami-file-display-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let image = root.join("test image.PNG");
        std::fs::write(&image, b"synthetic").unwrap();
        let missing = root.join("missing.txt");
        let paths = vec![image, root.clone(), missing, root.join("fourth.txt")];
        let entries = file_entries(&serde_json::to_string(&paths).unwrap()).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].name, "test image.PNG");
        assert!(entries[0].exists && entries[0].is_image && !entries[0].is_dir);
        assert!(entries[1].is_dir && !entries[1].is_image);
        assert!(!entries[2].exists);
        assert_eq!(entries[2].path, paths[2].to_string_lossy());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn preview_reads_only_regular_bounded_files_and_reports_missing_references() {
        let root = std::env::temp_dir().join(format!("nami-preview-read-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        assert_eq!(read_file_preview(&root).unwrap_err(), "invalid_argument");
        let path = root.join("image.png");
        assert_eq!(
            read_file_preview(&path).unwrap_err(),
            "clipboard_file_missing"
        );
        std::fs::write(&path, b"synthetic image").unwrap();
        assert_eq!(read_file_preview(&path).unwrap(), b"synthetic image");
        std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .unwrap()
            .set_len(MAX_IMAGE_BYTES as u64 + 1)
            .unwrap();
        assert_eq!(
            read_file_preview(&path).unwrap_err(),
            "clipboard_image_limit"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn file_list_rejects_invalid_storage_without_interpreting_it_as_a_path() {
        for text in [
            "not json",
            "[]",
            "[1]",
            "[\"relative.txt\"]",
            "[\"/tmp/a\\u0000\"]",
        ] {
            assert_eq!(file_entries(text).unwrap_err(), "storage_corrupt");
            assert_eq!(all_file_entries(text).unwrap_err(), "storage_corrupt");
        }
        assert_eq!(
            all_file_entries("[\"/tmp/a\",\"/tmp/b\",\"/tmp/c\",\"relative\"]").unwrap_err(),
            "storage_corrupt"
        );
        let paths = vec![std::env::temp_dir().join("a"); MAX_FILE_COUNT + 1];
        assert_eq!(
            file_entries(&serde_json::to_string(&paths).unwrap()).unwrap_err(),
            "storage_corrupt"
        );
    }

    #[test]
    fn validates_custom_pasteboard_types_and_ignored_regexps() {
        assert!(valid_pasteboard_type("org.example.SecretType"));
        assert!(valid_pasteboard_type("com.bitwarden.password:v1"));
        assert!(valid_pasteboard_type("Pasteboard generator type"));
        assert!(!valid_pasteboard_type(""));
        assert!(!valid_pasteboard_type("   "));
        assert!(!valid_pasteboard_type(" leading"));
        assert!(!valid_pasteboard_type("trailing "));
        assert!(!valid_pasteboard_type("line\nbreak"));
        assert!(!valid_pasteboard_type("tab\there"));
        assert!(!valid_pasteboard_type(&"a".repeat(256)));

        let compiled = compile_ignored_regex("^sk-[A-Za-z0-9]{8,}$").unwrap();
        assert!(compiled.is_match("sk-12345678"));
        assert!(!compiled.is_match("normal text"));
        assert_eq!(
            compile_ignored_regex("   ").unwrap_err(),
            "clipboard_invalid_regex"
        );
        assert_eq!(
            compile_ignored_regex("[unclosed").unwrap_err(),
            "clipboard_invalid_regex"
        );
        assert_eq!(
            compile_ignored_regex(&"a".repeat(MAX_IGNORED_REGEX_BYTES + 1)).unwrap_err(),
            "clipboard_invalid_regex"
        );
        assert_eq!(
            compile_ignored_regex("a{100000}").unwrap_err(),
            "clipboard_invalid_regex"
        );
    }

    #[test]
    fn built_in_secret_patterns_match_credentials_without_flagging_normal_text() {
        let samples = [
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjE=",
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA",
            "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
            "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE",
            "ghp_1234567890abcdefghijklmnopqrstuvwxyz",
            "sk-proj-1234567890abcdefghijklmnop",
            "AIzaSyD-1234567890abcdefghijklmnopqrstuvwxyz"[..39].as_ref(),
            "xoxb-1234567890-abcdefghij",
            "glpat-1234567890abcdefghij",
            "npm_1234567890abcdefghijklmnopqrstuvwxyz",
            "hf_1234567890abcdefghij",
        ];
        for secret in samples {
            assert!(matches_secret_pattern(secret), "expected secret: {secret}");
        }
        for benign in [
            "Hello world",
            "https://example.com/docs/auth",
            "sk-short",
            "eyJnotajwt",
            "const value = 42;",
        ] {
            assert!(
                !matches_secret_pattern(benign),
                "unexpected match: {benign}"
            );
        }
    }
}
