use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{io::Write, path::Path};

pub const MAX_FILE_COUNT: usize = 4096;
pub const MAX_TEXT_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_IMAGE_BYTES: usize = 64 * 1024 * 1024;

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
    pub date: String,
    pub sequence: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClipboardPage {
    pub records: Vec<ClipboardRecord>,
    pub cursor: Option<ClipboardCursor>,
    pub has_more: bool,
    pub total_records: usize,
}

pub fn fingerprint(kind: &str, bytes: &[u8]) -> String {
    let mut hash = Sha256::new();
    hash.update(kind.as_bytes());
    hash.update([0]);
    hash.update(bytes);
    format!("{:x}", hash.finalize())
}

fn valid_image_name(relative: &str) -> bool {
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
        assert_eq!(read_image(&directory, &relative).unwrap(), bytes);
        assert!(read_image(&directory, "../private.png").is_err());
        assert!(store_image(&directory, b"invalid", "png").is_err());
        std::fs::write(directory.join(&relative), b"corrupt").unwrap();
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
