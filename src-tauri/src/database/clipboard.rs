use super::{storage_error, HistoryDatabase};
use crate::clipboard::{
    ClipboardCursor, ClipboardPage, ClipboardRecord, MAX_FILE_COUNT, MAX_TEXT_BYTES,
};
use rusqlite::{params, OptionalExtension, Row, TransactionBehavior};

pub(super) fn migrate(connection: &mut rusqlite::Connection) -> Result<(), String> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    transaction.execute_batch("CREATE TABLE clipboard_records (
        sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
        kind TEXT NOT NULL CHECK(kind IN ('text','image','files')),
        text TEXT NOT NULL, summary TEXT NOT NULL, search_folded TEXT NOT NULL, date TEXT NOT NULL,
        is_favorite INTEGER NOT NULL CHECK(is_favorite IN (0,1)),
        is_pinned INTEGER NOT NULL CHECK(is_pinned IN (0,1)),
        source_app TEXT, image_path TEXT, fingerprint TEXT NOT NULL
    );
    CREATE INDEX clipboard_order ON clipboard_records(is_pinned DESC,date DESC,sequence DESC);
    CREATE INDEX clipboard_favorites ON clipboard_records(is_favorite,is_pinned DESC,date DESC,sequence DESC);
    CREATE INDEX clipboard_fingerprint ON clipboard_records(fingerprint);
    CREATE VIRTUAL TABLE clipboard_search USING fts5(search_folded,content='clipboard_records',content_rowid='sequence',tokenize='trigram case_sensitive 1');
    CREATE TRIGGER clipboard_insert AFTER INSERT ON clipboard_records BEGIN
        INSERT INTO clipboard_search(rowid,search_folded) VALUES(new.sequence,new.search_folded);
    END;
    CREATE TRIGGER clipboard_delete AFTER DELETE ON clipboard_records BEGIN
        INSERT INTO clipboard_search(clipboard_search,rowid,search_folded) VALUES('delete',old.sequence,old.search_folded);
    END;
    CREATE TABLE preferences(key TEXT PRIMARY KEY,value TEXT NOT NULL);
    INSERT INTO preferences(key,value) VALUES('clipboard_capture','false');
    PRAGMA user_version=2;")
    .map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

fn decode(row: &Row<'_>) -> rusqlite::Result<ClipboardRecord> {
    let source: Option<String> = row.get(7)?;
    let source_app = source
        .map(|source| serde_json::from_str(&source))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                7,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(ClipboardRecord {
        id: row.get(1)?,
        kind: row.get(2)?,
        text: row.get(3)?,
        date: row.get(4)?,
        is_favorite: row.get(5)?,
        is_pinned: row.get(6)?,
        source_app,
        image_path: row.get(8)?,
        fingerprint: row.get(9)?,
    })
}

const COLUMNS: &str =
    "sequence,id,kind,text,date,is_favorite,is_pinned,source_app,image_path,fingerprint";

impl HistoryDatabase {
    pub fn clipboard_page(
        &self,
        query: &str,
        favorites_only: bool,
        cursor: Option<&ClipboardCursor>,
        limit: usize,
    ) -> Result<ClipboardPage, String> {
        if query.chars().count() > 4096 || !(1..=200).contains(&limit) {
            return Err("invalid_argument".into());
        }
        let needle = query.trim().to_lowercase();
        let indexed = needle.chars().count() >= 3 && !needle.contains('\0');
        let search = if needle.is_empty() {
            ""
        } else if indexed {
            " AND sequence IN (SELECT rowid FROM clipboard_search WHERE clipboard_search MATCH :match) AND instr(search_folded,:needle)>0"
        } else {
            " AND instr(search_folded,:needle)>0"
        };
        let favorite = if favorites_only {
            " AND is_favorite=1"
        } else {
            ""
        };
        let columns = COLUMNS.replace("kind,text,date", "kind,summary,date");
        let sql = format!("SELECT {columns} FROM clipboard_records WHERE (is_pinned,date,sequence)<(:pinned,:date,:sequence){favorite}{search} ORDER BY is_pinned DESC,date DESC,sequence DESC LIMIT :limit");
        let mut statement = self
            .connection
            .prepare_cached(&sql)
            .map_err(storage_error)?;
        let pinned = cursor.map_or(2i64, |cursor| i64::from(cursor.pinned));
        let date = cursor.map_or("~", |cursor| cursor.date.as_str());
        let sequence = cursor.map_or(i64::MAX, |cursor| cursor.sequence);
        let take = limit as i64 + 1;
        let matched = format!("\"{}\"", needle.replace('"', "\"\""));
        let mut bindings: Vec<(&str, &dyn rusqlite::ToSql)> = vec![
            (":pinned", &pinned),
            (":date", &date),
            (":sequence", &sequence),
            (":limit", &take),
        ];
        if !needle.is_empty() {
            bindings.push((":needle", &needle));
            if indexed {
                bindings.push((":match", &matched));
            }
        }
        let mut rows = statement
            .query(bindings.as_slice())
            .map_err(storage_error)?;
        let mut records = Vec::with_capacity(limit);
        let mut cursor = None;
        let mut has_more = false;
        while let Some(row) = rows.next().map_err(storage_error)? {
            if records.len() == limit {
                has_more = true;
                break;
            }
            let record = decode(row).map_err(storage_error)?;
            cursor = Some(ClipboardCursor {
                pinned: record.is_pinned,
                date: record.date.clone(),
                sequence: row.get(0).map_err(storage_error)?,
            });
            records.push(record);
        }
        let total_records = self
            .connection
            .query_row("SELECT count(*) FROM clipboard_records", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(storage_error)? as usize;
        Ok(ClipboardPage {
            records,
            cursor,
            has_more,
            total_records,
        })
    }

    pub fn clipboard_record(&self, id: &str) -> Result<ClipboardRecord, String> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM clipboard_records WHERE id=?1"),
                [id],
                decode,
            )
            .optional()
            .map_err(storage_error)?
            .ok_or_else(|| "not_found".into())
    }

    pub fn clipboard_insert(
        &mut self,
        record: &ClipboardRecord,
    ) -> Result<ClipboardRecord, String> {
        if record.id.is_empty()
            || !["text", "image", "files"].contains(&record.kind.as_str())
            || record.text.len() > MAX_TEXT_BYTES
            || record.fingerprint.is_empty()
            || chrono::DateTime::parse_from_rfc3339(&record.date).is_err()
            || (record.kind == "image") != record.image_path.is_some()
        {
            return Err("invalid_argument".into());
        }
        let (summary, search_text) = if record.kind == "files" {
            let paths: Vec<String> =
                serde_json::from_str(&record.text).map_err(|_| "invalid_argument")?;
            if paths.is_empty()
                || paths.len() > MAX_FILE_COUNT
                || paths
                    .iter()
                    .any(|path| !std::path::Path::new(path).is_absolute())
            {
                return Err("invalid_argument".into());
            }
            let summary = paths
                .iter()
                .map(|path| {
                    std::path::Path::new(path)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or(path)
                })
                .collect::<Vec<_>>()
                .join("\n");
            (summary, paths.join("\n"))
        } else {
            (record.text.clone(), record.text.clone())
        };
        let summary: String = summary.chars().take(256).collect();
        let source = record
            .source_app
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| "invalid_argument")?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let duplicate: Option<String> = transaction.query_row("SELECT id FROM clipboard_records WHERE fingerprint=?1 AND kind=?2 AND ((kind='image' AND image_path IS ?4) OR (kind!='image' AND text=?3)) LIMIT 1", params![record.fingerprint,record.kind,record.text,record.image_path], |row| row.get(0)).optional().map_err(storage_error)?;
        let id = if let Some(id) = duplicate {
            transaction
                .execute(
                    "UPDATE clipboard_records SET date=?1,source_app=?2 WHERE id=?3",
                    params![record.date, source, id],
                )
                .map_err(storage_error)?;
            id
        } else {
            transaction.execute("INSERT INTO clipboard_records(id,kind,text,summary,search_folded,date,is_favorite,is_pinned,source_app,image_path,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",params![record.id,record.kind,record.text,summary,search_text.to_lowercase(),record.date,record.is_favorite,record.is_pinned,source,record.image_path,record.fingerprint]).map_err(storage_error)?;
            record.id.clone()
        };
        transaction.commit().map_err(storage_error)?;
        self.clipboard_record(&id)
    }

    pub fn clipboard_update(&mut self, id: &str, action: &str) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let (favorite, pinned): (bool, bool) = transaction
            .query_row(
                "SELECT is_favorite,is_pinned FROM clipboard_records WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage_error)?
            .ok_or("not_found")?;
        let sql = match action {
            "favorite" => "UPDATE clipboard_records SET is_favorite=NOT is_favorite WHERE id=?1",
            "pin" => "UPDATE clipboard_records SET is_pinned=NOT is_pinned WHERE id=?1",
            "delete" if favorite || pinned => return Err("clipboard_protected".into()),
            "delete" => "DELETE FROM clipboard_records WHERE id=?1",
            _ => return Err("invalid_argument".into()),
        };
        transaction.execute(sql, [id]).map_err(storage_error)?;
        transaction.commit().map_err(storage_error)
    }

    pub fn clipboard_capture_enabled(&self) -> Result<bool, String> {
        let value: String = self
            .connection
            .query_row(
                "SELECT value FROM preferences WHERE key='clipboard_capture'",
                [],
                |row| row.get(0),
            )
            .map_err(storage_error)?;
        match value.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err("storage_corrupt".into()),
        }
    }

    pub fn clipboard_capture_set(&mut self, enabled: bool) -> Result<(), String> {
        self.connection
            .execute(
                "UPDATE preferences SET value=?1 WHERE key='clipboard_capture'",
                [if enabled { "true" } else { "false" }],
            )
            .map_err(storage_error)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clipboard::fingerprint;

    fn record(id: &str, text: &str) -> ClipboardRecord {
        ClipboardRecord {
            id: id.into(),
            kind: "text".into(),
            text: text.into(),
            date: "2026-11-14T12:00:00.000Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
            image_path: None,
            fingerprint: fingerprint("text", text.as_bytes()),
        }
    }

    #[test]
    fn file_count_limit_matches_capture_and_restore() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let paths: Vec<_> = (0..MAX_FILE_COUNT)
            .map(|index| format!("/synthetic/{index}.txt"))
            .collect();
        let mut item = record("files-limit", &serde_json::to_string(&paths).unwrap());
        item.kind = "files".into();
        database.clipboard_insert(&item).unwrap();
        assert_eq!(database.clipboard_record(&item.id).unwrap().text, item.text);
        let mut oversized = paths;
        oversized.push("/synthetic/one-more.txt".into());
        item.id = "too-many-files".into();
        item.text = serde_json::to_string(&oversized).unwrap();
        assert_eq!(
            database.clipboard_insert(&item).unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            database.clipboard_record(&item.id).unwrap_err(),
            "not_found"
        );
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            1
        );
    }

    #[test]
    fn clipboard_persistence_deduplication_and_protection() {
        let directory =
            std::env::temp_dir().join(format!("nami-clipboard-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");
        let mut database = HistoryDatabase::open(&path).unwrap();
        assert!(!database.clipboard_capture_enabled().unwrap());
        assert!(database
            .clipboard_page("", false, None, 100)
            .unwrap()
            .records
            .is_empty());
        let item = record("first", "hello\n世界");
        database.clipboard_insert(&item).unwrap();
        database.clipboard_update("first", "favorite").unwrap();
        let mut duplicate = record("second", "hello\n世界");
        duplicate.date = "2026-11-14T13:00:00.000Z".into();
        let saved = database.clipboard_insert(&duplicate).unwrap();
        assert_eq!(saved.id, "first");
        assert!(saved.is_favorite);
        assert_eq!(saved.date, duplicate.date);
        assert_eq!(
            database.clipboard_update("first", "delete").unwrap_err(),
            "clipboard_protected"
        );
        database.clipboard_capture_set(true).unwrap();
        drop(database);
        let mut database = HistoryDatabase::open(&path).unwrap();
        assert!(database.clipboard_capture_enabled().unwrap());
        assert_eq!(database.clipboard_record("first").unwrap().text, item.text);
        database.clipboard_update("first", "favorite").unwrap();
        database.clipboard_update("first", "pin").unwrap();
        assert!(database.clipboard_update("first", "delete").is_err());
        database.clipboard_update("first", "pin").unwrap();
        database.clipboard_update("first", "delete").unwrap();
        assert_eq!(database.clipboard_record("first").unwrap_err(), "not_found");
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn clipboard_pagination_search_and_complete_details() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for index in 0..225 {
            database
                .clipboard_insert(&record(
                    &format!("item-{index}"),
                    &format!("ordinary {index}"),
                ))
                .unwrap();
        }
        let mut item = record(
            "special",
            &("x".repeat(300) + " needle 中文 100%_literal a\"b"),
        );
        item.is_favorite = true;
        item.is_pinned = true;
        database.clipboard_insert(&item).unwrap();
        let first = database.clipboard_page("", false, None, 100).unwrap();
        assert_eq!(first.records[0].id, "special");
        assert_eq!(first.records[0].text.chars().count(), 256);
        assert_eq!(
            database.clipboard_record("special").unwrap().text,
            item.text
        );
        let mut ids = std::collections::HashSet::new();
        let mut cursor = None;
        loop {
            let page = database
                .clipboard_page("", false, cursor.as_ref(), 100)
                .unwrap();
            for item in page.records {
                assert!(ids.insert(item.id));
            }
            cursor = page.cursor;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(ids.len(), 226);
        for query in ["needle", "中文", "100%_literal", "a\"b"] {
            let page = database.clipboard_page(query, false, None, 100).unwrap();
            assert_eq!(page.records.len(), 1);
            assert_eq!(page.records[0].id, "special");
        }
        assert_eq!(
            database
                .clipboard_page("", true, None, 100)
                .unwrap()
                .records
                .len(),
            1
        );
        assert!(database.clipboard_page("", false, None, 201).is_err());
    }

    #[test]
    fn clipboard_kinds_keep_original_content_and_do_not_merge_hash_collisions() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let mut files = record(
            "files",
            &serde_json::to_string(&["/synthetic/first.txt", "/synthetic/second.png"]).unwrap(),
        );
        files.kind = "files".into();
        files.fingerprint = fingerprint("files", files.text.as_bytes());
        database.clipboard_insert(&files).unwrap();
        assert_eq!(database.clipboard_record("files").unwrap().text, files.text);
        let mut collision = record(
            "collision",
            &serde_json::to_string(&["/synthetic/different.txt"]).unwrap(),
        );
        collision.fingerprint = files.fingerprint.clone();
        collision.kind = "files".into();
        database.clipboard_insert(&collision).unwrap();
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            2
        );
        let mut image = record("image", "Image");
        image.kind = "image".into();
        image.image_path = Some("origin/ab/abcdef.png".into());
        image.fingerprint = fingerprint("image", b"synthetic image");
        database.clipboard_insert(&image).unwrap();
        let mut duplicate = image.clone();
        duplicate.id = "second-image".into();
        duplicate.text = "Other title".into();
        assert_eq!(database.clipboard_insert(&duplicate).unwrap().id, "image");
        assert_eq!(
            database.clipboard_record("image").unwrap().image_path,
            image.image_path
        );
    }

    #[test]
    fn files_summary_uses_names_while_details_keep_complete_json_paths() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let paths = [
            "/synthetic/资料/第一行\n第二行.txt",
            "/synthetic/世界🌊.png",
        ];
        let mut files = record("files", &serde_json::to_string(&paths).unwrap());
        files.kind = "files".into();
        files.fingerprint = fingerprint("files", files.text.as_bytes());
        database.clipboard_insert(&files).unwrap();
        let page = database.clipboard_page("", false, None, 100).unwrap();
        assert_eq!(page.records[0].text, "第一行\n第二行.txt\n世界🌊.png");
        assert_eq!(database.clipboard_record("files").unwrap().text, files.text);
        assert_eq!(
            database
                .clipboard_page("资料", false, None, 100)
                .unwrap()
                .records
                .len(),
            1
        );
        let mut malformed = files.clone();
        malformed.id = "malformed".into();
        malformed.text = "not JSON".into();
        assert_eq!(
            database.clipboard_insert(&malformed).unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            1
        );
    }

    #[test]
    fn files_search_matches_decoded_names_with_json_escape_characters() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let paths = [
            "/synthetic/hello\"world.txt",
            "/synthetic/hello\\world.txt",
            "/synthetic/hello\nworld.txt",
        ];
        let mut files = record("escaped-files", &serde_json::to_string(&paths).unwrap());
        files.kind = "files".into();
        files.fingerprint = fingerprint("files", files.text.as_bytes());
        database.clipboard_insert(&files).unwrap();
        for name in ["hello\"world.txt", "hello\\world.txt", "hello\nworld.txt"] {
            let page = database.clipboard_page(name, false, None, 100).unwrap();
            assert_eq!(page.records.len(), 1, "display name={name:?}");
            assert_eq!(page.records[0].id, "escaped-files");
        }
        assert_eq!(
            database.clipboard_record("escaped-files").unwrap().text,
            files.text
        );
    }

    #[test]
    fn broken_version_one_database_is_not_migrated_or_replaced() {
        let directory =
            std::env::temp_dir().join(format!("nami-broken-upgrade-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection.execute_batch("CREATE TABLE preserved(value TEXT); INSERT INTO preserved VALUES('synthetic'); PRAGMA application_id=1312902473; PRAGMA user_version=1;").unwrap();
        drop(connection);
        assert!(HistoryDatabase::open(&path).is_err());
        let connection = rusqlite::Connection::open(&path).unwrap();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            connection
                .query_row("SELECT value FROM preserved", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "synthetic"
        );
        assert_eq!(
            connection
                .query_row(
                    "SELECT count(*) FROM sqlite_master WHERE name='clipboard_records'",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        drop(connection);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn migration_preserves_version_one_translation_records() {
        let directory = std::env::temp_dir().join(format!("nami-upgrade-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");
        let mut database = HistoryDatabase::open(&path).unwrap();
        let translation = crate::model::HistoryRecord {
            id: "preserved-translation".into(),
            text: "synthetic original".into(),
            source_language: "auto".into(),
            target_languages: vec!["en".into()],
            translations: vec!["synthetic translation".into()],
            date: "2026-11-14T12:00:00.000Z".into(),
            is_favorite: true,
            is_pinned: false,
            source_app: None,
        };
        database.insert(&translation, "edge", || true).unwrap();
        database.connection.execute_batch("DROP TRIGGER clipboard_insert; DROP TRIGGER clipboard_delete; DROP TABLE clipboard_search; DROP TABLE clipboard_records; DROP TABLE preferences; PRAGMA user_version=1;").unwrap();
        database
            .connection
            .execute(
                "INSERT INTO source_apps(payload) VALUES(?1)",
                ["synthetic preserved source"],
            )
            .unwrap();
        drop(database);
        let database = HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            database
                .connection
                .query_row("SELECT payload FROM source_apps", [], |row| row
                    .get::<_, String>(0))
                .unwrap(),
            "synthetic preserved source"
        );
        assert!(!database.clipboard_capture_enabled().unwrap());
        let restored = database.record("preserved-translation").unwrap().unwrap();
        assert_eq!(restored.text, translation.text);
        assert_eq!(restored.translations, translation.translations);
        assert!(restored.is_favorite);
        assert_eq!(
            database
                .connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            2
        );
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    use std::path::Path;
}
