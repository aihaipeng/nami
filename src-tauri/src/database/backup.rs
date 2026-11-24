use super::{storage_error, HistoryDatabase, APPLICATION_ID, VERSION};
use crate::clipboard;
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

const BACKUP_FORMAT_VERSION: &str = "1";
const MAX_BACKUP_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BackupImportMode {
    Merge,
    Replace,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupSummary {
    pub exported_at: String,
    pub translation_count: usize,
    pub clipboard_count: usize,
    pub image_count: usize,
    pub file_size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupImportReport {
    pub mode: BackupImportMode,
    pub imported_translations: usize,
    pub skipped_translations: usize,
    pub imported_clipboard: usize,
    pub skipped_clipboard: usize,
    pub restored_images: usize,
}

fn backup_error(error: rusqlite::Error) -> String {
    match error.sqlite_error_code() {
        Some(
            rusqlite::ErrorCode::NotADatabase
            | rusqlite::ErrorCode::DatabaseCorrupt
            | rusqlite::ErrorCode::SchemaChanged,
        ) => "backup_invalid".into(),
        _ => storage_error(error),
    }
}

fn open_validated_backup(path: &Path) -> Result<(Connection, u64), String> {
    let meta = std::fs::symlink_metadata(path).map_err(|_| "backup_invalid")?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_BACKUP_FILE_BYTES {
        return Err("backup_invalid".into());
    }
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = Connection::open_with_flags(path, flags).map_err(|_| "backup_invalid")?;
    let app_id: i64 = conn
        .pragma_query_value(None, "application_id", |row| row.get(0))
        .map_err(|_| "backup_invalid")?;
    if app_id != APPLICATION_ID {
        return Err("backup_invalid".into());
    }
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|_| "backup_invalid")?;
    if version > VERSION {
        return Err("backup_unsupported_version".into());
    }
    if version < 10 {
        return Err("backup_invalid".into());
    }
    let quick_check: String = conn
        .pragma_query_value(None, "quick_check", |row| row.get(0))
        .map_err(|_| "backup_invalid")?;
    if quick_check != "ok" {
        return Err("backup_invalid".into());
    }
    let format_version: Option<String> = conn
        .query_row(
            "SELECT value FROM _nami_backup_meta WHERE key = 'format_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| "backup_invalid")?;
    if format_version.as_deref() != Some(BACKUP_FORMAT_VERSION) {
        return Err("backup_invalid".into());
    }
    Ok((conn, meta.len()))
}

pub fn inspect_backup(path: &Path) -> Result<BackupSummary, String> {
    let (conn, file_size) = open_validated_backup(path)?;
    let exported_at: String = conn
        .query_row(
            "SELECT value FROM _nami_backup_meta WHERE key = 'exported_at'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(backup_error)?
        .unwrap_or_default();
    let translation_count: i64 = conn
        .query_row("SELECT count(*) FROM translation_records", [], |row| {
            row.get(0)
        })
        .map_err(backup_error)?;
    let clipboard_count: i64 = conn
        .query_row("SELECT count(*) FROM clipboard_records", [], |row| {
            row.get(0)
        })
        .map_err(backup_error)?;
    let image_count: i64 = conn
        .query_row("SELECT count(*) FROM _nami_backup_images", [], |row| {
            row.get(0)
        })
        .map_err(backup_error)?;
    Ok(BackupSummary {
        exported_at,
        translation_count: translation_count.max(0) as usize,
        clipboard_count: clipboard_count.max(0) as usize,
        image_count: image_count.max(0) as usize,
        file_size,
    })
}

fn collect_referenced_images(conn: &Connection) -> Result<HashSet<String>, String> {
    let mut paths = HashSet::new();
    for sql in [
        "SELECT DISTINCT image_path FROM clipboard_records WHERE image_path IS NOT NULL",
        "SELECT DISTINCT image_path FROM clipboard_file_representations WHERE image_path IS NOT NULL",
    ] {
        let mut stmt = conn.prepare(sql).map_err(storage_error)?;
        let rows = stmt
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        for item in rows {
            paths.insert(item.map_err(storage_error)?);
        }
    }
    Ok(paths)
}

fn resolve_source_app(tx: &Transaction<'_>, payload: &str) -> Result<i64, String> {
    tx.execute(
        "INSERT OR IGNORE INTO source_apps(payload) VALUES(?1)",
        params![payload],
    )
    .map_err(storage_error)?;
    tx.query_row(
        "SELECT id FROM source_apps WHERE payload = ?1",
        params![payload],
        |row| row.get(0),
    )
    .map_err(storage_error)
}

fn load_source_map(conn: &Connection) -> Result<HashMap<i64, String>, String> {
    let mut stmt = conn
        .prepare("SELECT id, payload FROM source_apps")
        .map_err(backup_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(backup_error)?;
    let mut map = HashMap::new();
    for row in rows {
        let (id, payload) = row.map_err(backup_error)?;
        map.insert(id, payload);
    }
    Ok(map)
}

fn validate_and_restore_images(backup: &Connection, image_root: &Path) -> Result<usize, String> {
    let mut stmt = backup
        .prepare("SELECT image_path, sha256, data FROM _nami_backup_images")
        .map_err(backup_error)?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
            ))
        })
        .map_err(backup_error)?;
    let mut restored = 0usize;
    for row in rows {
        let (image_path, sha256, data) = row.map_err(backup_error)?;
        if data.len() > clipboard::MAX_IMAGE_BYTES {
            return Err("backup_invalid".into());
        }
        let extension = if image_path.ends_with(".png") {
            "png"
        } else if image_path.ends_with(".tiff") {
            "tiff"
        } else {
            return Err("backup_invalid".into());
        };
        if clipboard::fingerprint("image", &data) != sha256 {
            return Err("backup_invalid".into());
        }
        let (stored_path, created) =
            clipboard::store_image_with_status(image_root, &data, extension)
                .map_err(|_| "backup_invalid")?;
        if stored_path != image_path {
            return Err("backup_invalid".into());
        }
        if created {
            restored += 1;
        }
    }
    Ok(restored)
}

impl HistoryDatabase {
    pub fn export_backup(
        &self,
        image_root: &Path,
        staging_path: &Path,
    ) -> Result<BackupSummary, String> {
        let staging_str = staging_path.to_str().ok_or("backup_export_failed")?;
        if staging_path.exists() {
            let _ = std::fs::remove_file(staging_path);
        }
        self.connection
            .execute("VACUUM INTO ?1", params![staging_str])
            .map_err(storage_error)?;

        let result = (|| -> Result<BackupSummary, String> {
            let mut backup = Connection::open(staging_path).map_err(storage_error)?;
            backup
                .pragma_update(None, "journal_mode", "DELETE")
                .map_err(storage_error)?;
            backup
                .pragma_update(None, "synchronous", "FULL")
                .map_err(storage_error)?;
            backup
                .pragma_update(None, "foreign_keys", "ON")
                .map_err(storage_error)?;

            let referenced = collect_referenced_images(&backup)?;
            let mut image_blobs: Vec<(String, String, Vec<u8>)> = Vec::new();
            for relative in referenced {
                match clipboard::read_image(image_root, &relative) {
                    Ok(bytes) => {
                        let hash = clipboard::fingerprint("image", &bytes);
                        image_blobs.push((relative, hash, bytes));
                    }
                    Err(code) if code == "clipboard_image_missing" => continue,
                    Err(code) => return Err(code),
                }
            }

            let exported_at = chrono::Utc::now().to_rfc3339();
            let translation_count: i64 = backup
                .query_row("SELECT count(*) FROM translation_records", [], |row| {
                    row.get(0)
                })
                .map_err(storage_error)?;
            let clipboard_count: i64 = backup
                .query_row("SELECT count(*) FROM clipboard_records", [], |row| {
                    row.get(0)
                })
                .map_err(storage_error)?;

            let tx = backup
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            tx.execute_batch(
                "DELETE FROM clipboard_pending_image_deletions;
                CREATE TABLE _nami_backup_meta (
                    key TEXT PRIMARY KEY NOT NULL,
                    value TEXT NOT NULL
                );
                CREATE TABLE _nami_backup_images (
                    image_path TEXT PRIMARY KEY NOT NULL,
                    sha256 TEXT NOT NULL,
                    data BLOB NOT NULL
                );",
            )
            .map_err(storage_error)?;
            for (path, sha256, data) in &image_blobs {
                tx.execute(
                    "INSERT INTO _nami_backup_images(image_path, sha256, data) VALUES(?1, ?2, ?3)",
                    params![path, sha256, data],
                )
                .map_err(storage_error)?;
            }
            let meta_entries = [
                ("format_version", BACKUP_FORMAT_VERSION.to_owned()),
                ("app_version", env!("CARGO_PKG_VERSION").to_owned()),
                ("schema_version", VERSION.to_string()),
                ("exported_at", exported_at.clone()),
                ("translation_count", translation_count.to_string()),
                ("clipboard_count", clipboard_count.to_string()),
                ("image_count", image_blobs.len().to_string()),
            ];
            for (key, value) in meta_entries {
                tx.execute(
                    "INSERT INTO _nami_backup_meta(key, value) VALUES(?1, ?2)",
                    params![key, value],
                )
                .map_err(storage_error)?;
            }
            tx.commit().map_err(storage_error)?;
            drop(backup);

            let file_size = std::fs::metadata(staging_path)
                .map_err(|_| "backup_export_failed")?
                .len();
            Ok(BackupSummary {
                exported_at,
                translation_count: translation_count.max(0) as usize,
                clipboard_count: clipboard_count.max(0) as usize,
                image_count: image_blobs.len(),
                file_size,
            })
        })();

        if result.is_err() {
            let _ = std::fs::remove_file(staging_path);
        }
        result
    }

    pub fn import_backup(
        &mut self,
        backup_path: &Path,
        image_root: &Path,
        mode: BackupImportMode,
    ) -> Result<BackupImportReport, String> {
        let (backup, _) = open_validated_backup(backup_path)?;
        let restored_images = validate_and_restore_images(&backup, image_root)?;
        let source_map = load_source_map(&backup)?;
        let old_images = if mode == BackupImportMode::Replace {
            collect_referenced_images(&self.connection)?
        } else {
            HashSet::new()
        };

        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;

        if mode == BackupImportMode::Replace {
            tx.execute_batch(
                "DELETE FROM translation_records;
                DELETE FROM clipboard_records;
                DELETE FROM source_apps;
                DELETE FROM preferences;",
            )
            .map_err(storage_error)?;
        }

        let mut imported_translations = 0usize;
        let mut skipped_translations = 0usize;
        {
            let mut stmt = backup
                .prepare(
                    "SELECT id, created_at, record_json, engine, is_favorite, is_pinned, source_app_id, source_folded, primary_folded, secondary_folded
                     FROM translation_records ORDER BY sequence ASC",
                )
                .map_err(backup_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, String>(7)?,
                        row.get::<_, String>(8)?,
                        row.get::<_, String>(9)?,
                    ))
                })
                .map_err(backup_error)?;
            for row in rows {
                let (
                    id,
                    created_at,
                    record_json,
                    engine,
                    is_favorite,
                    is_pinned,
                    src_id,
                    source_folded,
                    primary_folded,
                    secondary_folded,
                ) = row.map_err(backup_error)?;
                if mode == BackupImportMode::Merge {
                    let exists: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM translation_records WHERE id = ?1)",
                            params![&id],
                            |r| r.get(0),
                        )
                        .map_err(storage_error)?;
                    if exists {
                        skipped_translations += 1;
                        continue;
                    }
                }
                let mapped_src = match src_id.and_then(|sid| source_map.get(&sid)) {
                    Some(payload) => Some(resolve_source_app(&tx, payload)?),
                    None => None,
                };
                tx.execute(
                    "INSERT INTO translation_records(id, created_at, record_json, engine, is_favorite, is_pinned, source_app_id, source_folded, primary_folded, secondary_folded)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    params![
                        id,
                        created_at,
                        record_json,
                        engine,
                        is_favorite,
                        is_pinned,
                        mapped_src,
                        source_folded,
                        primary_folded,
                        secondary_folded
                    ],
                )
                .map_err(storage_error)?;
                imported_translations += 1;
            }
        }

        let mut imported_clipboard = 0usize;
        let mut skipped_clipboard = 0usize;
        {
            let mut stmt = backup
                .prepare(
                    "SELECT r.sequence, r.id, r.kind, r.summary, r.title, r.date, r.created_at, r.is_favorite, r.is_pinned, r.source_app_id, r.image_path, r.fingerprint,
                            c.text, c.search_folded,
                            f.paths, f.image_path,
                            t.search_folded,
                            o.original_text, o.search_folded,
                            fmt.html, fmt.rtf
                     FROM clipboard_records r
                     JOIN clipboard_contents c ON c.sequence = r.sequence
                     LEFT JOIN clipboard_file_representations f ON f.sequence = r.sequence
                     LEFT JOIN clipboard_titles t ON t.sequence = r.sequence
                     LEFT JOIN clipboard_ocr o ON o.sequence = r.sequence
                     LEFT JOIN clipboard_text_formatting fmt ON fmt.sequence = r.sequence
                     ORDER BY r.sequence ASC",
                )
                .map_err(backup_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, i64>(6)?,
                        row.get::<_, i64>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, Option<String>>(10)?,
                        row.get::<_, String>(11)?,
                        row.get::<_, String>(12)?,
                        row.get::<_, String>(13)?,
                        row.get::<_, Option<String>>(14)?,
                        row.get::<_, Option<String>>(15)?,
                        row.get::<_, Option<String>>(16)?,
                        row.get::<_, Option<String>>(17)?,
                        row.get::<_, Option<String>>(18)?,
                        row.get::<_, Option<Vec<u8>>>(19)?,
                        row.get::<_, Option<Vec<u8>>>(20)?,
                    ))
                })
                .map_err(backup_error)?;
            for row in rows {
                let (
                    id,
                    kind,
                    summary,
                    title,
                    date,
                    created_at,
                    is_favorite,
                    is_pinned,
                    src_id,
                    image_path,
                    fingerprint,
                    text,
                    search_folded,
                    rep_paths,
                    rep_image_path,
                    title_folded,
                    ocr_original,
                    ocr_folded,
                    html,
                    rtf,
                ) = row.map_err(backup_error)?;

                if mode == BackupImportMode::Merge {
                    let dup_by_id: bool = tx
                        .query_row(
                            "SELECT EXISTS(SELECT 1 FROM clipboard_records WHERE id = ?1)",
                            params![&id],
                            |r| r.get(0),
                        )
                        .map_err(storage_error)?;
                    let dup_by_content: bool = if dup_by_id {
                        true
                    } else {
                        tx.query_row(
                            "SELECT EXISTS(
                                SELECT 1 FROM clipboard_records r
                                JOIN clipboard_contents c ON c.sequence = r.sequence
                                WHERE r.kind = ?1 AND r.fingerprint = ?2 AND c.text = ?3
                                  AND COALESCE(r.image_path, '') = COALESCE(?4, '')
                            )",
                            params![&kind, &fingerprint, &text, &image_path],
                            |r| r.get(0),
                        )
                        .map_err(storage_error)?
                    };
                    if dup_by_content {
                        skipped_clipboard += 1;
                        continue;
                    }
                }

                let mapped_src = match src_id.and_then(|sid| source_map.get(&sid)) {
                    Some(payload) => Some(resolve_source_app(&tx, payload)?),
                    None => None,
                };
                tx.execute(
                    "INSERT INTO clipboard_records(id, kind, summary, title, date, created_at, is_favorite, is_pinned, source_app_id, image_path, fingerprint)
                     VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    params![
                        id,
                        kind,
                        summary,
                        title,
                        date,
                        created_at,
                        is_favorite,
                        is_pinned,
                        mapped_src,
                        image_path,
                        fingerprint
                    ],
                )
                .map_err(storage_error)?;
                let sequence = tx.last_insert_rowid();
                tx.execute(
                    "INSERT INTO clipboard_contents(sequence, text, search_folded) VALUES(?1, ?2, ?3)",
                    params![sequence, text, search_folded],
                )
                .map_err(storage_error)?;
                if let (Some(paths), Some(rep_img)) = (rep_paths, rep_image_path) {
                    tx.execute(
                        "INSERT INTO clipboard_file_representations(sequence, paths, image_path) VALUES(?1, ?2, ?3)",
                        params![sequence, paths, rep_img],
                    )
                    .map_err(storage_error)?;
                }
                if let Some(folded) = title_folded {
                    tx.execute(
                        "INSERT INTO clipboard_titles(sequence, search_folded) VALUES(?1, ?2)",
                        params![sequence, folded],
                    )
                    .map_err(storage_error)?;
                }
                if let Some(folded) = ocr_folded {
                    tx.execute(
                        "INSERT INTO clipboard_ocr(sequence, search_folded, original_text) VALUES(?1, ?2, ?3)",
                        params![sequence, folded, ocr_original],
                    )
                    .map_err(storage_error)?;
                }
                if html.is_some() || rtf.is_some() {
                    tx.execute(
                        "INSERT INTO clipboard_text_formatting(sequence, html, rtf) VALUES(?1, ?2, ?3)",
                        params![sequence, html, rtf],
                    )
                    .map_err(storage_error)?;
                }
                imported_clipboard += 1;
            }
        }

        if mode == BackupImportMode::Replace {
            let mut stmt = backup
                .prepare("SELECT key, value FROM preferences")
                .map_err(backup_error)?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(backup_error)?;
            for row in rows {
                let (key, value) = row.map_err(backup_error)?;
                if key == "translation_proxy_url"
                    && crate::translation::proxy_contains_credentials(&value)
                {
                    continue;
                }
                tx.execute(
                    "INSERT OR REPLACE INTO preferences(key, value) VALUES(?1, ?2)",
                    params![key, value],
                )
                .map_err(storage_error)?;
            }

            // Enqueue any previously referenced images that are no longer referenced after Replace.
            for old_path in old_images {
                let still_used: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM clipboard_records WHERE image_path = ?1)
                            OR EXISTS(SELECT 1 FROM clipboard_file_representations WHERE image_path = ?1)",
                        params![&old_path],
                        |r| r.get(0),
                    )
                    .map_err(storage_error)?;
                if !still_used {
                    tx.execute(
                        "INSERT OR IGNORE INTO clipboard_pending_image_deletions(image_path) VALUES(?1)",
                        params![old_path],
                    )
                    .map_err(storage_error)?;
                }
            }
        }

        tx.commit().map_err(storage_error)?;

        let total_records: i64 = self
            .connection
            .query_row("SELECT count(*) FROM translation_records", [], |r| r.get(0))
            .map_err(storage_error)?;
        let clipboard_total: i64 = self
            .connection
            .query_row("SELECT count(*) FROM clipboard_records", [], |r| r.get(0))
            .map_err(storage_error)?;
        self.total_records = total_records.max(0) as usize;
        self.clipboard_total_records = clipboard_total.max(0) as usize;

        Ok(BackupImportReport {
            mode,
            imported_translations,
            skipped_translations,
            imported_clipboard,
            skipped_clipboard,
            restored_images,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::HistoryQuery;
    use crate::model::HistoryRecord;

    fn sample_png() -> Vec<u8> {
        // Minimal valid 1x1 PNG signature + chunks.
        vec![
            0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, b'I', b'H',
            b'D', b'R', 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x02, 0x00, 0x00,
            0x00, 0x90, 0x77, 0x53, 0xDE, 0x00, 0x00, 0x00, 0x0C, b'I', b'D', b'A', b'T', 0x08,
            0xD7, 0x63, 0xF8, 0xFF, 0xFF, 0x3F, 0x00, 0x05, 0xFE, 0x02, 0xFE, 0xDC, 0xCC, 0x59,
            0xE7, 0x00, 0x00, 0x00, 0x00, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82,
        ]
    }

    fn translation(id: &str, text: &str) -> HistoryRecord {
        HistoryRecord {
            title: String::new(),
            id: id.into(),
            text: text.into(),
            source_language: "en".into(),
            translation: None,
            target_languages: vec!["zh-CN".into()],
            translations: vec![format!("译:{text}")],
            date: "2026-11-24T10:00:00Z".into(),
            is_favorite: true,
            is_pinned: false,
            source_app: Some(serde_json::json!({"name": "Notes", "bundleId": "com.apple.Notes"})),
        }
    }

    fn clip_text(id: &str, text: &str) -> clipboard::ClipboardRecord {
        clipboard::ClipboardRecord {
            id: id.into(),
            kind: "text".into(),
            text: text.into(),
            date: "2026-11-24T10:01:00Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: Some(serde_json::json!({"name": "Notes", "bundleId": "com.apple.Notes"})),
            image_path: None,
            fingerprint: clipboard::fingerprint("text", text.as_bytes()),
        }
    }

    fn clip_image(id: &str, rel_img: &str, png: &[u8]) -> clipboard::ClipboardRecord {
        clipboard::ClipboardRecord {
            id: id.into(),
            kind: "image".into(),
            text: String::new(),
            date: "2026-11-24T10:02:00Z".into(),
            is_favorite: true,
            is_pinned: true,
            source_app: None,
            image_path: Some(rel_img.into()),
            fingerprint: clipboard::fingerprint("image", png),
        }
    }

    #[test]
    fn backup_export_merge_replace_and_corruption_guards_work() {
        let root = std::env::temp_dir().join(format!("nami-backup-test-{}", uuid::Uuid::new_v4()));
        let src_images = root.join("src_images");
        let dst_images = root.join("dst_images");
        std::fs::create_dir_all(&src_images).unwrap();
        std::fs::create_dir_all(&dst_images).unwrap();

        let src_db_path = root.join("src.sqlite3");
        let mut src = HistoryDatabase::open(&src_db_path).unwrap();
        src.insert(&translation("tr-1", "hello backup"), "edge", || true)
            .unwrap();
        src.clipboard_insert(&clip_text("cb-1", "clip text one"))
            .unwrap();
        let png = sample_png();
        let rel_img = clipboard::store_image(&src_images, &png, "png").unwrap();
        src.clipboard_insert(&clip_image("cb-2", &rel_img, &png))
            .unwrap();
        src.preference_set("ui_locale", "zh-CN").unwrap();
        src.preference_set("window_position_mode", "cursor")
            .unwrap();

        let backup_path = root.join("export.namibak");
        let summary = src.export_backup(&src_images, &backup_path).unwrap();
        assert_eq!(summary.translation_count, 1);
        assert_eq!(summary.clipboard_count, 2);
        assert_eq!(summary.image_count, 1);
        assert!(summary.file_size > 0);

        let inspected = inspect_backup(&backup_path).unwrap();
        assert_eq!(inspected.translation_count, 1);
        assert_eq!(inspected.clipboard_count, 2);
        assert_eq!(inspected.image_count, 1);

        // Merge into a destination DB with one overlapping text and different locale.
        let dst_db_path = root.join("dst.sqlite3");
        let mut dst = HistoryDatabase::open(&dst_db_path).unwrap();
        dst.clipboard_insert(&clip_text("cb-local-dup", "clip text one"))
            .unwrap();
        dst.clipboard_insert(&clip_text("cb-local-only", "existing local only"))
            .unwrap();
        dst.preference_set("ui_locale", "en").unwrap();

        let merge = dst
            .import_backup(&backup_path, &dst_images, BackupImportMode::Merge)
            .unwrap();
        assert_eq!(merge.imported_translations, 1);
        assert_eq!(merge.skipped_translations, 0);
        assert_eq!(merge.imported_clipboard, 1);
        assert_eq!(merge.skipped_clipboard, 1);
        assert_eq!(merge.restored_images, 1);
        // Merge preserves local preferences.
        assert_eq!(dst.preference("ui_locale").unwrap().as_deref(), Some("en"));
        // Restored image is readable on disk with valid hash.
        assert_eq!(clipboard::read_image(&dst_images, &rel_img).unwrap(), png);
        // Translation FTS search works on imported record.
        let found = dst
            .page(
                &HistoryQuery {
                    text: "backup".into(),
                    favorites_only: false,
                },
                None,
                10,
            )
            .unwrap();
        assert_eq!(found.records.len(), 1);
        assert_eq!(found.records[0].id, "tr-1");

        // Replace overwrites records and preferences cleanly.
        let replace = dst
            .import_backup(&backup_path, &dst_images, BackupImportMode::Replace)
            .unwrap();
        assert_eq!(replace.imported_translations, 1);
        assert_eq!(replace.imported_clipboard, 2);
        assert_eq!(
            dst.preference("ui_locale").unwrap().as_deref(),
            Some("zh-CN")
        );
        assert_eq!(
            dst.preference("window_position_mode").unwrap().as_deref(),
            Some("cursor")
        );

        // Invalid / garbage file is rejected without touching destination DB.
        let bad_path = root.join("bad.namibak");
        std::fs::write(&bad_path, b"not a sqlite file").unwrap();
        assert_eq!(inspect_backup(&bad_path).unwrap_err(), "backup_invalid");
        assert_eq!(
            dst.import_backup(&bad_path, &dst_images, BackupImportMode::Merge)
                .unwrap_err(),
            "backup_invalid"
        );

        // Tampered image path (path traversal) is rejected before mutating destination DB.
        let tampered_path = root.join("tampered.namibak");
        std::fs::copy(&backup_path, &tampered_path).unwrap();
        {
            let conn = Connection::open(&tampered_path).unwrap();
            conn.execute(
                "UPDATE _nami_backup_images SET image_path = '../../escape.png'",
                [],
            )
            .unwrap();
        }
        assert_eq!(
            dst.import_backup(&tampered_path, &dst_images, BackupImportMode::Merge)
                .unwrap_err(),
            "backup_invalid"
        );

        drop(src);
        drop(dst);
        let _ = std::fs::remove_dir_all(root);
    }
}
