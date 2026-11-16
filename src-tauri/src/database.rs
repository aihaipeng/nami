mod clipboard;

use crate::model::HistoryRecord;
use rusqlite::{params, Connection, ErrorCode, Row, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};
use std::{path::Path, time::Duration};

const APPLICATION_ID: i64 = 0x4E414D49;
const VERSION: i64 = 4;
pub const PAGE_SIZE: usize = 100;

#[derive(Clone, Default, PartialEq, Eq)]
pub struct HistoryQuery {
    pub text: String,
    pub favorites_only: bool,
}

impl HistoryQuery {
    pub fn matches(&self, record: &HistoryRecord) -> bool {
        let needle = self.text.trim().to_lowercase();
        (!self.favorites_only || record.is_favorite)
            && (needle.is_empty()
                || record.text.to_lowercase().contains(&needle)
                || record
                    .translations
                    .iter()
                    .any(|text| text.to_lowercase().contains(&needle)))
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct HistoryCursor {
    pub pinned: bool,
    pub sequence: i64,
    pub created_at: i64,
}

#[derive(Clone)]
pub struct HistoryPage {
    pub records: Vec<HistoryRecord>,
    pub cursor: Option<HistoryCursor>,
    pub has_more: bool,
    pub total_records: usize,
}

pub struct HistoryDatabase {
    connection: Connection,
    total_records: usize,
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

fn decode(row: &Row<'_>) -> Result<HistoryRecord, String> {
    let id: String = row.get(1).map_err(storage_error)?;
    let json: String = row.get(2).map_err(storage_error)?;
    let mut record: HistoryRecord = serde_json::from_str(&json).map_err(|_| "storage_corrupt")?;
    if record.id != id
        || record.id.is_empty()
        || !(1..=2).contains(&record.target_languages.len())
        || record.target_languages.len() != record.translations.len()
    {
        return Err("storage_corrupt".into());
    }
    record.is_favorite = row.get(3).map_err(storage_error)?;
    record.is_pinned = row.get(4).map_err(storage_error)?;
    record.source_app = row
        .get::<_, Option<String>>(5)
        .map_err(storage_error)?
        .map(|json| serde_json::from_str(&json).map_err(|_| "storage_corrupt".to_owned()))
        .transpose()?;
    Ok(record)
}

const COLUMNS: &str =
    "r.sequence, r.id, r.record_json, r.is_favorite, r.is_pinned, a.payload, r.created_at";

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
            transaction.execute_batch("CREATE TABLE source_apps (id INTEGER PRIMARY KEY, payload TEXT NOT NULL UNIQUE);
                CREATE TABLE translation_records (
                    sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE, created_at INTEGER NOT NULL,
                    record_json TEXT NOT NULL, engine TEXT NOT NULL CHECK(engine IN ('edge','google')),
                    is_favorite INTEGER NOT NULL CHECK(is_favorite IN (0,1)),
                    is_pinned INTEGER NOT NULL CHECK(is_pinned IN (0,1)),
                    source_app_id INTEGER REFERENCES source_apps(id),
                    source_folded TEXT NOT NULL, primary_folded TEXT NOT NULL, secondary_folded TEXT NOT NULL
                );
                CREATE INDEX history_order ON translation_records(is_pinned DESC, created_at DESC, sequence DESC);
                CREATE INDEX history_favorites ON translation_records(is_favorite, is_pinned DESC, created_at DESC, sequence DESC);
                CREATE VIRTUAL TABLE translation_search USING fts5(source_folded,primary_folded,secondary_folded,content='translation_records',content_rowid='sequence',tokenize='trigram case_sensitive 1');
                CREATE TRIGGER history_insert AFTER INSERT ON translation_records BEGIN
                    INSERT INTO translation_search(rowid,source_folded,primary_folded,secondary_folded) VALUES(new.sequence,new.source_folded,new.primary_folded,new.secondary_folded);
                END;
                CREATE TRIGGER history_delete AFTER DELETE ON translation_records BEGIN
                    INSERT INTO translation_search(translation_search,rowid,source_folded,primary_folded,secondary_folded) VALUES('delete',old.sequence,old.source_folded,old.primary_folded,old.secondary_folded);
                END;
            ").map_err(storage_error)?;
            transaction
                .pragma_update(None, "application_id", APPLICATION_ID)
                .map_err(storage_error)?;
            transaction
                .pragma_update(None, "user_version", 1)
                .map_err(storage_error)?;
            transaction.commit().map_err(storage_error)?;
        } else if application != APPLICATION_ID {
            return Err("storage_schema".into());
        }
        let total_records = connection
            .query_row("SELECT count(*) FROM translation_records", [], |row| {
                row.get::<_, i64>(0)
            })
            .map_err(storage_error)?;
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
            tx.execute_batch("ALTER TABLE clipboard_records ADD COLUMN title TEXT NOT NULL DEFAULT ''; PRAGMA user_version=4;").map_err(storage_error)?;
            tx.commit().map_err(storage_error)?;
        }
        clipboard::ensure_filter_indexes(&connection)?;
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
            total_records: total_records as usize,
            clipboard_total_records,
        })
    }

    pub fn page(
        &self,
        query: &HistoryQuery,
        cursor: Option<&HistoryCursor>,
        limit: usize,
    ) -> Result<HistoryPage, String> {
        if !(1..=200).contains(&limit) || query.text.chars().count() > 4096 {
            return Err("invalid_argument".into());
        }
        let needle = query.text.trim().to_lowercase();
        let indexed = needle.chars().count() >= 3 && !needle.contains('\0');
        let search = if needle.is_empty() {
            ""
        } else if indexed {
            " AND r.sequence IN (SELECT rowid FROM translation_search WHERE translation_search MATCH :match) AND (instr(r.source_folded,:needle)>0 OR instr(r.primary_folded,:needle)>0 OR instr(r.secondary_folded,:needle)>0)"
        } else {
            " AND (instr(r.source_folded,:needle)>0 OR instr(r.primary_folded,:needle)>0 OR instr(r.secondary_folded,:needle)>0)"
        };
        let favorites = if query.favorites_only {
            " AND r.is_favorite=1"
        } else {
            ""
        };
        let match_value = if indexed {
            format!("\"{}\"", needle.replace('"', "\"\""))
        } else {
            needle.clone()
        };
        // Broad terms benefit from walking the ordered index and stopping at one page.
        let broad = if indexed {
            let mut probe=self.connection.prepare_cached("SELECT rowid FROM translation_search WHERE translation_search MATCH ?1 LIMIT 1024").map_err(storage_error)?;
            let mut rows = probe.query([&match_value]).map_err(storage_error)?;
            let mut count = 0;
            while rows.next().map_err(storage_error)?.is_some() {
                count += 1;
            }
            count == 1024
        } else {
            false
        };
        let order_hint = if broad {
            if query.favorites_only {
                " INDEXED BY history_favorites"
            } else {
                " INDEXED BY history_order"
            }
        } else {
            ""
        };
        let sql = format!("SELECT {COLUMNS} FROM translation_records r LEFT JOIN source_apps a ON a.id=r.source_app_id WHERE r.sequence IN (SELECT r.sequence FROM translation_records r{order_hint} WHERE (r.is_pinned,r.created_at,r.sequence)<(:pinned,:created,:sequence){favorites}{search} ORDER BY r.is_pinned DESC,r.created_at DESC,r.sequence DESC LIMIT :limit) ORDER BY r.is_pinned DESC,r.created_at DESC,r.sequence DESC");
        let mut statement = self
            .connection
            .prepare_cached(&sql)
            .map_err(storage_error)?;
        let search_value = match_value;
        let mut bindings: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
        let pinned = cursor.map_or(2_i64, |cursor| i64::from(cursor.pinned));
        let sequence = cursor.map_or(i64::MAX, |cursor| cursor.sequence);
        let created = cursor.map_or(i64::MAX, |cursor| cursor.created_at);
        let take = limit as i64 + 1;
        bindings.extend([
            (":pinned", &pinned as &dyn rusqlite::ToSql),
            (":sequence", &sequence),
            (":created", &created),
            (":limit", &take),
        ]);
        if !needle.is_empty() {
            bindings.push((":needle", &needle));
            if indexed {
                bindings.push((":match", &search_value));
            }
        }
        let mut rows = statement
            .query(bindings.as_slice())
            .map_err(storage_error)?;
        let mut records = Vec::with_capacity(limit);
        let mut last = None;
        let mut has_more = false;
        while let Some(row) = rows.next().map_err(storage_error)? {
            if records.len() == limit {
                has_more = true;
                break;
            }
            let record = decode(row)?;
            last = Some(HistoryCursor {
                pinned: record.is_pinned,
                sequence: row.get(0).map_err(storage_error)?,
                created_at: row.get(6).map_err(storage_error)?,
            });
            records.push(record);
        }
        let total_records = if needle.is_empty() && !query.favorites_only {
            self.total_records
        } else {
            let sql =
                format!("SELECT COUNT(*) FROM translation_records r WHERE 1=1{favorites}{search}");
            let mut statement = self
                .connection
                .prepare_cached(&sql)
                .map_err(storage_error)?;
            let mut bindings: Vec<(&str, &dyn rusqlite::ToSql)> = Vec::new();
            if !needle.is_empty() {
                bindings.push((":needle", &needle));
                if indexed {
                    bindings.push((":match", &search_value));
                }
            }
            statement
                .query_row(bindings.as_slice(), |row| row.get::<_, i64>(0))
                .map_err(storage_error)? as usize
        };
        Ok(HistoryPage {
            records,
            cursor: last,
            has_more,
            total_records,
        })
    }

    pub fn record(&self, id: &str) -> Result<Option<HistoryRecord>, String> {
        let sql = format!("SELECT {COLUMNS} FROM translation_records r LEFT JOIN source_apps a ON a.id=r.source_app_id WHERE r.id=?1");
        let mut statement = self
            .connection
            .prepare_cached(&sql)
            .map_err(storage_error)?;
        let mut rows = statement.query([id]).map_err(storage_error)?;
        rows.next().map_err(storage_error)?.map(decode).transpose()
    }

    #[cfg(any(test, feature = "ui-tests"))]
    pub fn load(&self) -> Result<Vec<HistoryRecord>, String> {
        let sql=format!("SELECT {COLUMNS} FROM translation_records r LEFT JOIN source_apps a ON a.id=r.source_app_id ORDER BY r.sequence");
        let mut statement = self.connection.prepare(&sql).map_err(storage_error)?;
        let mut rows = statement.query([]).map_err(storage_error)?;
        let mut records = Vec::new();
        while let Some(row) = rows.next().map_err(storage_error)? {
            records.push(decode(row)?);
        }
        Ok(records)
    }

    fn insert_in(
        transaction: &Transaction<'_>,
        record: &HistoryRecord,
        engine: &str,
    ) -> Result<(), String> {
        let mut stored = record.clone();
        let source_app = stored
            .source_app
            .take()
            .map(|source| serde_json::to_string(&source).map_err(|_| "storage_record"))
            .transpose()?;
        stored.is_favorite = false;
        stored.is_pinned = false;
        let json = serde_json::to_string(&stored).map_err(|_| "storage_record")?;
        let source_id = if let Some(source) = source_app {
            transaction
                .prepare_cached("INSERT OR IGNORE INTO source_apps(payload) VALUES(?1)")
                .map_err(storage_error)?
                .execute([&source])
                .map_err(storage_error)?;
            Some(
                transaction
                    .prepare_cached("SELECT id FROM source_apps WHERE payload=?1")
                    .map_err(storage_error)?
                    .query_row([source], |row| row.get::<_, i64>(0))
                    .map_err(storage_error)?,
            )
        } else {
            None
        };
        transaction.prepare_cached("INSERT INTO translation_records(id,record_json,engine,is_favorite,is_pinned,source_app_id,source_folded,primary_folded,secondary_folded,created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)").map_err(storage_error)?.execute(params![record.id,json,engine,record.is_favorite,record.is_pinned,source_id,record.text.to_lowercase(),record.translations.first().map(|s|s.to_lowercase()).unwrap_or_default(),record.translations.get(1).map(|s|s.to_lowercase()).unwrap_or_default(),chrono::DateTime::parse_from_rfc3339(&record.date).map_err(|_|"storage_record")?.timestamp_millis()]).map_err(storage_error)?;
        Ok(())
    }

    pub fn insert(
        &mut self,
        record: &HistoryRecord,
        engine: &str,
        current: impl FnOnce() -> bool,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        Self::insert_in(&transaction, record, engine)?;
        if !current() {
            return Err("cancelled".into());
        }
        transaction.commit().map_err(storage_error)?;
        self.total_records += 1;
        Ok(())
    }

    pub fn update(
        &mut self,
        previous: &HistoryRecord,
        next: Option<&HistoryRecord>,
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let changed = if let Some(next) = next {
            if next.id != previous.id {
                return Err("storage_record".into());
            }
            transaction.prepare_cached("UPDATE translation_records SET is_favorite=?1,is_pinned=?2 WHERE id=?3 AND is_favorite=?4 AND is_pinned=?5").map_err(storage_error)?.execute(params![next.is_favorite,next.is_pinned,previous.id,previous.is_favorite,previous.is_pinned]).map_err(storage_error)?
        } else {
            if previous.is_favorite || previous.is_pinned {
                return Err("history_protected".into());
            }
            transaction
                .prepare_cached(
                    "DELETE FROM translation_records WHERE id=?1 AND is_favorite=0 AND is_pinned=0",
                )
                .map_err(storage_error)?
                .execute([&previous.id])
                .map_err(storage_error)?
        };
        if changed != 1 {
            return Err("storage_conflict".into());
        }
        transaction.commit().map_err(storage_error)?;
        if next.is_none() {
            self.total_records -= 1;
        }
        Ok(())
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
    fn record(id: &str) -> HistoryRecord {
        HistoryRecord {
            id: id.into(),
            text: "Text\0\n中文 🐋 %_literal".into(),
            source_language: "auto".into(),
            target_languages: vec!["en".into(), "fr".into()],
            translations: vec!["First\r\nsecond".into(), "Texte".into()],
            date: "2026-11-14T12:34:56.789Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: Some(
                serde_json::json!({"bundleId":"test.app", "name":"Synthetic", "iconDataUrl":"data:image/png;base64,synthetic"}),
            ),
        }
    }
    #[test]
    fn full_text_candidates_are_verified_against_literal_content_with_nul() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        let mut item = record("nul-gap");
        item.text = "ab\0cd".into();
        item.translations = vec!["ordinary".into()];
        item.target_languages = vec!["en".into()];
        db.insert(&item, "edge", || true).unwrap();
        assert!(db
            .page(
                &HistoryQuery {
                    text: "abc".into(),
                    favorites_only: false
                },
                None,
                100
            )
            .unwrap()
            .records
            .is_empty());
        assert_eq!(
            db.page(
                &HistoryQuery {
                    text: "b\0c".into(),
                    favorites_only: false
                },
                None,
                100
            )
            .unwrap()
            .records
            .len(),
            1
        );
    }

    #[test]
    fn keyset_pages_are_bounded_complete_and_stable_across_new_inserts() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        let tx = db.connection.transaction().unwrap();
        for index in 0..325 {
            let mut item = record(&format!("page-{index}"));
            item.is_pinned = index < 7;
            seed_in(&tx, &item);
        }
        tx.commit().unwrap();
        db.total_records = 325;
        let query = HistoryQuery::default();
        let first = db.page(&query, None, 100).unwrap();
        assert_eq!(first.records.len(), 100);
        assert!(first.has_more);
        assert_eq!(first.total_records, 325);
        assert!(first.records[..7].iter().all(|r| r.is_pinned));
        let mut ids: std::collections::HashSet<String> =
            first.records.iter().map(|r| r.id.clone()).collect();
        db.insert(&record("added-after-first-page"), "edge", || true)
            .unwrap();
        let mut cursor = first.cursor;
        loop {
            let page = db.page(&query, cursor.as_ref(), 100).unwrap();
            assert!(page.records.len() <= 100);
            for r in page.records {
                assert!(ids.insert(r.id));
            }
            cursor = page.cursor;
            if !page.has_more {
                break;
            }
        }
        assert_eq!(ids.len(), 325);
        assert!(!ids.contains("added-after-first-page"));
        assert_eq!(db.page(&query, None, 100).unwrap().total_records, 326);
        assert!(db.page(&query, None, 201).is_err());
    }

    fn seed_in(transaction: &Transaction<'_>, item: &HistoryRecord) {
        HistoryDatabase::insert_in(transaction, item, "edge").unwrap();
    }

    #[test]
    fn global_literal_search_covers_unicode_quotes_short_queries_and_unloaded_rows() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        for index in 0..120 {
            let mut item = record(&format!("search-{index}"));
            item.text = format!("ordinary {index}");
            item.translations = vec!["ordinary".into(), "result".into()];
            db.insert(&item, "edge", || true).unwrap();
        }
        let mut special = record("older-special");
        special.text = "ÉCO 中文 🐋 100%_literal a\"b zero\0tail".into();
        special.translations = vec!["Only translated needle".into(), "Texte".into()];
        special.is_favorite = true;
        db.insert(&special, "google", || true).unwrap();
        // Another newer page hides this record from the first 100.
        for index in 120..225 {
            let mut item = record(&format!("search-{index}"));
            item.text = format!("ordinary {index}");
            item.translations = vec!["ordinary".into(), "result".into()];
            db.insert(&item, "edge", || true).unwrap();
        }
        assert!(!db
            .page(&HistoryQuery::default(), None, 100)
            .unwrap()
            .records
            .iter()
            .any(|r| r.id == special.id));
        for needle in [
            "éco",
            "中文",
            "🐋",
            "100%_literal",
            "a\"b",
            "zero\0tail",
            "translated needle",
            "Texte",
        ] {
            let page = db
                .page(
                    &HistoryQuery {
                        text: needle.into(),
                        favorites_only: false,
                    },
                    None,
                    100,
                )
                .unwrap();
            assert_eq!(
                page.records
                    .iter()
                    .map(|r| r.id.as_str())
                    .collect::<Vec<_>>(),
                vec!["older-special"],
                "needle={needle:?}"
            );
            assert_eq!(page.total_records, 1, "needle={needle:?}");
        }
        let favorites = db
            .page(
                &HistoryQuery {
                    text: String::new(),
                    favorites_only: true,
                },
                None,
                100,
            )
            .unwrap();
        assert_eq!(favorites.records.len(), 1);
        assert_eq!(favorites.total_records, 1);
        assert!(db
            .page(
                &HistoryQuery {
                    text: "missing literal".into(),
                    favorites_only: false
                },
                None,
                100
            )
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn filtered_totals_cover_all_matching_rows_independent_of_cursor_and_page_size() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        for index in 0..6 {
            let mut item = record(&format!("count-{index}"));
            item.text = "ordinary".into();
            item.translations = vec!["primary".into(), "secondary needle".into()];
            item.is_favorite = index < 3;
            db.insert(&item, "edge", || true).unwrap();
        }
        let query = HistoryQuery {
            text: "secondary needle".into(),
            favorites_only: true,
        };
        let first = db.page(&query, None, 1).unwrap();
        assert_eq!(first.records.len(), 1);
        assert_eq!(first.total_records, 3);
        assert!(first.has_more);
        let second = db.page(&query, first.cursor.as_ref(), 1).unwrap();
        assert_eq!(second.records.len(), 1);
        assert_eq!(second.total_records, 3);
        assert_ne!(first.records[0].id, second.records[0].id);
        let missing = db
            .page(
                &HistoryQuery {
                    text: "absent".into(),
                    favorites_only: false,
                },
                None,
                1,
            )
            .unwrap();
        assert_eq!(missing.total_records, 0);
    }

    #[test]
    fn flag_updates_do_not_rewrite_payload_search_index_or_duplicate_source_icons() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        let mut item = record("large");
        item.text = "large text ".repeat(3000);
        db.insert(&item, "edge", || true).unwrap();
        db.insert(&record("same-app"), "edge", || true).unwrap();
        let before: String = db
            .connection
            .query_row(
                "SELECT record_json FROM translation_records WHERE id='large'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        let icons: i64 = db
            .connection
            .query_row("SELECT count(*) FROM source_apps", [], |r| r.get(0))
            .unwrap();
        assert_eq!(icons, 1);
        let changes = db.connection.total_changes();
        let mut next = item.clone();
        next.is_favorite = true;
        db.update(&item, Some(&next)).unwrap();
        assert_eq!(db.connection.total_changes() - changes, 1);
        let after: String = db
            .connection
            .query_row(
                "SELECT record_json FROM translation_records WHERE id='large'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(before, after);
        assert!(db.record("large").unwrap().unwrap().is_favorite);
        assert!(db.record("missing").unwrap().is_none());
    }

    #[test]
    #[ignore = "explicit isolated 10k/100k storage performance measurement"]
    fn benchmark_large_history() {
        fn milliseconds(start: std::time::Instant) -> f64 {
            start.elapsed().as_secs_f64() * 1000.0
        }
        fn summary(mut values: Vec<f64>) -> serde_json::Value {
            values.sort_by(f64::total_cmp);
            serde_json::json!({"p50Ms":values[values.len()/2],"p95Ms":values[(values.len()*95/100).min(values.len()-1)],"samples":values.len()})
        }
        let mut long = record("long");
        long.text = ("x".repeat(4999) + "\n").repeat(20);
        long.translations = vec![long.text.clone(), long.text.clone()];
        long.target_languages = vec!["en".into(), "fr".into()];
        let store = crate::history::HistoryStore::new(
            (0..100)
                .map(|i| {
                    let mut item = long.clone();
                    item.id = format!("long-{i}");
                    item
                })
                .collect(),
        );
        let mut snapshot_times = Vec::new();
        let mut snapshot_bytes = 0;
        for _ in 0..30 {
            let start = std::time::Instant::now();
            let snapshot = store.snapshot();
            snapshot_bytes = serde_json::to_vec(&snapshot).unwrap().len();
            snapshot_times.push(milliseconds(start));
        }
        assert!(snapshot_bytes < 2_000_000);
        let long_snapshot = summary(snapshot_times);
        let mut reports = Vec::new();
        for size in [10_000, 100_000] {
            let fixture = Fixture::new();
            let mut db = HistoryDatabase::open(&fixture.0).unwrap();
            let mut base = record("bench");
            base.text = "Synthetic translation history performance payload. ".repeat(4);
            base.translations = vec![
                "Synthetic translated output. ".repeat(5),
                "Deuxième traduction synthétique. ".repeat(5),
            ];
            base.source_app = Some(
                serde_json::json!({"name":"Synthetic app","bundleId":"test.benchmark","iconDataUrl":"data:image/png;base64,".to_owned()+&"A".repeat(4096)}),
            );
            let start = std::time::Instant::now();
            let tx = db.connection.transaction().unwrap();
            for index in 0..size {
                let mut item = base.clone();
                item.id = format!("bench-{index}");
                item.text.push_str(&format!(" needle-{index:06}"));
                HistoryDatabase::insert_in(&tx, &item, "edge").unwrap();
            }
            tx.commit().unwrap();
            db.total_records = size;
            let seed_ms = milliseconds(start);
            drop(db);
            let start = std::time::Instant::now();
            let mut db = HistoryDatabase::open(&fixture.0).unwrap();
            let first = db.page(&HistoryQuery::default(), None, 100).unwrap();
            let open_ms = milliseconds(start);
            assert_eq!(first.records.len(), 100);
            let mut reads = Vec::new();
            let mut pages = Vec::new();
            let mut searches = Vec::new();
            let mut broad_searches = Vec::new();
            let mut updates = Vec::new();
            let mut inserts = Vec::new();
            let mut deep = Vec::new();
            let cursor = HistoryCursor {
                pinned: false,
                sequence: (size / 2) as i64,
                created_at: chrono::DateTime::parse_from_rfc3339(&base.date)
                    .unwrap()
                    .timestamp_millis(),
            };
            let mut current = db.record("bench-1").unwrap().unwrap();
            for n in 0..30 {
                let start = std::time::Instant::now();
                assert!(db
                    .record(&format!("bench-{}", size / 2 + n))
                    .unwrap()
                    .is_some());
                reads.push(milliseconds(start));
                let start = std::time::Instant::now();
                assert_eq!(
                    db.page(&HistoryQuery::default(), None, 100)
                        .unwrap()
                        .records
                        .len(),
                    100
                );
                pages.push(milliseconds(start));
                let start = std::time::Instant::now();
                assert_eq!(
                    db.page(&HistoryQuery::default(), Some(&cursor), 100)
                        .unwrap()
                        .records
                        .len(),
                    100
                );
                deep.push(milliseconds(start));
                let start = std::time::Instant::now();
                assert_eq!(
                    db.page(
                        &HistoryQuery {
                            text: format!("needle-{:06}", size - 1),
                            favorites_only: false
                        },
                        None,
                        100
                    )
                    .unwrap()
                    .records
                    .len(),
                    1
                );
                searches.push(milliseconds(start));
                let start = std::time::Instant::now();
                assert_eq!(
                    db.page(
                        &HistoryQuery {
                            text: "Synthetic".into(),
                            favorites_only: false
                        },
                        None,
                        100
                    )
                    .unwrap()
                    .records
                    .len(),
                    100
                );
                broad_searches.push(milliseconds(start));
                let mut next = current.clone();
                next.is_favorite = !next.is_favorite;
                let start = std::time::Instant::now();
                db.update(&current, Some(&next)).unwrap();
                updates.push(milliseconds(start));
                current = next;
                let mut item = base.clone();
                item.id = format!("new-{n}");
                let start = std::time::Instant::now();
                db.insert(&item, "edge", || true).unwrap();
                inserts.push(milliseconds(start));
            }
            let result = serde_json::json!({"records":size,"seedTransactionMs":seed_ms,"openAndFirst100Ms":open_ms,"detailRead":summary(reads),"firstPage100":summary(pages),"deepKeysetPage100":summary(deep),"indexedLiteralSearch":summary(searches),"commonLiteralSearch":summary(broad_searches),"durableFlagWrite":summary(updates),"durableInsert":summary(inserts),"databaseBytes":std::fs::metadata(&fixture.0).unwrap().len(),"sqlite":"WAL + synchronous FULL","iconBytesPerSource":4096,"summary100LongTexts":long_snapshot,"summarySerializedBytes":snapshot_bytes});
            println!("{result}");
            assert!(open_ms < 3000.0, "cold load exceeded 3 seconds");
            for key in [
                "detailRead",
                "firstPage100",
                "deepKeysetPage100",
                "indexedLiteralSearch",
                "commonLiteralSearch",
                "durableFlagWrite",
                "durableInsert",
            ] {
                assert!(
                    result[key]["p95Ms"].as_f64().unwrap() < 100.0,
                    "{key} p95 exceeded 100ms"
                );
            }
            reports.push(result);
        }
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/database-benchmark.json");
        std::fs::write(&path, serde_json::to_vec_pretty(&reports).unwrap()).unwrap();
        println!("Benchmark report: {}", path.display());
    }

    #[test]
    fn restart_restores_complete_records_flags_and_order() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        assert!(db.load().unwrap().is_empty());
        let old = record("first");
        db.insert(&old, "edge", || true).unwrap();
        db.insert(&record("second"), "google", || true).unwrap();
        let mut updated = old.clone();
        updated.is_favorite = true;
        updated.is_pinned = true;
        db.update(&old, Some(&updated)).unwrap();
        drop(db);
        let db = HistoryDatabase::open(&fixture.0).unwrap();
        let records = db.load().unwrap();
        assert_eq!(
            records.iter().map(|r| r.id.as_str()).collect::<Vec<_>>(),
            vec!["first", "second"]
        );
        assert_eq!(
            serde_json::to_value(&records[0]).unwrap(),
            serde_json::to_value(updated).unwrap()
        );
    }
    #[test]
    fn cancelled_duplicate_and_failed_writes_leave_database_unchanged() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        assert_eq!(
            db.insert(&record("cancelled"), "edge", || false),
            Err("cancelled".into())
        );
        db.insert(&record("first"), "edge", || true).unwrap();
        assert!(db.insert(&record("first"), "edge", || true).is_err());
        assert!(db
            .insert(&record("bad-engine"), "invalid", || true)
            .is_err());
        db.connection
            .pragma_update(None, "query_only", true)
            .unwrap();
        assert!(db.insert(&record("read-only"), "edge", || true).is_err());
        assert_eq!(db.load().unwrap().len(), 1);
    }
    #[test]
    fn protected_delete_and_conflicting_updates_preserve_saved_data() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        let old = record("one");
        db.insert(&old, "edge", || true).unwrap();
        let mut pinned = old.clone();
        pinned.is_pinned = true;
        db.update(&old, Some(&pinned)).unwrap();
        assert_eq!(db.update(&old, None), Err("storage_conflict".into()));
        assert_eq!(db.update(&pinned, None), Err("history_protected".into()));
        let mut favorite = old.clone();
        favorite.is_favorite = true;
        assert_eq!(db.update(&pinned, None), Err("history_protected".into()));
        assert_eq!(
            db.update(&old, Some(&favorite)),
            Err("storage_conflict".into())
        );
        db.update(&pinned, Some(&old)).unwrap();
        db.update(&old, None).unwrap();
        drop(db);
        assert!(HistoryDatabase::open(&fixture.0)
            .unwrap()
            .load()
            .unwrap()
            .is_empty());
    }
    #[test]
    fn future_foreign_and_corrupt_databases_are_not_replaced() {
        for kind in ["future", "foreign", "corrupt", "payload"] {
            let fixture = Fixture::new();
            if kind == "corrupt" {
                std::fs::write(&fixture.0, b"not a sqlite database").unwrap();
            } else {
                let db = HistoryDatabase::open(&fixture.0).unwrap();
                match kind {
                    "future" => db
                        .connection
                        .pragma_update(None, "user_version", VERSION + 1)
                        .unwrap(),
                    "foreign" => db
                        .connection
                        .pragma_update(None, "application_id", 42)
                        .unwrap(),
                    _ => {
                        let mut db = db;
                        db.insert(&record("bad"), "edge", || true).unwrap();
                        db.connection.execute("UPDATE translation_records SET record_json='not json' WHERE id='bad'", []).unwrap();
                    }
                }
            }
            let before = std::fs::read(&fixture.0).unwrap();
            assert!(HistoryDatabase::open(&fixture.0)
                .and_then(|db| db.page(&HistoryQuery::default(), None, PAGE_SIZE))
                .is_err());
            assert_eq!(std::fs::read(&fixture.0).unwrap(), before);
        }
    }
    #[test]
    fn locked_database_returns_error_without_partial_insert() {
        let fixture = Fixture::new();
        let mut db = HistoryDatabase::open(&fixture.0).unwrap();
        db.connection
            .busy_timeout(Duration::from_millis(10))
            .unwrap();
        let blocker = Connection::open(&fixture.0).unwrap();
        blocker.execute_batch("BEGIN IMMEDIATE").unwrap();
        assert_eq!(
            db.insert(&record("locked"), "edge", || true),
            Err("storage_busy".into())
        );
        blocker.execute_batch("ROLLBACK").unwrap();
        assert!(db.load().unwrap().is_empty());
    }
}
