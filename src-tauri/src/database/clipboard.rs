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

pub(super) fn migrate_v3(connection: &mut rusqlite::Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch("DROP TRIGGER clipboard_insert; DROP TRIGGER clipboard_delete; DROP TABLE clipboard_search;
        ALTER TABLE clipboard_records RENAME TO clipboard_records_v2;
        CREATE TABLE clipboard_records (
            sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
            kind TEXT NOT NULL CHECK(kind IN ('text','image','files')), summary TEXT NOT NULL,
            date TEXT NOT NULL, created_at INTEGER NOT NULL,
            is_favorite INTEGER NOT NULL CHECK(is_favorite IN (0,1)),
            is_pinned INTEGER NOT NULL CHECK(is_pinned IN (0,1)),
            source_app_id INTEGER REFERENCES source_apps(id), image_path TEXT, fingerprint TEXT NOT NULL
        );
        CREATE TABLE clipboard_contents (
            sequence INTEGER PRIMARY KEY REFERENCES clipboard_records(sequence) ON DELETE CASCADE,
            text TEXT NOT NULL, search_folded TEXT NOT NULL
        );
        INSERT OR IGNORE INTO source_apps(payload) SELECT DISTINCT source_app FROM clipboard_records_v2 WHERE source_app IS NOT NULL;
    ").map_err(storage_error)?;
    {
        let mut rows = tx.prepare("SELECT sequence,id,kind,summary,date,is_favorite,is_pinned,source_app,image_path,fingerprint FROM clipboard_records_v2").map_err(storage_error)?;
        let mut source = rows.query([]).map_err(storage_error)?;
        let mut insert = tx.prepare("INSERT INTO clipboard_records(sequence,id,kind,summary,date,created_at,is_favorite,is_pinned,source_app_id,image_path,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,(SELECT id FROM source_apps WHERE payload=?9),?10,?11)").map_err(storage_error)?;
        while let Some(row) = source.next().map_err(storage_error)? {
            let date: String = row.get(4).map_err(storage_error)?;
            let created = chrono::DateTime::parse_from_rfc3339(&date)
                .map_err(|_| "storage_corrupt")?
                .timestamp_millis();
            let app: Option<String> = row.get(7).map_err(storage_error)?;
            if let Some(app) = &app {
                serde_json::from_str::<serde_json::Value>(app).map_err(|_| "storage_corrupt")?;
            }
            insert
                .execute(params![
                    row.get::<_, i64>(0).map_err(storage_error)?,
                    row.get::<_, String>(1).map_err(storage_error)?,
                    row.get::<_, String>(2).map_err(storage_error)?,
                    row.get::<_, String>(3).map_err(storage_error)?,
                    date,
                    created,
                    row.get::<_, bool>(5).map_err(storage_error)?,
                    row.get::<_, bool>(6).map_err(storage_error)?,
                    app,
                    row.get::<_, Option<String>>(8).map_err(storage_error)?,
                    row.get::<_, String>(9).map_err(storage_error)?
                ])
                .map_err(storage_error)?;
        }
    }
    tx.execute_batch("INSERT INTO clipboard_contents(sequence,text,search_folded) SELECT sequence,text,search_folded FROM clipboard_records_v2;
        DROP TABLE clipboard_records_v2;
        CREATE INDEX clipboard_order ON clipboard_records(is_pinned DESC,created_at DESC,sequence DESC);
        CREATE INDEX clipboard_favorites ON clipboard_records(is_favorite,is_pinned DESC,created_at DESC,sequence DESC);
        CREATE INDEX clipboard_fingerprint ON clipboard_records(fingerprint);
        CREATE VIRTUAL TABLE clipboard_search USING fts5(search_folded,content='clipboard_contents',content_rowid='sequence',tokenize='trigram case_sensitive 1');
        INSERT INTO clipboard_search(clipboard_search) VALUES('rebuild');
        CREATE TRIGGER clipboard_insert AFTER INSERT ON clipboard_contents BEGIN
            INSERT INTO clipboard_search(rowid,search_folded) VALUES(new.sequence,new.search_folded);
        END;
        CREATE TRIGGER clipboard_delete AFTER DELETE ON clipboard_contents BEGIN
            INSERT INTO clipboard_search(clipboard_search,rowid,search_folded) VALUES('delete',old.sequence,old.search_folded);
        END;
        PRAGMA user_version=3;").map_err(storage_error)?;
    tx.commit().map_err(storage_error)
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
    "r.sequence,r.id,r.kind,c.text,r.date,r.is_favorite,r.is_pinned,a.payload,r.image_path,r.fingerprint";

pub(super) fn ensure_filter_indexes(connection: &rusqlite::Connection) -> Result<(), String> {
    connection.execute_batch("CREATE INDEX IF NOT EXISTS clipboard_kind_order ON clipboard_records(kind,is_pinned DESC,created_at DESC,sequence DESC);
        CREATE INDEX IF NOT EXISTS clipboard_kind_favorites ON clipboard_records(kind,is_favorite,is_pinned DESC,created_at DESC,sequence DESC);").map_err(storage_error)
}

impl HistoryDatabase {
    pub fn clipboard_page(
        &self,
        query: &str,
        favorites_only: bool,
        cursor: Option<&ClipboardCursor>,
        limit: usize,
    ) -> Result<ClipboardPage, String> {
        self.clipboard_page_filtered(query, favorites_only, None, cursor, limit)
    }

    pub fn clipboard_page_filtered(
        &self,
        query: &str,
        favorites_only: bool,
        kind: Option<&str>,
        cursor: Option<&ClipboardCursor>,
        limit: usize,
    ) -> Result<ClipboardPage, String> {
        if query.chars().count() > 4096 || !(1..=200).contains(&limit) {
            return Err("invalid_argument".into());
        }
        if kind.is_some_and(|kind| !["text", "image", "files"].contains(&kind)) {
            return Err("invalid_argument".into());
        }
        let kind_filter = if kind.is_some() {
            " AND kind=:kind"
        } else {
            ""
        };
        let needle = query.trim().to_lowercase();
        let indexed = needle.chars().count() >= 3 && !needle.contains('\0');
        let favorite = if favorites_only {
            " AND is_favorite=1"
        } else {
            ""
        };
        let matched = format!("\"{}\"", needle.replace('"', "\"\""));
        let broad = if indexed {
            let mut probe = self
                .connection
                .prepare_cached(
                    "SELECT rowid FROM clipboard_search WHERE clipboard_search MATCH ?1 LIMIT 1024",
                )
                .map_err(storage_error)?;
            let mut rows = probe.query([&matched]).map_err(storage_error)?;
            let mut count = 0;
            while rows.next().map_err(storage_error)?.is_some() {
                count += 1;
            }
            count == 1024
        } else {
            false
        };
        let search = if needle.is_empty() {
            ""
        } else if indexed && !broad {
            " AND sequence IN (SELECT rowid FROM clipboard_search WHERE clipboard_search MATCH :match) AND EXISTS (SELECT 1 FROM clipboard_contents c WHERE c.sequence=clipboard_records.sequence AND instr(c.search_folded,:needle)>0)"
        } else {
            " AND EXISTS (SELECT 1 FROM clipboard_contents c WHERE c.sequence=clipboard_records.sequence AND instr(c.search_folded,:needle)>0)"
        };
        let hint = if broad {
            if kind.is_some() && favorites_only {
                " INDEXED BY clipboard_kind_favorites"
            } else if kind.is_some() {
                " INDEXED BY clipboard_kind_order"
            } else if favorites_only {
                " INDEXED BY clipboard_favorites"
            } else {
                " INDEXED BY clipboard_order"
            }
        } else {
            ""
        };
        let sql = format!("SELECT r.sequence,r.id,r.kind,r.summary,r.date,r.is_favorite,r.is_pinned,r.source_app_id,r.created_at,r.title FROM clipboard_records r WHERE r.sequence IN (SELECT sequence FROM clipboard_records{hint} WHERE (is_pinned,created_at,sequence)<(:pinned,:created,:sequence){favorite}{kind_filter}{search} ORDER BY is_pinned DESC,created_at DESC,sequence DESC LIMIT :limit) ORDER BY r.is_pinned DESC,r.created_at DESC,r.sequence DESC");
        let mut statement = self
            .connection
            .prepare_cached(&sql)
            .map_err(storage_error)?;
        let pinned = cursor.map_or(2i64, |cursor| i64::from(cursor.pinned));
        let created = cursor.map_or(i64::MAX, |cursor| cursor.created_at);
        let sequence = cursor.map_or(i64::MAX, |cursor| cursor.sequence);
        let take = limit as i64 + 1;
        let mut filters: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
        if let Some(kind) = &kind {
            filters.push((":kind", kind));
        }
        if !needle.is_empty() {
            filters.push((":needle", &needle));
            if indexed && !broad {
                filters.push((":match", &matched));
            }
        }
        let total_records = if needle.is_empty() && !favorites_only && kind.is_none() {
            self.clipboard_total_records
        } else {
            let sql = format!(
                "SELECT count(*) FROM clipboard_records WHERE 1=1{favorite}{kind_filter}{search}"
            );
            self.connection
                .prepare_cached(&sql)
                .map_err(storage_error)?
                .query_row(filters.as_slice(), |row| row.get::<_, i64>(0))
                .map_err(storage_error)? as usize
        };
        let mut bindings: Vec<(&str, &dyn rusqlite::ToSql)> = vec![
            (":pinned", &pinned),
            (":created", &created),
            (":sequence", &sequence),
            (":limit", &take),
        ];
        bindings.extend(filters);
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
            let record = crate::clipboard::ClipboardSummary {
                title: row.get(9).map_err(storage_error)?,
                id: row.get(1).map_err(storage_error)?,
                kind: row.get(2).map_err(storage_error)?,
                text: row.get(3).map_err(storage_error)?,
                date: row.get(4).map_err(storage_error)?,
                is_favorite: row.get(5).map_err(storage_error)?,
                is_pinned: row.get(6).map_err(storage_error)?,
                source_app_id: row.get(7).map_err(storage_error)?,
            };
            cursor = Some(ClipboardCursor {
                pinned: record.is_pinned,
                created_at: row.get(8).map_err(storage_error)?,
                sequence: row.get(0).map_err(storage_error)?,
            });
            records.push(record);
        }
        let mut sources = std::collections::BTreeMap::new();
        for record in &records {
            if let Some(id) = record.source_app_id {
                if let std::collections::btree_map::Entry::Vacant(entry) = sources.entry(id) {
                    let json: String = self
                        .connection
                        .query_row("SELECT payload FROM source_apps WHERE id=?1", [id], |row| {
                            row.get(0)
                        })
                        .map_err(storage_error)?;
                    entry.insert(serde_json::from_str(&json).map_err(|_| "storage_corrupt")?);
                }
            }
        }
        Ok(ClipboardPage {
            records,
            cursor,
            has_more,
            total_records,
            sources,
        })
    }

    pub fn clipboard_record(&self, id: &str) -> Result<ClipboardRecord, String> {
        self.connection
            .query_row(
                &format!("SELECT {COLUMNS} FROM clipboard_records r JOIN clipboard_contents c USING(sequence) LEFT JOIN source_apps a ON a.id=r.source_app_id WHERE r.id=?1"),
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
        let created = chrono::DateTime::parse_from_rfc3339(&record.date)
            .map_err(|_| "invalid_argument")?
            .timestamp_millis();
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let source_id = if let Some(source) = source {
            transaction
                .execute(
                    "INSERT OR IGNORE INTO source_apps(payload) VALUES(?1)",
                    [&source],
                )
                .map_err(storage_error)?;
            Some(
                transaction
                    .query_row(
                        "SELECT id FROM source_apps WHERE payload=?1",
                        [&source],
                        |row| row.get::<_, i64>(0),
                    )
                    .map_err(storage_error)?,
            )
        } else {
            None
        };
        let duplicate: Option<String> = transaction.query_row("SELECT id FROM clipboard_records WHERE fingerprint=?1 AND kind=?2 AND ((kind='image' AND image_path IS ?4) OR (kind!='image' AND EXISTS (SELECT 1 FROM clipboard_contents c WHERE c.sequence=clipboard_records.sequence AND c.text=?3))) LIMIT 1", params![record.fingerprint,record.kind,record.text,record.image_path], |row| row.get(0)).optional().map_err(storage_error)?;
        let inserted = duplicate.is_none();
        let id = if let Some(id) = duplicate {
            transaction
                .execute(
                    "UPDATE clipboard_records SET date=?1,source_app_id=?2,created_at=?4 WHERE id=?3",
                    params![record.date, source_id, id, created],
                )
                .map_err(storage_error)?;
            id
        } else {
            transaction.execute("INSERT INTO clipboard_records(id,kind,summary,date,created_at,is_favorite,is_pinned,source_app_id,image_path,fingerprint) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",params![record.id,record.kind,summary,record.date,created,record.is_favorite,record.is_pinned,source_id,record.image_path,record.fingerprint]).map_err(storage_error)?;
            transaction
                .execute(
                    "INSERT INTO clipboard_contents(sequence,text,search_folded) VALUES(?1,?2,?3)",
                    params![
                        transaction.last_insert_rowid(),
                        record.text,
                        search_text.to_lowercase()
                    ],
                )
                .map_err(storage_error)?;
            record.id.clone()
        };
        transaction.commit().map_err(storage_error)?;
        if inserted {
            self.clipboard_total_records += 1;
        }
        self.clipboard_record(&id)
    }

    pub fn clipboard_detail(&self, id: &str) -> Result<crate::clipboard::ClipboardDetail, String> {
        let record = self.clipboard_record(id)?;
        let title = self
            .connection
            .query_row(
                "SELECT title FROM clipboard_records WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .map_err(storage_error)?;
        Ok(crate::clipboard::ClipboardDetail {
            title,
            text: record.text,
            kind: record.kind,
        })
    }

    pub fn clipboard_edit(
        &mut self,
        id: &str,
        title: &str,
        text: Option<&str>,
    ) -> Result<(), String> {
        if title.chars().count() > 200
            || title.contains('\0')
            || text.is_some_and(|text| text.len() > MAX_TEXT_BYTES)
        {
            return Err("invalid_argument".into());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let (sequence, kind): (i64, String) = tx
            .query_row(
                "SELECT sequence,kind FROM clipboard_records WHERE id=?1",
                [id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(storage_error)?
            .ok_or("not_found")?;
        if let Some(text) = text {
            if kind != "text" {
                return Err("invalid_argument".into());
            }
            // Delete/insert fires the existing external-content FTS triggers atomically.
            tx.execute(
                "DELETE FROM clipboard_contents WHERE sequence=?1",
                [sequence],
            )
            .map_err(storage_error)?;
            tx.execute(
                "INSERT INTO clipboard_contents(sequence,text,search_folded) VALUES(?1,?2,?3)",
                params![sequence, text, text.to_lowercase()],
            )
            .map_err(storage_error)?;
            let summary: String = text.chars().take(256).collect();
            tx.execute(
                "UPDATE clipboard_records SET summary=?2,fingerprint=?3 WHERE id=?1",
                params![
                    id,
                    summary,
                    crate::clipboard::fingerprint("text", text.as_bytes())
                ],
            )
            .map_err(storage_error)?;
        }
        tx.execute(
            "UPDATE clipboard_records SET title=?2 WHERE id=?1",
            params![id, title],
        )
        .map_err(storage_error)?;
        tx.commit().map_err(storage_error)
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
        transaction.commit().map_err(storage_error)?;
        if action == "delete" {
            self.clipboard_total_records -= 1;
        }
        Ok(())
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
    fn clipboard_editor_migrates_v3_and_preserves_full_content_flags_and_identity() {
        let root = std::env::temp_dir().join(format!("nami-editor-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.sqlite3");
        let mut db = HistoryDatabase::open(&path).unwrap();
        let original = record("edit", &"Original text ".repeat(100));
        db.clipboard_insert(&original).unwrap();
        db.clipboard_update("edit", "favorite").unwrap();
        db.connection
            .execute_batch(
                "ALTER TABLE clipboard_records DROP COLUMN title; PRAGMA user_version=3;",
            )
            .unwrap();
        drop(db);
        let mut db = HistoryDatabase::open(&path).unwrap();
        let detail = db.clipboard_detail("edit").unwrap();
        assert!(detail.title.is_empty());
        assert_eq!(detail.text, original.text);
        let edited = "Updated 🌊\ncomplete content ".repeat(100);
        db.clipboard_edit("edit", "My title", Some(&edited))
            .unwrap();
        assert!(db
            .clipboard_page("Original", false, None, 100)
            .unwrap()
            .records
            .is_empty());
        let page = db.clipboard_page("Updated", true, None, 100).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].title, "My title");
        assert_eq!(page.records[0].text.chars().count(), 256);
        assert_eq!(page.total_records, 1);
        assert!(db.clipboard_record("edit").unwrap().is_favorite);
        // Re-copying edited text reuses the record and its title.
        assert_eq!(
            db.clipboard_insert(&record("recopy", &edited)).unwrap().id,
            "edit"
        );
        drop(db);
        let db = HistoryDatabase::open(&path).unwrap();
        assert_eq!(db.clipboard_detail("edit").unwrap().text, edited);
        assert_eq!(db.clipboard_detail("edit").unwrap().title, "My title");
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn clipboard_editor_rejects_invalid_edits_and_rolls_back_content_and_search() {
        let mut db = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        db.clipboard_insert(&record("edit", "Original text"))
            .unwrap();
        assert_eq!(
            db.clipboard_edit("missing", "", None).unwrap_err(),
            "not_found"
        );
        assert_eq!(
            db.clipboard_edit("edit", &"x".repeat(201), None)
                .unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_edit("edit", "bad\0title", None).unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_edit("edit", "", Some(&"x".repeat(MAX_TEXT_BYTES + 1)))
                .unwrap_err(),
            "invalid_argument"
        );
        db.connection.execute_batch("CREATE TRIGGER reject_title BEFORE UPDATE OF title ON clipboard_records BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(db
            .clipboard_edit("edit", "New title", Some("Replacement text"))
            .is_err());
        assert_eq!(db.clipboard_record("edit").unwrap().text, "Original text");
        assert_eq!(
            db.clipboard_page("Original", false, None, 100)
                .unwrap()
                .records
                .len(),
            1
        );
        assert!(db
            .clipboard_page("Replacement", false, None, 100)
            .unwrap()
            .records
            .is_empty());
        db.connection
            .execute_batch("DROP TRIGGER reject_title")
            .unwrap();
        let mut files = record(
            "files",
            &serde_json::to_string(&vec![file_path("test.txt")]).unwrap(),
        );
        files.kind = "files".into();
        db.clipboard_insert(&files).unwrap();
        assert_eq!(
            db.clipboard_edit("files", "", Some("new text"))
                .unwrap_err(),
            "invalid_argument"
        );
        db.clipboard_edit("files", "Files title", None).unwrap();
        assert_eq!(db.clipboard_record("files").unwrap().text, files.text);
        assert_eq!(db.clipboard_detail("files").unwrap().title, "Files title");
        db.clipboard_update("files", "delete").unwrap();
        assert!(db.clipboard_detail("files").is_err());
    }

    fn file_path(relative: &str) -> String {
        let path = std::env::temp_dir().join("nami-synthetic").join(relative);
        assert!(
            path.is_absolute(),
            "file fixtures require native absolute paths"
        );
        path.to_string_lossy().into_owned()
    }

    #[test]
    fn version_two_migration_preserves_all_content_and_rolls_back_on_bad_data() {
        for corrupt in [false, true] {
            let directory =
                std::env::temp_dir().join(format!("nami-clipboard-v2-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("test.sqlite3");
            let mut db = HistoryDatabase::open(&path).unwrap();
            db.connection.execute_batch("DROP TRIGGER clipboard_insert; DROP TRIGGER clipboard_delete; DROP TABLE clipboard_search; DROP TABLE clipboard_contents; DROP TABLE clipboard_records; DROP TABLE preferences;").unwrap();
            migrate(&mut db.connection).unwrap();
            let source =
                serde_json::json!({"name":"Synthetic","iconDataUrl":"data:image/png;base64,test"})
                    .to_string();
            for (index, kind) in ["text", "image", "files"].into_iter().enumerate() {
                let text = match kind {
                    "files" => "[\"/synthetic/a.txt\"]",
                    "image" => "",
                    _ => "完整正文\0🌊\nlast",
                };
                let date = if corrupt && index == 2 {
                    "invalid"
                } else {
                    "2026-11-15T08:00:00+08:00"
                };
                db.connection.execute("INSERT INTO clipboard_records(id,kind,text,summary,search_folded,date,is_favorite,is_pinned,source_app,image_path,fingerprint) VALUES(?1,?2,?3,?3,?3,?4,1,1,?5,?6,?7)",params![kind,kind,text,date,source,if kind=="image"{Some("origin/ab/abcdef.png")}else{None},kind]).unwrap();
            }
            db.connection
                .execute(
                    "UPDATE preferences SET value='true' WHERE key='clipboard_capture'",
                    [],
                )
                .unwrap();
            drop(db);
            if corrupt {
                assert!(
                    matches!(HistoryDatabase::open(&path), Err(error) if error=="storage_corrupt")
                );
                let check = rusqlite::Connection::open(&path).unwrap();
                assert_eq!(
                    check
                        .pragma_query_value(None, "user_version", |r| r.get::<_, i64>(0))
                        .unwrap(),
                    2
                );
                assert_eq!(
                    check
                        .query_row(
                            "SELECT count(*) FROM clipboard_records WHERE source_app IS NOT NULL",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    3
                );
                assert_eq!(
                    check
                        .query_row(
                            "SELECT count(*) FROM sqlite_master WHERE name='clipboard_contents'",
                            [],
                            |r| r.get::<_, i64>(0)
                        )
                        .unwrap(),
                    0
                );
            } else {
                let mut db = HistoryDatabase::open(&path).unwrap();
                assert!(db.clipboard_capture_enabled().unwrap());
                let page = db.clipboard_page("", true, None, 100).unwrap();
                assert_eq!(page.total_records, 3);
                assert_eq!(page.sources.len(), 1);
                assert_eq!(
                    page.records
                        .iter()
                        .map(|r| r.id.as_str())
                        .collect::<Vec<_>>(),
                    vec!["files", "image", "text"]
                );
                let restored = db.clipboard_record("text").unwrap();
                assert_eq!(restored.text, "完整正文\0🌊\nlast");
                assert!(restored.is_favorite && restored.is_pinned);
                assert_eq!(restored.source_app.unwrap()["name"], "Synthetic");
                assert_eq!(
                    db.clipboard_record("image").unwrap().image_path.as_deref(),
                    Some("origin/ab/abcdef.png")
                );
                assert_eq!(
                    db.clipboard_record("files").unwrap().text,
                    "[\"/synthetic/a.txt\"]"
                );
                db.clipboard_update("text", "favorite").unwrap();
                db.clipboard_update("text", "pin").unwrap();
                db.clipboard_update("text", "delete").unwrap();
                assert_eq!(
                    db.clipboard_page("", false, None, 100)
                        .unwrap()
                        .total_records,
                    2
                );
                assert!(db
                    .clipboard_page("last", false, None, 100)
                    .unwrap()
                    .records
                    .is_empty());
                assert_eq!(
                    db.connection
                        .query_row("SELECT count(*) FROM clipboard_contents", [], |r| r
                            .get::<_, i64>(0))
                        .unwrap(),
                    2
                );
            }
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn summary_pages_deduplicate_sources_and_count_only_successful_inserts() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let source = serde_json::json!({"name":"Synthetic","iconDataUrl":"data:image/png;base64,".to_owned()+&"A".repeat(4096)});
        for index in 0..100 {
            let mut item = record(&format!("record-{index}"), &format!("synthetic {index}"));
            item.source_app = Some(source.clone());
            db.clipboard_insert(&item).unwrap();
        }
        let page = db.clipboard_page("", false, None, 100).unwrap();
        assert_eq!(page.sources.len(), 1);
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM source_apps", [], |r| r
                    .get::<_, i64>(0))
                .unwrap(),
            1
        );
        let json = serde_json::to_string(&page).unwrap();
        assert_eq!(json.matches("iconDataUrl").count(), 1);
        assert!(!json.contains("fingerprint") && !json.contains("imagePath"));
        assert!(json.len() < 35_000, "summary payload {}", json.len());
        let mut duplicate = record("duplicate", "synthetic 0");
        duplicate.source_app = Some(source);
        assert_eq!(db.clipboard_insert(&duplicate).unwrap().id, "record-0");
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            100
        );
        let conflicting = record("record-0", "different text");
        assert!(db.clipboard_insert(&conflicting).is_err());
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            100
        );
        db.clipboard_update("record-0", "delete").unwrap();
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            99
        );
    }

    #[test]
    fn integer_cursor_orders_timezones_and_pinned_pages_without_duplicates() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for (id, date) in [
            ("later", "2026-11-15T01:00:00Z"),
            ("earlier", "2026-11-15T08:00:00+08:00"),
        ] {
            let mut item = record(id, id);
            item.date = date.into();
            db.clipboard_insert(&item).unwrap();
        }
        let first = db.clipboard_page("", false, None, 1).unwrap();
        assert_eq!(first.records[0].id, "later");
        let next = db
            .clipboard_page("", false, first.cursor.as_ref(), 1)
            .unwrap();
        assert_eq!(next.records[0].id, "earlier");
        assert!(!next.has_more);
    }

    #[test]
    fn broad_search_preserves_literal_matching_and_favorite_cursor_order() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for index in 0..1100 {
            let mut item = record(
                &format!("broad-{index}"),
                &format!("common literal %_ {index}"),
            );
            item.is_favorite = index % 3 == 0;
            item.is_pinned = index % 17 == 0;
            db.clipboard_insert(&item).unwrap();
        }
        let mut cursor = None;
        let mut ids = std::collections::HashSet::new();
        loop {
            let page = db
                .clipboard_page("common", true, cursor.as_ref(), 100)
                .unwrap();
            for item in &page.records {
                assert!(item.is_favorite);
                assert!(ids.insert(item.id.clone()));
            }
            cursor = page.cursor;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(ids.len(), 367);
        assert!(db
            .clipboard_page("common%", false, None, 100)
            .unwrap()
            .records
            .is_empty());
        assert_eq!(
            db.clipboard_page("%_", false, None, 100)
                .unwrap()
                .records
                .len(),
            100
        );
    }

    #[test]
    #[ignore = "explicit isolated 10k/100k clipboard performance measurement"]
    fn benchmark_large_clipboard() {
        fn summary(mut times: Vec<f64>) -> serde_json::Value {
            times.sort_by(f64::total_cmp);
            serde_json::json!({"p50Ms":times[times.len()/2],"p95Ms":times[times.len()*95/100],"samples":times.len()})
        }
        let mut reports = Vec::new();
        for size in [10_000, 100_000] {
            let directory = std::env::temp_dir()
                .join(format!("nami-clipboard-benchmark-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("test.sqlite3");
            let mut db = HistoryDatabase::open(&path).unwrap();
            let source=serde_json::json!({"name":"Synthetic","iconDataUrl":"data:image/png;base64,".to_owned()+&"A".repeat(4096)}).to_string();
            {
                let tx = db.connection.transaction().unwrap();
                tx.execute("INSERT INTO source_apps(payload) VALUES(?1)", [&source])
                    .unwrap();
                let source_id = tx.last_insert_rowid();
                let mut metadata=tx.prepare("INSERT INTO clipboard_records(id,kind,summary,date,created_at,is_favorite,is_pinned,source_app_id,fingerprint) VALUES(?1,'text',?2,'2026-11-15T00:00:00Z',?3,?4,?5,?6,?1)").unwrap();
                let mut contents=tx.prepare("INSERT INTO clipboard_contents(sequence,text,search_folded) VALUES(?1,?2,?2)").unwrap();
                for index in 0..size {
                    let text =
                        format!("common synthetic clipboard {index:06} {}", "x".repeat(1000));
                    metadata
                        .execute(params![
                            format!("item-{index}"),
                            text.chars().take(256).collect::<String>(),
                            index,
                            index % 10 == 0,
                            index % 1000 == 0,
                            source_id
                        ])
                        .unwrap();
                    contents
                        .execute(params![tx.last_insert_rowid(), text])
                        .unwrap();
                }
                drop(metadata);
                drop(contents);
                tx.commit().unwrap();
            }
            drop(db);
            let mut db = HistoryDatabase::open(&path).unwrap();
            let mut measurements = serde_json::Map::new();
            for (label, query, favorite, kind) in [
                ("all", "", false, None),
                ("favorites", "", true, None),
                ("sparse", "099999", false, None),
                ("broad", "common", false, None),
                ("text", "", false, Some("text")),
                ("favoriteText", "", true, Some("text")),
                ("image", "", false, Some("image")),
                ("broadText", "common", false, Some("text")),
            ] {
                let mut times = Vec::new();
                for _ in 0..30 {
                    let started = std::time::Instant::now();
                    let page = db
                        .clipboard_page_filtered(query, favorite, kind, None, 100)
                        .unwrap();
                    let _json = serde_json::to_vec(&page).unwrap();
                    times.push(started.elapsed().as_secs_f64() * 1000.0);
                }
                let stats = summary(times);
                assert!(stats["p95Ms"].as_f64().unwrap() < 100.0, "{label}: {stats}");
                measurements.insert(label.into(), stats);
            }
            let mut cursor = None;
            for _ in 0..(size / 100 - 1) {
                cursor = db
                    .clipboard_page("", false, cursor.as_ref(), 100)
                    .unwrap()
                    .cursor;
            }
            let mut deep = Vec::new();
            let mut writes = Vec::new();
            for index in 0..30 {
                let started = std::time::Instant::now();
                db.clipboard_page("", false, cursor.as_ref(), 100).unwrap();
                deep.push(started.elapsed().as_secs_f64() * 1000.0);
                let started = std::time::Instant::now();
                db.clipboard_insert(&record(
                    &format!("write-{index}"),
                    &format!("durable-{index}"),
                ))
                .unwrap();
                writes.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            let page = db.clipboard_page("", false, None, 100).unwrap();
            let bytes = serde_json::to_vec(&page).unwrap().len();
            assert!(bytes < 70_000, "page {bytes}");
            measurements.insert("deepPage".into(), summary(deep));
            measurements.insert("durableWrite".into(), summary(writes));
            db.connection
                .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
                .unwrap();
            reports.push(serde_json::json!({"records":size,"measurements":measurements,"pageBytes":bytes,"databaseBytes":std::fs::metadata(&path).unwrap().len()}));
            drop(db);
            std::fs::remove_dir_all(directory).unwrap();
        }
        let report = serde_json::json!({"scope":"isolated synthetic clipboard; database+summary JSON, not click-to-frame or physical disk cold start","runs":reports});
        let output = Path::new(env!("CARGO_MANIFEST_DIR")).join("target/clipboard-benchmark.json");
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
        println!("{report}");
    }

    #[test]
    fn file_count_limit_matches_capture_and_restore() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let paths: Vec<_> = (0..MAX_FILE_COUNT)
            .map(|index| file_path(&format!("{index}.txt")))
            .collect();
        let mut item = record("files-limit", &serde_json::to_string(&paths).unwrap());
        item.kind = "files".into();
        database.clipboard_insert(&item).unwrap();
        assert_eq!(database.clipboard_record(&item.id).unwrap().text, item.text);
        let mut oversized = paths;
        oversized.push(file_path("one-more.txt"));
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
    fn kind_filters_combine_with_search_favorites_and_cursor_totals() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for kind in ["text", "image", "files"] {
            for index in 0..9 {
                let id = format!("{kind}-{index}");
                let text = format!("{} {id}", if index < 6 { "common" } else { "other" });
                let mut item = record(&id, &text);
                item.kind = kind.into();
                item.is_favorite = index % 2 == 0;
                item.is_pinned = index == 0;
                if kind == "image" {
                    item.image_path = Some(format!("origin/ab/{id}.png"));
                } else if kind == "files" {
                    item.text =
                        serde_json::to_string(&[file_path(&format!("{text}.txt"))]).unwrap();
                }
                item.fingerprint = fingerprint(kind, item.text.as_bytes());
                database.clipboard_insert(&item).unwrap();
            }
        }
        for kind in ["text", "image", "files"] {
            for (query, favorite, expected) in [
                ("", false, 9),
                ("", true, 5),
                ("common", false, 6),
                ("common", true, 3),
                ("co", true, 3),
                ("unmatched", false, 0),
            ] {
                let mut cursor = None;
                let mut ids = std::collections::HashSet::new();
                loop {
                    let page = database
                        .clipboard_page_filtered(query, favorite, Some(kind), cursor.as_ref(), 2)
                        .unwrap();
                    assert_eq!(page.total_records, expected, "{query}/{favorite}/{kind}");
                    for row in page.records {
                        assert_eq!(row.kind, kind);
                        assert!(!favorite || row.is_favorite);
                        assert!(ids.insert(row.id));
                    }
                    cursor = page.cursor;
                    if !page.has_more {
                        break;
                    }
                }
                assert_eq!(ids.len(), expected);
            }
        }
        assert_eq!(
            database
                .clipboard_page("common", false, None, 100)
                .unwrap()
                .total_records,
            18
        );
        assert_eq!(
            database
                .clipboard_page("", true, None, 100)
                .unwrap()
                .total_records,
            15
        );
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            27
        );
        for invalid in ["", "folder", "TEXT", "text' OR 1=1 --"] {
            assert!(
                matches!(database.clipboard_page_filtered("", false, Some(invalid), None, 100), Err(error) if error == "invalid_argument")
            );
        }
    }

    #[test]
    fn kind_filter_indexes_cover_both_sort_orders_and_existing_databases() {
        let database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        ensure_filter_indexes(&database.connection).unwrap();
        for (favorite, index) in [
            ("", "clipboard_kind_order"),
            (" AND is_favorite=1", "clipboard_kind_favorites"),
        ] {
            let sql = format!("EXPLAIN QUERY PLAN SELECT sequence FROM clipboard_records WHERE kind='text'{favorite} AND (is_pinned,created_at,sequence)<(2,9223372036854775807,9223372036854775807) ORDER BY is_pinned DESC,created_at DESC,sequence DESC LIMIT 101");
            let mut statement = database.connection.prepare(&sql).unwrap();
            let plan = statement
                .query_map([], |row| row.get::<_, String>(3))
                .unwrap()
                .map(Result::unwrap)
                .collect::<Vec<_>>()
                .join("\n");
            assert!(plan.contains(index), "{plan}");
            assert!(!plan.contains("TEMP B-TREE"), "{plan}");
        }
    }

    #[test]
    fn clipboard_kinds_keep_original_content_and_do_not_merge_hash_collisions() {
        let mut database = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let mut files = record(
            "files",
            &serde_json::to_string(&[file_path("first.txt"), file_path("second.png")]).unwrap(),
        );
        files.kind = "files".into();
        files.fingerprint = fingerprint("files", files.text.as_bytes());
        database.clipboard_insert(&files).unwrap();
        assert_eq!(database.clipboard_record("files").unwrap().text, files.text);
        let mut collision = record(
            "collision",
            &serde_json::to_string(&[file_path("different.txt")]).unwrap(),
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
            file_path("资料/第一行\n第二行.txt"),
            file_path("世界🌊.png"),
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
            file_path("hello\"world.txt"),
            file_path("hello\\world.txt"),
            file_path("hello\nworld.txt"),
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
        database.connection.execute_batch("DROP TRIGGER clipboard_insert; DROP TRIGGER clipboard_delete; DROP TABLE clipboard_search; DROP TABLE clipboard_contents; DROP TABLE clipboard_records; DROP TABLE preferences; PRAGMA user_version=1;").unwrap();
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
            super::super::VERSION
        );
        drop(database);
        std::fs::remove_dir_all(directory).unwrap();
    }

    use std::path::Path;
}
