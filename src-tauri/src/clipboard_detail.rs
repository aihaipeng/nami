use crate::platform;
use nami::clipboard::{self, ClipboardDetail};
use std::path::Path;

pub(crate) fn enrich(
    mut detail: ClipboardDetail,
    root: &Path,
    measure_files: bool,
    image_file_paths: Option<Vec<String>>,
) -> Result<ClipboardDetail, String> {
    let image = match detail.kind.as_str() {
        "image" => {
            if let Some(paths) = image_file_paths {
                detail.files = clipboard::file_entries_from_paths(paths)?;
            }
            detail
                .image_path
                .as_deref()
                .and_then(|relative| clipboard::open_verified_image(root, relative).ok())
                .map(|(file, size)| {
                    detail.size = Some(size);
                    file
                })
        }
        "files" => {
            detail.files = clipboard::all_file_entries(&detail.text)?;
            if measure_files {
                detail.size = detail.files.iter().try_fold(0u64, |total, entry| {
                    let metadata = std::fs::metadata(&entry.path).ok()?;
                    metadata.is_file().then_some(())?;
                    total.checked_add(metadata.len())
                });
            }
            match detail.files.as_slice() {
                [entry] if entry.exists && entry.is_image => {
                    clipboard::open_file_preview(Path::new(&entry.path)).ok()
                }
                _ => None,
            }
        }
        "text" => None,
        _ => return Err("storage_corrupt".into()),
    };
    if let Some((width, height)) =
        image.and_then(|file| platform::image_dimensions_from_file(&file).ok())
    {
        detail.width = Some(width);
        detail.height = Some(height);
    }
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::{clipboard::ClipboardRecord, database::HistoryDatabase};

    const WIDE_PNG: &[u8] = include_bytes!("../../tests/fixtures/clipboard-wide.png");
    const WIDE_DIMENSIONS: (Option<u32>, Option<u32>) =
        if cfg!(all(target_os = "macos", not(feature = "portable-ui"))) {
            (Some(2048), Some(256))
        } else {
            (None, None)
        };

    fn detail(kind: &str, text: &str) -> ClipboardDetail {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        database
            .clipboard_insert(&ClipboardRecord {
                id: "detail".into(),
                kind: kind.into(),
                text: text.into(),
                date: "2026-11-16T12:00:00Z".into(),
                is_favorite: false,
                is_pinned: false,
                source_app: Some(serde_json::json!({"name":"Synthetic Application"})),
                image_path: None,
                fingerprint: clipboard::fingerprint(kind, text.as_bytes()),
            })
            .unwrap();
        database.clipboard_detail("detail").unwrap()
    }

    #[test]
    fn text_detail_returns_complete_unicode_content_and_record_metadata() {
        let text = "Full 🌊\n中文 text ".repeat(200);
        let result = enrich(detail("text", &text), Path::new("unused"), true, None).unwrap();
        assert_eq!(result.text, text);
        assert_eq!(result.character_count, Some(text.chars().count()));
        assert_eq!(result.id, "detail");
        assert_eq!(result.source_app.unwrap()["name"], "Synthetic Application");
        assert!(result.width.is_none() && result.height.is_none() && result.size.is_none());
        assert!(result.files.is_empty());
    }

    #[test]
    fn file_detail_lists_every_file_and_never_reports_partial_or_directory_sizes() {
        let root = std::env::temp_dir().join(format!("nami-detail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let paths: Vec<_> = (0..5).map(|i| root.join(format!("file-{i}.txt"))).collect();
        for path in &paths {
            std::fs::write(path, b"synthetic").unwrap();
        }
        let text = serde_json::to_string(&paths).unwrap();
        let result = enrich(detail("files", &text), &root, true, None).unwrap();
        assert_eq!(result.files.len(), 5);
        assert_eq!(result.size, Some(45));
        assert!(enrich(detail("files", &text), &root, false, None)
            .unwrap()
            .size
            .is_none());
        assert!(result.character_count.is_none() && result.width.is_none());
        std::fs::remove_file(&paths[1]).unwrap();
        let missing = enrich(detail("files", &text), &root, true, None).unwrap();
        assert!(!missing.files[1].exists);
        assert!(missing.size.is_none());
        let directory = serde_json::to_string(&vec![root.clone()]).unwrap();
        assert!(enrich(detail("files", &directory), &root, true, None)
            .unwrap()
            .size
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    fn files_detail(root: &Path, paths: &[&Path], measure_files: bool) -> ClipboardDetail {
        let text = serde_json::to_string(paths).unwrap();
        enrich(detail("files", &text), root, measure_files, None).unwrap()
    }

    #[test]
    fn single_image_file_returns_original_dimensions_and_byte_size() {
        let root = std::env::temp_dir().join(format!("nami-detail-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("synthetic.png");
        std::fs::write(&path, WIDE_PNG).unwrap();
        for measure in [true, false] {
            let result = files_detail(&root, &[&path], measure);
            assert_eq!((result.width, result.height), WIDE_DIMENSIONS);
            assert_eq!(result.size, measure.then_some(WIDE_PNG.len() as u64));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn invalid_oversized_and_multiple_images_keep_file_sizes_without_dimensions() {
        let root =
            std::env::temp_dir().join(format!("nami-detail-limits-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let first = root.join("first.png");
        let second = root.join("second.png");
        std::fs::write(&first, WIDE_PNG).unwrap();
        std::fs::write(&second, WIDE_PNG).unwrap();
        let multiple = files_detail(&root, &[&first, &second], true);
        assert_eq!(multiple.files.len(), 2);
        assert_eq!(multiple.size, Some(2 * WIDE_PNG.len() as u64));
        assert_eq!((multiple.width, multiple.height), (None, None));

        std::fs::write(&second, b"not a PNG").unwrap();
        let invalid = files_detail(&root, &[&second], true);
        assert_eq!(
            (invalid.width, invalid.height, invalid.size),
            (None, None, Some(9))
        );
        let over_limit = clipboard::MAX_IMAGE_BYTES as u64 + 1;
        std::fs::OpenOptions::new()
            .write(true)
            .open(&first)
            .unwrap()
            .set_len(over_limit)
            .unwrap();
        for measure in [true, false] {
            let large = files_detail(&root, &[&first], measure);
            assert_eq!((large.width, large.height), (None, None));
            assert_eq!(large.size, measure.then_some(over_limit));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_image_dimensions_use_verified_attachment_and_reject_changed_bytes() {
        let root =
            std::env::temp_dir().join(format!("nami-detail-private-{}", uuid::Uuid::new_v4()));
        let relative = clipboard::store_image(&root, WIDE_PNG, "png").unwrap();
        let make_detail = || {
            let mut value = detail("text", "");
            value.kind = "image".into();
            value.character_count = None;
            value.image_path = Some(relative.clone());
            value
        };
        let result = enrich(make_detail(), &root, true, None).unwrap();
        assert_eq!((result.width, result.height), WIDE_DIMENSIONS);
        assert_eq!(result.size, Some(WIDE_PNG.len() as u64));
        assert!(result.character_count.is_none());
        assert!(result.files.is_empty());
        let original = root.join("local-photo.png");
        let with_original = || {
            enrich(
                make_detail(),
                &root,
                false,
                Some(vec![original.to_string_lossy().into_owned()]),
            )
            .unwrap()
        };
        std::fs::write(&original, b"different local file").unwrap();
        let local = with_original();
        assert_eq!(local.files.len(), 1);
        assert_eq!(local.files[0].path, original.to_string_lossy());
        assert_eq!(local.files[0].name, "local-photo.png");
        assert!(local.files[0].exists);
        assert!(local.files[0].is_image);
        assert_eq!((local.width, local.height), WIDE_DIMENSIONS);
        assert_eq!(local.size, Some(WIDE_PNG.len() as u64));
        std::fs::remove_file(&original).unwrap();
        let removed = with_original();
        assert_eq!(removed.files.len(), 1);
        assert_eq!(removed.files[0].path, original.to_string_lossy());
        assert!(!removed.files[0].exists);
        assert_eq!((removed.width, removed.height), WIDE_DIMENSIONS);
        assert_eq!(removed.size, Some(WIDE_PNG.len() as u64));

        std::fs::write(&original, WIDE_PNG).unwrap();
        let stored = root.join(&relative);
        let assert_unavailable = || {
            let unavailable = with_original();
            assert_eq!(
                (unavailable.width, unavailable.height, unavailable.size),
                (None, None, None)
            );
        };
        std::fs::write(
            &stored,
            include_bytes!("../../tests/fixtures/clipboard-preview.jpg"),
        )
        .unwrap();
        assert_unavailable();
        std::fs::remove_file(&stored).unwrap();
        assert_unavailable();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&original, &stored).unwrap();
            assert_unavailable();
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_size_requires_a_full_hash_but_not_decodable_dimensions() {
        let root = std::env::temp_dir().join(format!("nami-detail-hash-{}", uuid::Uuid::new_v4()));
        let mut bytes = vec![0; 128 * 1024];
        bytes[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        let relative = clipboard::store_image(&root, &bytes, "png").unwrap();
        let mut value = detail("text", "");
        value.kind = "image".into();
        value.image_path = Some(relative.clone());
        let verified = enrich(value.clone(), &root, false, None).unwrap();
        assert_eq!(
            (verified.width, verified.height, verified.size),
            (None, None, Some(bytes.len() as u64))
        );
        *bytes.last_mut().unwrap() = 1;
        std::fs::write(root.join(relative), bytes).unwrap();
        let corrupt = enrich(value, &root, false, None).unwrap();
        assert_eq!(
            (corrupt.width, corrupt.height, corrupt.size),
            (None, None, None)
        );
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn image_detail_rejects_untrusted_resource_paths_without_serializing_them() {
        let mut value = detail("text", "");
        value.kind = "image".into();
        value.image_path = Some("../outside.png".into());
        let unavailable = enrich(value, Path::new("unused"), true, None).unwrap();
        assert!(
            unavailable.width.is_none()
                && unavailable.height.is_none()
                && unavailable.size.is_none()
        );
        let mut value = detail("text", "");
        value.kind = "image".into();
        value.image_path = Some("private/path".into());
        assert!(serde_json::to_value(value)
            .unwrap()
            .get("imagePath")
            .is_none());
    }
}
