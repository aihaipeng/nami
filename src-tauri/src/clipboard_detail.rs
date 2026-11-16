use crate::platform;
use nami::clipboard::{self, ClipboardDetail};
use std::path::Path;

pub(crate) fn enrich(mut detail: ClipboardDetail, root: &Path) -> Result<ClipboardDetail, String> {
    let image = match detail.kind.as_str() {
        "image" => detail
            .image_path
            .as_deref()
            .and_then(|path| clipboard::read_image(root, path).ok()),
        "files" => {
            detail.files = clipboard::all_file_entries(&detail.text)?;
            detail.file_size = detail.files.iter().try_fold(0u64, |total, entry| {
                let metadata = std::fs::metadata(&entry.path).ok()?;
                metadata.is_file().then_some(())?;
                total.checked_add(metadata.len())
            });
            if detail.files.len() == 1 && detail.files[0].exists && detail.files[0].is_image {
                clipboard::read_file_preview(Path::new(&detail.files[0].path)).ok()
            } else {
                None
            }
        }
        "text" => None,
        _ => return Err("storage_corrupt".into()),
    };
    if let Some((width, height)) = image.and_then(|bytes| platform::image_dimensions(&bytes).ok()) {
        detail.width = Some(width);
        detail.height = Some(height);
    }
    Ok(detail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nami::{clipboard::ClipboardRecord, database::HistoryDatabase};

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
        let result = enrich(detail("text", &text), Path::new("unused")).unwrap();
        assert_eq!(result.text, text);
        assert_eq!(result.character_count, Some(text.chars().count()));
        assert_eq!(result.id, "detail");
        assert_eq!(result.source_app.unwrap()["name"], "Synthetic Application");
        assert!(result.width.is_none() && result.height.is_none() && result.file_size.is_none());
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
        let result = enrich(detail("files", &text), &root).unwrap();
        assert_eq!(result.files.len(), 5);
        assert_eq!(result.file_size, Some(45));
        assert!(result.character_count.is_none() && result.width.is_none());
        std::fs::remove_file(&paths[1]).unwrap();
        let missing = enrich(detail("files", &text), &root).unwrap();
        assert!(!missing.files[1].exists);
        assert!(missing.file_size.is_none());
        let directory = serde_json::to_string(&vec![root.clone()]).unwrap();
        assert!(enrich(detail("files", &directory), &root)
            .unwrap()
            .file_size
            .is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn single_image_file_returns_original_dimensions_and_byte_size() {
        let root = std::env::temp_dir().join(format!("nami-detail-image-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let bytes = include_bytes!("../../tests/fixtures/clipboard-wide.png");
        let path = root.join("synthetic.png");
        std::fs::write(&path, bytes).unwrap();
        let text = serde_json::to_string(&vec![path]).unwrap();
        let result = enrich(detail("files", &text), &root).unwrap();
        assert_eq!(
            result.width,
            Some(u32::from_be_bytes(bytes[16..20].try_into().unwrap()))
        );
        assert_eq!(
            result.height,
            Some(u32::from_be_bytes(bytes[20..24].try_into().unwrap()))
        );
        assert_eq!(result.file_size, Some(bytes.len() as u64));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(all(target_os = "macos", not(feature = "portable-ui")))]
    #[test]
    fn private_image_dimensions_use_verified_attachment_and_reject_changed_bytes() {
        let root =
            std::env::temp_dir().join(format!("nami-detail-private-{}", uuid::Uuid::new_v4()));
        let bytes = include_bytes!("../../tests/fixtures/clipboard-wide.png");
        let relative = clipboard::store_image(&root, bytes, "png").unwrap();
        let make_detail = || {
            let mut value = detail("text", "");
            value.kind = "image".into();
            value.character_count = None;
            value.image_path = Some(relative.clone());
            value
        };
        let result = enrich(make_detail(), &root).unwrap();
        assert!(result.width.is_some() && result.height.is_some());
        assert!(result.file_size.is_none() && result.character_count.is_none());
        std::fs::write(root.join(&relative), b"changed").unwrap();
        let unavailable = enrich(make_detail(), &root).unwrap();
        assert!(unavailable.width.is_none() && unavailable.height.is_none());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn image_detail_rejects_untrusted_resource_paths_without_serializing_them() {
        let mut value = detail("text", "");
        value.kind = "image".into();
        value.image_path = Some("../outside.png".into());
        let unavailable = enrich(value, Path::new("unused")).unwrap();
        assert!(unavailable.width.is_none() && unavailable.height.is_none());
        let mut value = detail("text", "");
        value.kind = "image".into();
        value.image_path = Some("private/path".into());
        assert!(serde_json::to_value(value)
            .unwrap()
            .get("imagePath")
            .is_none());
    }
}
