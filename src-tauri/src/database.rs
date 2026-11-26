mod backup;
mod clipboard;
mod retention;

pub use backup::{
    decrypt_backup_file, encrypt_backup_file, inspect_backup, inspect_backup_with_password,
    is_encrypted_backup, BackupImportMode, BackupImportReport, BackupSummary, TempFileGuard,
    MAX_BACKUP_PASSWORD_BYTES,
};
pub use clipboard::unlink_pending_clipboard_images;

use rusqlite::{Connection, ErrorCode, TransactionBehavior};
use std::{path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4E414D49;
const VERSION: i64 = 12;
pub const PAGE_SIZE: usize = 100;

pub struct HistoryDatabase {
    connection: Connection,
    clipboard_total_records: usize,
}

fn storage_error(error: rusqlite::Error) -> String {
    match error.sqlite_error_code() {
        Some(ErrorCode::DatabaseBusy | ErrorCode::DatabaseLocked) => "storage_busy",
        Some(ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase) => "storage_corrupt",
        _ => "storage_unavailable",
    }
    .into()
}

fn migrate_v11(connection: &mut Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch(
        "DROP TRIGGER IF EXISTS history_insert;
        DROP TRIGGER IF EXISTS history_delete;
        DROP TABLE IF EXISTS translation_search;
        DROP TABLE IF EXISTS translation_records;
        DELETE FROM preferences WHERE key IN (
            'translation_history_limit',
            'translation_history_policy',
            'translation_preview_lines'
        );
        DELETE FROM source_apps WHERE id NOT IN (
            SELECT DISTINCT source_app_id FROM clipboard_records WHERE source_app_id IS NOT NULL
        );
        PRAGMA user_version=11;",
    )
    .map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

fn migrate_v12(connection: &mut Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    if !clipboard::has_column(&tx, "clipboard_records", "pin_paste_key")? {
        tx.execute(
            "ALTER TABLE clipboard_records ADD COLUMN pin_paste_key TEXT",
            [],
        )
        .map_err(storage_error)?;
    }
    tx.execute(
        "CREATE UNIQUE INDEX IF NOT EXISTS clipboard_pin_paste_key ON clipboard_records(pin_paste_key) WHERE pin_paste_key IS NOT NULL",
        [],
    )
    .map_err(storage_error)?;
    tx.pragma_update(None, "user_version", 12)
        .map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

impl HistoryDatabase {
    pub fn open(path: &Path) -> Result<Self, String> {
        let mut connection = Connection::open(path).map_err(storage_error)?;
        connection
            .busy_timeout(Duration::from_secs(2))
            .map_err(storage_error)?;
        let version: i64 = connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(storage_error)?;
        let application: i64 = connection
            .pragma_query_value(None, "application_id", |row| row.get(0))
            .map_err(storage_error)?;
        if version > VERSION {
            return Err("storage_version".into());
        }
        if version == 0 {
            let tables: i64 = connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name NOT LIKE 'sqlite_%'",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            if tables != 0 || application != 0 {
                return Err("storage_schema".into());
            }
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            transaction
                .execute_batch(
                    "CREATE TABLE source_apps (id INTEGER PRIMARY KEY, payload TEXT NOT NULL UNIQUE);",
                )
                .map_err(storage_error)?;
            transaction
                .pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(storage_error)?;
            transaction
                .pragma_update(None, "user_version", 1)
                .map_err(storage_error)?;
            transaction.commit().map_err(storage_error)?;
        } else if application != APPLICATION_ID {
            return Err("storage_schema".into());
        } else {
            let has_source_apps: bool = connection
                .query_row(
                    "SELECT count(*) > 0 FROM sqlite_master WHERE type = 'table' AND name = 'source_apps'",
                    [],
                    |row| row.get(0),
                )
                .map_err(storage_error)?;
            if !has_source_apps {
                return Err("storage_schema".into());
            }
        }
        if version < 2 {
            clipboard::migrate(&mut connection)?;
        }
        if version < 3 {
            clipboard::migrate_v3(&mut connection)?;
        }
        if version < 4 {
            let tx = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(storage_error)?;
            tx.execute_batch("ALTER TABLE clipboard_records ADD COLUMN title TEXT NOT NULL DEFAULT ''; PRAGMA user_version=4;")
                .map_err(storage_error)?;
            tx.commit().map_err(storage_error)?;
        }
        if version < 5 {
            clipboard::migrate_v5(&mut connection)?;
        }
        if version < 6 {
            clipboard::migrate_v6(&mut connection)?;
        }
        if version < 7 {
            clipboard::migrate_v7(&mut connection)?;
        }
        if version < 8 {
            clipboard::migrate_v8(&mut connection)?;
        }
        if version < 9 {
            clipboard::migrate_v9(&mut connection)?;
        }
        if version < 10 {
            clipboard::migrate_v10(&mut connection)?;
        }
        if version < 11 {
            migrate_v11(&mut connection)?;
        }
        if version < 12 {
            migrate_v12(&mut connection)?;
        }
        clipboard::ensure_filter_indexes(&connection)?;
        clipboard::ensure_translation_preferences(&connection)?;
        clipboard::sync_pin_paste_keys(&connection)?;
        let clipboard_total_records = connection
            .query_row("SELECT count(*) FROM clipboard_records", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(storage_error)? as usize;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(storage_error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(storage_error)?;
        connection
            .pragma_update(None, "foreign_keys", true)
            .map_err(storage_error)?;
        Ok(Self {
            connection,
            clipboard_total_records,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture(std::path::PathBuf);
    impl Fixture {
        fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("nami-db-test-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir(&directory).unwrap();
            Self(directory.join("history.sqlite3"))
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(self.0.parent().unwrap());
        }
    }

    #[test]
    fn migration_v10_to_v11_drops_translation_tables_and_preserves_clipboard_and_preferences() {
        let fixture = Fixture::new();
        {
            let mut db = HistoryDatabase::open(&fixture.0).unwrap();
            db.clipboard_insert(&crate::clipboard::ClipboardRecord {
                id: "clip-1".into(),
                kind: "text".into(),
                text: "Keep this clipboard item".into(),
                date: "2026-11-24T12:00:00Z".into(),
                is_favorite: true,
                is_pinned: false,
                source_app: Some(
                    serde_json::json!({"name": "Notes", "bundleId": "com.apple.Notes"}),
                ),
                image_path: None,
                fingerprint: crate::clipboard::fingerprint("text", b"Keep this clipboard item"),
            })
            .unwrap();
            db.preference_set("ui_locale", "zh-CN").unwrap();
            // Simulate legacy v10 schema with translation_records & translation preferences.
            db.connection
                .execute_batch(
                    "CREATE TABLE translation_records (
                        sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL,
                        record_json TEXT NOT NULL, engine TEXT NOT NULL, is_favorite INTEGER NOT NULL, is_pinned INTEGER NOT NULL,
                        source_app_id INTEGER REFERENCES source_apps(id),
                        source_folded TEXT NOT NULL, primary_folded TEXT NOT NULL, secondary_folded TEXT NOT NULL
                    );
                    CREATE VIRTUAL TABLE translation_search USING fts5(source_folded, primary_folded, secondary_folded, content='translation_records', content_rowid='sequence');
                    INSERT INTO source_apps(payload) VALUES('{\"name\":\"TranslationOnly\"}');
                    INSERT INTO translation_records(id, created_at, record_json, engine, is_favorite, is_pinned, source_app_id, source_folded, primary_folded, secondary_folded)
                        VALUES('tr-1', 1, '{}', 'edge', 0, 0, last_insert_rowid(), 'hi', '你好', '');
                    INSERT OR REPLACE INTO preferences(key, value) VALUES('translation_history_policy', '{\"limit\":500}');
                    INSERT OR REPLACE INTO preferences(key, value) VALUES('translation_preview_lines', '2');
                    PRAGMA user_version = 10;",
                )
                .unwrap();
        }

        let db = HistoryDatabase::open(&fixture.0).unwrap();
        let version: i64 = db
            .connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 12);
        for dropped in ["translation_records", "translation_search"] {
            let exists: bool = db
                .connection
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?1)",
                    [dropped],
                    |row| row.get(0),
                )
                .unwrap();
            assert!(!exists, "{dropped} must be dropped in v11");
        }
        assert_eq!(db.preference("translation_history_policy").unwrap(), None);
        assert_eq!(db.preference("translation_preview_lines").unwrap(), None);
        assert_eq!(
            db.preference("ui_locale").unwrap().as_deref(),
            Some("zh-CN")
        );
        let clip = db.clipboard_record("clip-1").unwrap();
        assert_eq!(clip.text, "Keep this clipboard item");
        assert!(clip.is_favorite);
    }

    #[test]
    fn future_schema_version_is_rejected() {
        let fixture = Fixture::new();
        {
            let db = HistoryDatabase::open(&fixture.0).unwrap();
            db.connection
                .pragma_update(None, "user_version", VERSION + 1)
                .unwrap();
        }
        assert_eq!(
            HistoryDatabase::open(&fixture.0).err().as_deref(),
            Some("storage_version")
        );
    }
}
