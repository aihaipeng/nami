use super::retention::{check_cleanup, clipboard_cleanup_sequences, persist_limit, CleanupContext};
use super::{storage_error, HistoryDatabase};
use crate::clipboard::{
    valid_image_name, ClipboardCursor, ClipboardFileRepresentation, ClipboardPage, ClipboardRecord,
    ClipboardSearchMode, ClipboardSearchOptions, ClipboardTextFormatting, HistoryPolicy,
    MAX_FILE_COUNT, MAX_RICH_TEXT_BYTES, MAX_TEXT_BYTES,
};
use crate::model::RecordView;
use rusqlite::{
    functions::FunctionFlags, params, Connection, OptionalExtension, Row, TransactionBehavior,
};
use std::{
    cell::OnceCell,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

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

pub(super) fn migrate_v5(connection: &mut rusqlite::Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch(
        "CREATE TABLE clipboard_file_representations (
        sequence INTEGER PRIMARY KEY REFERENCES clipboard_records(sequence) ON DELETE CASCADE,
        paths TEXT NOT NULL, image_path TEXT NOT NULL
    );
    CREATE INDEX clipboard_image_resources ON clipboard_records(image_path) WHERE image_path IS NOT NULL;
    CREATE INDEX clipboard_file_image_resources ON clipboard_file_representations(image_path);
    PRAGMA user_version=5;",
    )
    .map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

pub(super) fn migrate_v6(connection: &mut rusqlite::Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch("CREATE TABLE clipboard_titles (
        sequence INTEGER PRIMARY KEY REFERENCES clipboard_records(sequence) ON DELETE CASCADE,
        search_folded TEXT NOT NULL
    );
    CREATE VIRTUAL TABLE clipboard_title_search USING fts5(search_folded,content='clipboard_titles',content_rowid='sequence',tokenize='trigram case_sensitive 1');
    CREATE TRIGGER clipboard_title_insert AFTER INSERT ON clipboard_titles BEGIN
        INSERT INTO clipboard_title_search(rowid,search_folded) VALUES(new.sequence,new.search_folded);
    END;
    CREATE TRIGGER clipboard_title_delete AFTER DELETE ON clipboard_titles BEGIN
        INSERT INTO clipboard_title_search(clipboard_title_search,rowid,search_folded) VALUES('delete',old.sequence,old.search_folded);
    END;")
    .map_err(storage_error)?;
    {
        let mut source = tx
            .prepare("SELECT sequence,title FROM clipboard_records WHERE title!=''")
            .map_err(storage_error)?;
        let mut rows = source.query([]).map_err(storage_error)?;
        let mut insert = tx
            .prepare("INSERT INTO clipboard_titles(sequence,search_folded) VALUES(?1,?2)")
            .map_err(storage_error)?;
        while let Some(row) = rows.next().map_err(storage_error)? {
            insert
                .execute(params![
                    row.get::<_, i64>(0).map_err(storage_error)?,
                    row.get::<_, String>(1)
                        .map_err(storage_error)?
                        .to_lowercase()
                ])
                .map_err(storage_error)?;
        }
    }
    tx.pragma_update(None, "user_version", 6)
        .map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

pub(super) fn migrate_v7(connection: &mut Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch("CREATE TABLE clipboard_ocr (
        sequence INTEGER PRIMARY KEY REFERENCES clipboard_records(sequence) ON DELETE CASCADE,
        search_folded TEXT NOT NULL
    );
    CREATE VIRTUAL TABLE clipboard_ocr_search USING fts5(search_folded,content='clipboard_ocr',content_rowid='sequence',tokenize='trigram case_sensitive 1');
    CREATE TRIGGER clipboard_ocr_insert AFTER INSERT ON clipboard_ocr BEGIN
        INSERT INTO clipboard_ocr_search(rowid,search_folded) VALUES(new.sequence,new.search_folded);
    END;
    CREATE TRIGGER clipboard_ocr_delete AFTER DELETE ON clipboard_ocr BEGIN
        INSERT INTO clipboard_ocr_search(clipboard_ocr_search,rowid,search_folded) VALUES('delete',old.sequence,old.search_folded);
    END;
    CREATE TRIGGER clipboard_ocr_update AFTER UPDATE OF search_folded ON clipboard_ocr
    WHEN old.search_folded != new.search_folded BEGIN
        INSERT INTO clipboard_ocr_search(clipboard_ocr_search,rowid,search_folded) VALUES('delete',old.sequence,old.search_folded);
        INSERT INTO clipboard_ocr_search(rowid,search_folded) VALUES(new.sequence,new.search_folded);
    END;
    PRAGMA user_version=7;")
    .map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

pub(super) fn migrate_v8(connection: &mut Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch(&format!("CREATE TABLE clipboard_text_formatting (
        sequence INTEGER PRIMARY KEY REFERENCES clipboard_records(sequence) ON DELETE CASCADE,
        html BLOB CHECK(html IS NULL OR (typeof(html)='blob' AND length(html) BETWEEN 1 AND {MAX_RICH_TEXT_BYTES})),
        rtf BLOB CHECK(rtf IS NULL OR (typeof(rtf)='blob' AND length(rtf) BETWEEN 1 AND {MAX_RICH_TEXT_BYTES})),
        CHECK(html IS NOT NULL OR rtf IS NOT NULL)
    ); PRAGMA user_version=8;")).map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

fn file_search_name(path: &str) -> &str {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
}

pub(super) fn migrate_v9(connection: &mut Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch("ALTER TABLE clipboard_ocr ADD COLUMN original_text TEXT;
        CREATE TRIGGER IF NOT EXISTS clipboard_search_update AFTER UPDATE OF search_folded ON clipboard_contents
        WHEN old.search_folded != new.search_folded BEGIN
            INSERT INTO clipboard_search(clipboard_search,rowid,search_folded) VALUES('delete',old.sequence,old.search_folded);
            INSERT INTO clipboard_search(rowid,search_folded) VALUES(new.sequence,new.search_folded);
        END; PRAGMA user_version=9;").map_err(storage_error)?;
    {
        let mut query = tx.prepare("SELECT r.sequence,f.paths FROM clipboard_records r JOIN clipboard_file_representations f USING(sequence) WHERE r.kind='image' AND octet_length(f.paths)<=?1").map_err(storage_error)?;
        let mut rows = query
            .query([MAX_TEXT_BYTES as i64])
            .map_err(storage_error)?;
        while let Some(row) = rows.next().map_err(storage_error)? {
            let sequence: i64 = row.get(0).map_err(storage_error)?;
            let paths: String = row.get(1).map_err(storage_error)?;
            let Ok(paths) = serde_json::from_str::<Vec<String>>(&paths) else {
                continue;
            };
            let [path] = paths.as_slice() else { continue };
            if !std::path::Path::new(path).is_absolute() || path.contains('\0') {
                continue;
            }
            tx.execute(
                "UPDATE clipboard_contents SET search_folded=?2 WHERE sequence=?1",
                params![sequence, file_search_name(path).to_lowercase()],
            )
            .map_err(storage_error)?;
        }
    }
    tx.commit().map_err(storage_error)
}

pub(super) fn migrate_v10(connection: &mut Connection) -> Result<(), String> {
    let tx = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS clipboard_pending_image_deletions (image_path TEXT PRIMARY KEY NOT NULL);
         PRAGMA user_version=10;",
    )
    .map_err(storage_error)?;
    tx.commit().map_err(storage_error)
}

fn save_ocr(
    connection: &Connection,
    id: &str,
    fingerprint: Option<&str>,
    text: &str,
) -> Result<bool, String> {
    if text.contains('\0') || text.len() > MAX_TEXT_BYTES {
        return Err("invalid_argument".into());
    }
    let updated = connection.execute(
        "INSERT INTO clipboard_ocr(sequence,search_folded,original_text)
         SELECT r.sequence,?3,CASE WHEN r.kind='image' THEN ?4 END
         FROM clipboard_records r JOIN clipboard_contents c USING(sequence)
         WHERE r.id=?1 AND (r.kind='image' OR (?2 IS NULL AND r.kind='files' AND json_array_length(c.text)=1))
         AND (?2 IS NULL OR (r.fingerprint=?2 AND NOT EXISTS (SELECT 1 FROM clipboard_ocr o WHERE o.sequence=r.sequence AND o.original_text IS NOT NULL)))
         ON CONFLICT(sequence) DO UPDATE SET search_folded=excluded.search_folded,original_text=excluded.original_text",
        params![id, fingerprint, text.to_lowercase(), text],
    ).map_err(storage_error)?;
    Ok(updated != 0)
}

fn valid_file_representation(
    record: &ClipboardRecord,
    representation: &ClipboardFileRepresentation,
) -> bool {
    !representation.paths.is_empty()
        && representation.paths.len() <= MAX_FILE_COUNT
        && representation
            .paths
            .iter()
            .all(|path| std::path::Path::new(path).is_absolute() && !path.contains('\0'))
        && valid_image_name(&representation.image_path)
        && match record.kind.as_str() {
            "image" => {
                representation.paths.len() == 1
                    && record.image_path.as_ref() == Some(&representation.image_path)
            }
            "files" => serde_json::from_str::<Vec<String>>(&record.text)
                .is_ok_and(|paths| paths == representation.paths),
            _ => false,
        }
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

fn image_referenced(connection: &Connection, path: &str) -> Result<bool, String> {
    connection.prepare_cached(
        "SELECT EXISTS(SELECT 1 FROM clipboard_records WHERE image_path=?1 UNION ALL SELECT 1 FROM clipboard_file_representations WHERE image_path=?1)",
    ).map_err(storage_error)?.query_row([path], |row| row.get(0)).map_err(storage_error)
}

pub fn unlink_pending_clipboard_images(
    root: &std::path::Path,
    pending: Vec<(String, bool)>,
) -> (Vec<String>, bool) {
    let count = pending.len();
    let complete: Vec<_> = pending
        .into_iter()
        .filter_map(|(path, referenced)| {
            (referenced || crate::clipboard::remove_image(root, &path).is_ok()).then_some(path)
        })
        .collect();
    let failed = complete.len() != count;
    (complete, failed)
}

pub(super) fn ensure_filter_indexes(connection: &rusqlite::Connection) -> Result<(), String> {
    connection.execute_batch("CREATE INDEX IF NOT EXISTS clipboard_kind_order ON clipboard_records(kind,is_pinned DESC,created_at DESC,sequence DESC);
        CREATE INDEX IF NOT EXISTS clipboard_kind_favorites ON clipboard_records(kind,is_favorite,is_pinned DESC,created_at DESC,sequence DESC);").map_err(storage_error)
}

// Non-indexed modes use a bounded query-local matcher without retaining user input in the DB.
const ADVANCED_MATCH: &str = "clipboard_advanced_match";

enum AdvancedMatch {
    Fuzzy(Vec<char>),
    Regex(regex::Regex),
}

impl AdvancedMatch {
    fn is_match(&self, text: &str) -> bool {
        match self {
            Self::Fuzzy(needle) => {
                let mut chars = text.chars();
                needle.iter().all(|&wanted| chars.any(|ch| ch == wanted))
            }
            Self::Regex(regex) => regex.is_match(text),
        }
    }
}

#[derive(Clone, Copy)]
struct ClipboardPageScope<'a> {
    favorites_only: bool,
    kind: Option<&'a str>,
    cursor: Option<&'a ClipboardCursor>,
    limit: usize,
    pinned: Option<bool>,
}

struct AdvancedSearchBudget {
    remaining: AtomicUsize,
    deadline: Instant,
    exhausted: AtomicBool,
}

impl Default for AdvancedSearchBudget {
    fn default() -> Self {
        Self {
            remaining: AtomicUsize::new(128 * 1024 * 1024),
            deadline: Instant::now() + Duration::from_secs(2),
            exhausted: AtomicBool::new(false),
        }
    }
}

struct ClipboardSearch<'a> {
    needle: &'a str,
    options: ClipboardSearchOptions,
    advanced: bool,
}

impl ClipboardSearch<'_> {
    // These identifiers and expressions are internal SQL, never user input.
    fn projections(&self) -> impl Iterator<Item = (&'static str, &'static str, &'static str)> {
        [
            (self.options.body, "clipboard_contents", "clipboard_search", "CASE WHEN EXISTS (SELECT 1 FROM clipboard_records b WHERE b.sequence=p.sequence AND b.kind='text') THEN p.text ELSE p.search_folded END"),
            (self.options.note, "clipboard_titles", "clipboard_title_search", "(SELECT n.title FROM clipboard_records n WHERE n.sequence=p.sequence)"),
            (self.options.image_text, "clipboard_ocr", "clipboard_ocr_search", "COALESCE(p.original_text,p.search_folded)"),
        ].into_iter().filter_map(|(enabled, table, index, original)| enabled.then_some((table, index, original)))
    }

    fn predicate(&self, original: &str) -> String {
        match self.options.mode {
            ClipboardSearchMode::Exact => "instr(p.search_folded,:needle)>0".into(),
            ClipboardSearchMode::Fuzzy => format!("{ADVANCED_MATCH}(p.search_folded)"),
            ClipboardSearchMode::Regex => format!("{ADVANCED_MATCH}({original})"),
            ClipboardSearchMode::Mixed => unreachable!(),
        }
    }
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
        self.clipboard_page_with_search(
            query,
            favorites_only,
            kind,
            cursor,
            limit,
            ClipboardSearchOptions::default(),
        )
    }

    pub fn clipboard_page_with_search(
        &self,
        query: &str,
        favorites_only: bool,
        kind: Option<&str>,
        cursor: Option<&ClipboardCursor>,
        limit: usize,
        options: ClipboardSearchOptions,
    ) -> Result<ClipboardPage, String> {
        if query.chars().count() > 4096 || !(1..=200).contains(&limit) || !options.has_scope() {
            return Err("invalid_argument".into());
        }
        if kind.is_some_and(|kind| !["text", "image", "files"].contains(&kind)) {
            return Err("invalid_argument".into());
        }
        let query = query.trim();
        let scope = ClipboardPageScope {
            favorites_only,
            kind,
            cursor,
            limit,
            pinned: None,
        };
        let budget = OnceCell::new();
        if options.mode != ClipboardSearchMode::Mixed || query.is_empty() {
            let mode = if query.is_empty() {
                ClipboardSearchMode::Exact
            } else {
                options.mode
            };
            return self.clipboard_page_for_mode(
                query,
                ClipboardSearchOptions { mode, ..options },
                scope,
                &budget,
            );
        }

        // Resolve the rule before slicing by cursor, independently for each visual section.
        let section = |scope| {
            for mode in [ClipboardSearchMode::Exact, ClipboardSearchMode::Regex] {
                match self.clipboard_page_for_mode(
                    query,
                    ClipboardSearchOptions { mode, ..options },
                    scope,
                    &budget,
                ) {
                    Ok(page) if page.total_records > 0 => return Ok(page),
                    Ok(_) => continue,
                    Err(error)
                        if mode == ClipboardSearchMode::Regex
                            && error == "clipboard_invalid_regex" =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
            self.clipboard_page_for_mode(
                query,
                ClipboardSearchOptions {
                    mode: ClipboardSearchMode::Fuzzy,
                    ..options
                },
                scope,
                &budget,
            )
        };
        let mut pinned = section(ClipboardPageScope {
            pinned: Some(true),
            ..scope
        })?;
        let remaining = limit - pinned.records.len();
        let ordinary = section(ClipboardPageScope {
            pinned: Some(false),
            limit: remaining.max(1),
            ..scope
        })?;
        pinned.total_records += ordinary.total_records;
        if remaining == 0 {
            pinned.has_more |= !ordinary.records.is_empty();
        } else {
            pinned.has_more |= ordinary.has_more;
            pinned.cursor = ordinary.cursor.or(pinned.cursor);
            pinned.records.extend(ordinary.records);
            pinned.sources.extend(ordinary.sources);
        }
        Ok(pinned)
    }

    fn clipboard_page_for_mode(
        &self,
        query: &str,
        options: ClipboardSearchOptions,
        scope: ClipboardPageScope<'_>,
        budget: &OnceCell<Arc<AdvancedSearchBudget>>,
    ) -> Result<ClipboardPage, String> {
        let folded = (options.mode != ClipboardSearchMode::Regex).then(|| query.to_lowercase());
        let needle = folded.as_deref().unwrap_or(query);
        let search = ClipboardSearch {
            needle,
            options,
            advanced: !query.is_empty() && options.mode != ClipboardSearchMode::Exact,
        };
        if !search.advanced {
            return self.query_clipboard_page(&search, scope);
        }
        let matcher = match options.mode {
            ClipboardSearchMode::Fuzzy => AdvancedMatch::Fuzzy(needle.chars().collect()),
            ClipboardSearchMode::Regex => AdvancedMatch::Regex(
                regex::RegexBuilder::new(needle)
                    .size_limit(1024 * 1024)
                    .dfa_size_limit(1024 * 1024)
                    .build()
                    .map_err(|_| "clipboard_invalid_regex")?,
            ),
            ClipboardSearchMode::Exact | ClipboardSearchMode::Mixed => unreachable!(),
        };
        let budget = budget.get_or_init(|| Arc::new(AdvancedSearchBudget::default()));
        let shared_budget = Arc::clone(budget);
        self.connection
            .create_scalar_function(
                ADVANCED_MATCH,
                1,
                FunctionFlags::SQLITE_UTF8 | FunctionFlags::SQLITE_DIRECTONLY,
                move |ctx| {
                    let text = ctx
                        .get_raw(0)
                        .as_str()
                        .map_err(|_| rusqlite::Error::InvalidQuery)?;
                    let remaining = shared_budget.remaining.load(Ordering::Relaxed);
                    if text.len() > remaining || Instant::now() >= shared_budget.deadline {
                        shared_budget.exhausted.store(true, Ordering::Relaxed);
                        return Err(rusqlite::Error::InvalidQuery);
                    }
                    shared_budget
                        .remaining
                        .store(remaining - text.len(), Ordering::Relaxed);
                    Ok(matcher.is_match(text))
                },
            )
            .map_err(storage_error)?;
        let result = self.query_clipboard_page(&search, scope);
        self.connection
            .remove_function(ADVANCED_MATCH, 1)
            .map_err(storage_error)?;
        if budget.exhausted.load(Ordering::Relaxed) {
            return Err("clipboard_search_too_broad".into());
        }
        result
    }

    fn query_clipboard_page(
        &self,
        search: &ClipboardSearch<'_>,
        scope: ClipboardPageScope<'_>,
    ) -> Result<ClipboardPage, String> {
        let ClipboardPageScope {
            favorites_only,
            kind,
            cursor,
            limit,
            pinned,
        } = scope;
        let needle = search.needle;
        let kind_filter = if kind.is_some() {
            " AND kind=:kind"
        } else {
            ""
        };
        let indexed = !search.advanced && needle.chars().count() >= 3 && !needle.contains('\0');
        let favorite = if favorites_only {
            " AND is_favorite=1"
        } else {
            ""
        };
        let pin = match pinned {
            Some(true) => " AND is_pinned=1",
            Some(false) => " AND is_pinned=0",
            None => "",
        };
        let unfiltered = !favorites_only && kind.is_none() && pinned.is_none();
        let matched = format!("\"{}\"", needle.replace('"', "\"\""));
        let candidates = if indexed {
            search
                .projections()
                .map(|(_, index, _)| {
                    format!("SELECT rowid FROM {index} WHERE {index} MATCH :match")
                })
                .collect::<Vec<_>>()
                .join(" UNION ALL ")
        } else {
            String::new()
        };
        let broad = if indexed {
            let mut probe = self
                .connection
                .prepare_cached(&format!("{candidates} LIMIT 1024"))
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
        let mut filter = String::new();
        if !needle.is_empty() {
            if indexed && !broad {
                filter.push_str(&format!(" AND sequence IN ({candidates})"));
            }
            let literals = search.projections().map(|(table, _, original)| format!("EXISTS (SELECT 1 FROM {table} p WHERE p.sequence=clipboard_records.sequence AND {})", search.predicate(original))).collect::<Vec<_>>().join(" OR ");
            filter.push_str(&format!(" AND ({literals})"));
        }
        let image_text_match = if !needle.is_empty() && search.options.image_text {
            format!(
                "EXISTS (SELECT 1 FROM clipboard_ocr p WHERE p.sequence=r.sequence AND {})",
                search.predicate("COALESCE(p.original_text,p.search_folded)")
            )
        } else {
            "0".into()
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
        let sql = format!("SELECT r.sequence,r.id,r.kind,r.summary,r.date,r.is_favorite,r.is_pinned,r.source_app_id,r.created_at,r.title,{image_text_match},r.kind='image' AND EXISTS (SELECT 1 FROM clipboard_file_representations f WHERE f.sequence=r.sequence),r.kind='text' AND EXISTS (SELECT 1 FROM clipboard_text_formatting t WHERE t.sequence=r.sequence) FROM clipboard_records r WHERE r.sequence IN (SELECT sequence FROM clipboard_records{hint} WHERE (is_pinned,created_at,sequence)<(:pinned,:created,:sequence){favorite}{kind_filter}{pin}{filter} ORDER BY is_pinned DESC,created_at DESC,sequence DESC LIMIT :limit) ORDER BY r.is_pinned DESC,r.created_at DESC,r.sequence DESC");
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
            if !search.advanced {
                filters.push((":needle", &needle));
            }
            if indexed && !broad {
                filters.push((":match", &matched));
            }
        }
        let total_records = if needle.is_empty() && unfiltered {
            self.clipboard_total_records
        } else {
            let matches = (!needle.is_empty() && !search.advanced).then(|| search.projections().map(|(table, index, original)| {
                let predicate = search.predicate(original);
                if indexed {
                    format!("SELECT p.sequence FROM {index} JOIN {table} p ON p.sequence={index}.rowid WHERE {index} MATCH :match AND {predicate}")
                } else {
                    format!("SELECT p.sequence FROM {table} p WHERE {predicate}")
                }
            }).collect::<Vec<_>>().join(" UNION ALL "));
            let mut count_filters = filters.clone();
            if indexed && broad {
                count_filters.push((":match", &matched));
            }
            let sql = match matches {
                Some(matches) if unfiltered =>
                    format!("SELECT count(DISTINCT sequence) FROM ({matches})"),
                Some(matches) => format!("SELECT count(*) FROM clipboard_records WHERE sequence IN ({matches}){favorite}{kind_filter}{pin}"),
                None => format!("SELECT count(*) FROM clipboard_records WHERE 1=1{favorite}{kind_filter}{pin}{filter}"),
            };
            self.connection
                .prepare_cached(&sql)
                .map_err(storage_error)?
                .query_row(count_filters.as_slice(), |row| row.get::<_, i64>(0))
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
                image_text_match: row.get(10).map_err(storage_error)?,
                has_file_paths: row.get(11).map_err(storage_error)?,
                has_rich_text: row.get(12).map_err(storage_error)?,
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
        self.clipboard_insert_with_file_representation(record, None)
    }

    pub fn clipboard_insert_with_file_representation(
        &mut self,
        record: &ClipboardRecord,
        representation: Option<&ClipboardFileRepresentation>,
    ) -> Result<ClipboardRecord, String> {
        self.clipboard_insert_capture(record, representation, None)
    }

    pub fn clipboard_insert_capture(
        &mut self,
        record: &ClipboardRecord,
        representation: Option<&ClipboardFileRepresentation>,
        formatting: Option<&ClipboardTextFormatting>,
    ) -> Result<ClipboardRecord, String> {
        self.insert_clipboard_record(record, representation, formatting, true, None)
    }

    pub fn clipboard_text_formatting(
        &self,
        id: &str,
    ) -> Result<Option<ClipboardTextFormatting>, String> {
        let stored = self.connection.query_row(
            "SELECT f.html,f.rtf FROM clipboard_text_formatting f JOIN clipboard_records r USING(sequence) WHERE r.id=?1 AND r.kind='text'",
            [id], |row| Ok(ClipboardTextFormatting { html: row.get(0)?, rtf: row.get(1)? }),
        ).optional().map_err(storage_error)?;
        if stored.as_ref().is_some_and(|value| !value.valid()) {
            return Err("storage_corrupt".into());
        }
        Ok(stored)
    }

    pub fn clipboard_ocr_text(&self, id: &str) -> Result<Option<String>, String> {
        self.connection.query_row("SELECT o.original_text FROM clipboard_ocr o JOIN clipboard_records r USING(sequence) WHERE r.id=?1 AND r.kind='image' AND o.original_text IS NOT NULL", [id], |row| row.get(0)).optional().map_err(storage_error)
    }

    pub fn clipboard_index_image(
        &mut self,
        id: &str,
        fingerprint: &str,
        text: &str,
    ) -> Result<bool, String> {
        save_ocr(&self.connection, id, Some(fingerprint), text)
    }

    /// Saves the existing extracted text record and indexes its source as one change.
    pub fn clipboard_insert_ocr_result(
        &mut self,
        source_id: &str,
        record: &ClipboardRecord,
    ) -> Result<ClipboardRecord, String> {
        if record.kind != "text" || record.text.trim().is_empty() || record.text.contains('\0') {
            return Err("invalid_argument".into());
        }
        self.insert_clipboard_record(record, None, None, true, Some(source_id))
    }

    pub fn clipboard_insert_translation_copy(&mut self, text: String) -> Result<(), String> {
        let record = ClipboardRecord {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "text".into(),
            fingerprint: crate::clipboard::fingerprint("text", text.as_bytes()),
            text,
            date: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            is_favorite: false,
            is_pinned: false,
            source_app: Some(serde_json::json!({"name":"Nami"})),
            image_path: None,
        };
        self.insert_clipboard_record(&record, None, None, false, None)
            .map(|_| ())
    }

    pub fn clipboard_duplicate(&mut self, id: &str) -> Result<(), String> {
        self.duplicate_clipboard_record(id, false)
    }

    pub fn clipboard_duplicate_plain_text(&mut self, id: &str) -> Result<(), String> {
        self.duplicate_clipboard_record(id, true)
    }

    fn duplicate_clipboard_record(&mut self, id: &str, plain_text: bool) -> Result<(), String> {
        let mut record = self.clipboard_record(id)?;
        if plain_text && record.kind != "text" {
            return Err("invalid_argument".into());
        }
        let representation = if plain_text {
            None
        } else {
            self.clipboard_file_representation(id)?
        };
        let formatting = if plain_text {
            None
        } else {
            self.clipboard_text_formatting(id)?
        };
        record.id = uuid::Uuid::new_v4().to_string();
        record.date = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        record.is_favorite = false;
        record.is_pinned = false;
        if plain_text {
            record.fingerprint = crate::clipboard::fingerprint("text", record.text.as_bytes());
        }
        self.insert_clipboard_record(
            &record,
            representation.as_ref(),
            formatting.as_ref(),
            false,
            None,
        )?;
        Ok(())
    }

    fn insert_clipboard_record(
        &mut self,
        record: &ClipboardRecord,
        representation: Option<&ClipboardFileRepresentation>,
        formatting: Option<&ClipboardTextFormatting>,
        deduplicate: bool,
        ocr_source_id: Option<&str>,
    ) -> Result<ClipboardRecord, String> {
        if record.id.is_empty()
            || !["text", "image", "files"].contains(&record.kind.as_str())
            || record.text.len() > MAX_TEXT_BYTES
            || formatting.is_some_and(|value| {
                record.kind != "text" || record.text.trim().is_empty() || !value.valid()
            })
            || record.fingerprint.is_empty()
            || chrono::DateTime::parse_from_rfc3339(&record.date).is_err()
            || (record.kind == "image") != record.image_path.is_some()
        {
            return Err("invalid_argument".into());
        }
        let paths = representation
            .map(|representation| {
                if !valid_file_representation(record, representation) {
                    return Err("invalid_argument");
                }
                let paths =
                    serde_json::to_string(&representation.paths).map_err(|_| "invalid_argument")?;
                if paths.len() > MAX_TEXT_BYTES {
                    return Err("invalid_argument");
                }
                Ok(paths)
            })
            .transpose()?;
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
                .map(|path| file_search_name(path))
                .collect::<Vec<_>>()
                .join("\n");
            (summary, paths.join("\n"))
        } else {
            let search = representation
                .filter(|_| record.kind == "image")
                .map_or_else(
                    || record.text.clone(),
                    |file| file_search_name(&file.paths[0]).to_owned(),
                );
            (record.text.clone(), search)
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
        if let Some(ocr_source_id) = ocr_source_id {
            if !save_ocr(&transaction, ocr_source_id, None, &record.text)? {
                return Err("invalid_argument".into());
            }
        }
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
        let html = formatting.and_then(|value| value.html.as_deref());
        let rtf = formatting.and_then(|value| value.rtf.as_deref());
        let duplicate: Option<String> = if deduplicate {
            transaction.query_row("SELECT r.id FROM clipboard_records r LEFT JOIN clipboard_text_formatting f USING(sequence) WHERE r.fingerprint=?1 AND r.kind=?2 AND ((r.kind='image' AND r.image_path IS ?4) OR (r.kind!='image' AND EXISTS (SELECT 1 FROM clipboard_contents c WHERE c.sequence=r.sequence AND c.text=?3))) AND f.html IS ?5 AND f.rtf IS ?6 ORDER BY r.sequence DESC LIMIT 1", params![record.fingerprint,record.kind,record.text,record.image_path,html,rtf], |row| row.get(0)).optional().map_err(storage_error)?
        } else {
            None
        };
        let inserted = duplicate.is_none();
        let id = if let Some(id) = duplicate {
            transaction
                .execute(
                    "UPDATE clipboard_records SET date=?1,source_app_id=?2,created_at=CASE WHEN is_pinned THEN MAX(?4,COALESCE((SELECT MAX(created_at) FROM clipboard_records WHERE is_pinned=1),0)+1) ELSE ?4 END WHERE id=?3",
                    params![record.date, source_id, id, created],
                )
                .map_err(storage_error)?;
            if record.kind == "image" {
                transaction.execute("UPDATE clipboard_contents SET search_folded=?2 WHERE sequence=(SELECT sequence FROM clipboard_records WHERE id=?1) AND search_folded<>?2", params![id,search_text.to_lowercase()]).map_err(storage_error)?;
            }
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
        if let (Some(representation), Some(paths)) = (representation, paths) {
            transaction.execute("INSERT INTO clipboard_file_representations(sequence,paths,image_path) VALUES((SELECT sequence FROM clipboard_records WHERE id=?1),?2,?3) ON CONFLICT(sequence) DO UPDATE SET paths=excluded.paths,image_path=excluded.image_path", params![id,paths,representation.image_path]).map_err(storage_error)?;
        } else {
            transaction.execute("DELETE FROM clipboard_file_representations WHERE sequence=(SELECT sequence FROM clipboard_records WHERE id=?1)", [&id]).map_err(storage_error)?;
        }
        if formatting.is_some() && inserted {
            transaction.execute("INSERT INTO clipboard_text_formatting(sequence,html,rtf) VALUES((SELECT sequence FROM clipboard_records WHERE id=?1),?2,?3)", params![id,html,rtf]).map_err(storage_error)?;
        }
        transaction.commit().map_err(storage_error)?;
        if inserted {
            self.clipboard_total_records += 1;
        }
        self.clipboard_record(&id)
    }

    pub fn clipboard_file_representation(
        &self,
        id: &str,
    ) -> Result<Option<ClipboardFileRepresentation>, String> {
        let stored: Option<(String, String)> = self.connection.query_row(
            "SELECT f.paths,f.image_path FROM clipboard_file_representations f JOIN clipboard_records r USING(sequence) WHERE r.id=?1",
            [id], |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional().map_err(storage_error)?;
        let Some((paths, image_path)) = stored else {
            return Ok(None);
        };
        if paths.len() > MAX_TEXT_BYTES {
            return Err("storage_corrupt".into());
        }
        let representation = ClipboardFileRepresentation {
            paths: serde_json::from_str(&paths).map_err(|_| "storage_corrupt")?,
            image_path,
        };
        let record = self.clipboard_record(id)?;
        if !valid_file_representation(&record, &representation) {
            return Err("storage_corrupt".into());
        }
        Ok(Some(representation))
    }

    pub fn clipboard_image_referenced(&self, path: &str) -> Result<bool, String> {
        image_referenced(&self.connection, path)
    }

    pub fn pending_clipboard_image_deletions(&self) -> Result<Vec<(String, bool)>, String> {
        let mut query = self
            .connection
            .prepare("SELECT image_path FROM clipboard_pending_image_deletions")
            .map_err(storage_error)?;
        let rows = query
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(storage_error)?;
        rows.map(|row| {
            let path = row.map_err(storage_error)?;
            let referenced = self.clipboard_image_referenced(&path)?;
            Ok((path, referenced))
        })
        .collect()
    }

    pub fn acknowledge_clipboard_image_deletions(
        &mut self,
        paths: &[String],
    ) -> Result<(), String> {
        if paths.is_empty() {
            return Ok(());
        }
        let tx = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        {
            let mut delete = tx
                .prepare("DELETE FROM clipboard_pending_image_deletions WHERE image_path=?1")
                .map_err(storage_error)?;
            for path in paths {
                delete.execute([path]).map_err(storage_error)?;
            }
        }
        tx.commit().map_err(storage_error)
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
        let (rich_text_format, rich_text_html) = if record.kind == "text" {
            self.clipboard_text_formatting(id)?
                .as_ref()
                .map(crate::clipboard::rich_text::preview)
                .unwrap_or((None, None))
        } else {
            (None, None)
        };
        Ok(crate::clipboard::ClipboardDetail {
            title,
            character_count: (record.kind == "text").then(|| record.text.chars().count()),
            rich_text_format,
            rich_text_html,
            image_index_status: None,
            id: record.id,
            date: record.date,
            is_favorite: record.is_favorite,
            is_pinned: record.is_pinned,
            source_app: record.source_app,
            image_path: record.image_path,
            text: record.text,
            kind: record.kind,
            width: None,
            height: None,
            size: None,
            files: Vec::new(),
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
        if text.is_some() && kind != "text" {
            return Err("invalid_argument".into());
        }
        let same_text = if let Some(text) = text {
            tx.query_row(
                "SELECT text=?2 FROM clipboard_contents WHERE sequence=?1",
                params![sequence, text],
                |row| row.get::<_, bool>(0),
            )
            .map_err(storage_error)?
        } else {
            false
        };
        if let Some(text) = text.filter(|_| !same_text) {
            tx.execute(
                "DELETE FROM clipboard_text_formatting WHERE sequence=?1",
                [sequence],
            )
            .map_err(storage_error)?;
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
        tx.execute("DELETE FROM clipboard_titles WHERE sequence=?1", [sequence])
            .map_err(storage_error)?;
        if !title.is_empty() {
            tx.execute(
                "INSERT INTO clipboard_titles(sequence,search_folded) VALUES(?1,?2)",
                params![sequence, title.to_lowercase()],
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
        let pinned: bool = transaction
            .query_row(
                "SELECT is_pinned FROM clipboard_records WHERE id=?1",
                [id],
                |row| row.get(0),
            )
            .optional()
            .map_err(storage_error)?
            .ok_or("not_found")?;
        let sql = match action {
            "favorite" => "UPDATE clipboard_records SET is_favorite=NOT is_favorite WHERE id=?1",
            "pin" => {
                let order = if pinned {
                    let date: String = transaction
                        .query_row(
                            "SELECT date FROM clipboard_records WHERE id=?1",
                            [id],
                            |row| row.get(0),
                        )
                        .map_err(storage_error)?;
                    chrono::DateTime::parse_from_rfc3339(&date)
                        .map_err(|_| "storage_record")?
                        .timestamp_millis()
                } else {
                    transaction.query_row("SELECT MAX(?1,COALESCE(MAX(created_at),0)+1) FROM clipboard_records WHERE is_pinned=1", [chrono::Utc::now().timestamp_millis()], |row| row.get::<_, i64>(0)).map_err(storage_error)?
                };
                transaction.execute("UPDATE clipboard_records SET is_pinned=NOT is_pinned,created_at=?2 WHERE id=?1", params![id,order]).map_err(storage_error)?;
                transaction.commit().map_err(storage_error)?;
                return Ok(());
            }
            "delete" => {
                transaction.execute(
                    "INSERT OR IGNORE INTO clipboard_pending_image_deletions(image_path)
                     SELECT image_path FROM clipboard_records WHERE id=?1 AND image_path IS NOT NULL
                     UNION
                     SELECT f.image_path FROM clipboard_file_representations f JOIN clipboard_records r USING(sequence) WHERE r.id=?1",
                    [id],
                ).map_err(storage_error)?;
                "DELETE FROM clipboard_records WHERE id=?1"
            }
            _ => return Err("invalid_argument".into()),
        };
        transaction.execute(sql, [id]).map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        if action == "delete" {
            self.clipboard_total_records -= 1;
        }
        Ok(())
    }

    pub fn clipboard_promote(&mut self, id: &str) -> Result<(), String> {
        let now = chrono::Utc::now();
        let date = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let created = now.timestamp_millis();
        let updated = self
            .connection
            .execute(
                "UPDATE clipboard_records SET date=?1,created_at=CASE WHEN is_pinned THEN MAX(?2,COALESCE((SELECT MAX(created_at) FROM clipboard_records WHERE is_pinned=1),0)+1) ELSE ?2 END WHERE id=?3",
                params![date, created, id],
            )
            .map_err(storage_error)?;
        if updated == 0 {
            return Err("not_found".into());
        }
        Ok(())
    }

    pub fn clipboard_cleanup(
        &mut self,
        policy: Option<HistoryPolicy>,
        categories: u8,
        expected: Option<&str>,
    ) -> Result<(usize, Vec<String>), String> {
        self.clipboard_cleanup_scoped(policy, categories, expected, None)
    }

    pub fn clipboard_cleanup_scoped(
        &mut self,
        policy: Option<HistoryPolicy>,
        categories: u8,
        expected: Option<&str>,
        recent_since: Option<i64>,
    ) -> Result<(usize, Vec<String>), String> {
        self.clipboard_cleanup_with_media(policy, categories, expected, recent_since, None)
    }

    pub fn clipboard_cleanup_with_media(
        &mut self,
        policy: Option<HistoryPolicy>,
        categories: u8,
        expected: Option<&str>,
        recent_since: Option<i64>,
        image_root: Option<&std::path::Path>,
    ) -> Result<(usize, Vec<String>), String> {
        self.clipboard_cleanup_internal(
            policy,
            categories,
            expected,
            recent_since,
            image_root,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn clipboard_cleanup_with_budget(
        &mut self,
        policy: Option<HistoryPolicy>,
        expected: Option<&str>,
        image_root: Option<&std::path::Path>,
        storage_budget_override: Option<u64>,
    ) -> Result<(usize, Vec<String>), String> {
        self.clipboard_cleanup_internal(
            policy,
            0,
            expected,
            None,
            image_root,
            storage_budget_override,
        )
    }

    fn clipboard_cleanup_internal(
        &mut self,
        policy: Option<HistoryPolicy>,
        categories: u8,
        expected: Option<&str>,
        recent_since: Option<i64>,
        image_root: Option<&std::path::Path>,
        storage_budget_override: Option<u64>,
    ) -> Result<(usize, Vec<String>), String> {
        if let (Some(policy), 0, None) = (policy, categories, expected) {
            if policy.valid()
                && policy.max_age_days.is_none()
                && policy.max_storage_mb.is_none()
                && storage_budget_override.is_none()
                && recent_since.is_none()
                && self.clipboard_total_records <= policy.limit as usize
            {
                return Ok((0, vec![]));
            }
        }
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        let view = RecordView::Clipboard;
        // Freeze the clock across verification and deletion so a record cannot age into the confirmed set mid-transaction.
        let ctx = CleanupContext {
            policy,
            categories,
            recent_since,
            now: chrono::Utc::now().timestamp_millis(),
            image_root,
            storage_budget_override,
        };
        check_cleanup(&transaction, view, ctx, expected)?;
        let sequences = clipboard_cleanup_sequences(&transaction, ctx)?;
        let mut images = std::collections::HashSet::new();
        {
            let mut image_query = transaction
                .prepare_cached(
                    "SELECT r.image_path, f.image_path
                    FROM clipboard_records r
                    LEFT JOIN clipboard_file_representations f ON f.sequence = r.sequence
                    WHERE r.sequence = ?1",
                )
                .map_err(storage_error)?;
            for &seq in &sequences {
                if let Some((primary, file_rep)) = image_query
                    .query_row([seq], |row| {
                        Ok((
                            row.get::<_, Option<String>>(0)?,
                            row.get::<_, Option<String>>(1)?,
                        ))
                    })
                    .optional()
                    .map_err(storage_error)?
                {
                    if let Some(path) = primary {
                        images.insert(path);
                    }
                    if let Some(path) = file_rep {
                        images.insert(path);
                    }
                }
            }
        }
        let mut removed = 0usize;
        {
            let mut delete_stmt = transaction
                .prepare_cached("DELETE FROM clipboard_records WHERE sequence = ?1")
                .map_err(storage_error)?;
            for &seq in &sequences {
                removed += delete_stmt.execute([seq]).map_err(storage_error)?;
            }
        }
        persist_limit(&transaction, view, policy, expected)?;
        let mut unreferenced = Vec::new();
        for image in images {
            if !image_referenced(&transaction, &image)? {
                transaction.execute(
                    "INSERT OR IGNORE INTO clipboard_pending_image_deletions(image_path) VALUES(?1)",
                    [&image],
                ).map_err(storage_error)?;
                unreferenced.push(image);
            }
        }
        transaction.commit().map_err(storage_error)?;
        self.clipboard_total_records -= removed;
        Ok((removed, unreferenced))
    }

    pub fn clipboard_unprotected_count(&self) -> Result<usize, String> {
        self.connection
            .query_row(
                "SELECT count(*) FROM clipboard_records WHERE is_favorite=0 AND is_pinned=0",
                [],
                |row| row.get::<_, i64>(0),
            )
            .map_err(storage_error)
            .and_then(|count| usize::try_from(count).map_err(|_| "storage_corrupt".into()))
    }

    pub fn clipboard_storage_bytes(&self) -> Result<u64, String> {
        self.connection.query_row(
            "SELECT
                COALESCE((SELECT sum(octet_length(id)+octet_length(kind)+octet_length(summary)+octet_length(date)+octet_length(title)+octet_length(fingerprint)+COALESCE(octet_length(image_path),0)) FROM clipboard_records),0)
                + COALESCE((SELECT sum(octet_length(text)+octet_length(search_folded)) FROM clipboard_contents),0)
                + COALESCE((SELECT sum(octet_length(search_folded)) FROM clipboard_titles),0)
                + COALESCE((SELECT sum(octet_length(search_folded)+COALESCE(octet_length(original_text),0)) FROM clipboard_ocr),0)
                + COALESCE((SELECT sum(octet_length(paths)+octet_length(image_path)) FROM clipboard_file_representations),0)
                + COALESCE((SELECT sum(COALESCE(length(html),0)+COALESCE(length(rtf),0)) FROM clipboard_text_formatting),0)",
            [], |row| row.get::<_, i64>(0),
        ).map_err(storage_error).and_then(|bytes| u64::try_from(bytes).map_err(|_| "storage_corrupt".into()))
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

    pub fn preference(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row("SELECT value FROM preferences WHERE key=?1", [key], |row| {
                row.get(0)
            })
            .optional()
            .map_err(storage_error)
    }

    pub fn reset_shortcut_group(
        &mut self,
        bindings: &str,
        disabled: &str,
        selection_enabled: bool,
        records: Option<&str>,
    ) -> Result<(), String> {
        self.connection
            .execute(
                "INSERT INTO preferences(key,value)
             SELECT 'translation_shortcuts', ?1 UNION ALL
             SELECT 'translation_disabled_shortcuts', ?2 UNION ALL
             SELECT 'translation_selection_enabled', ?3 UNION ALL
             SELECT 'clipboard_record_shortcuts', ?4 WHERE ?4 IS NOT NULL
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![bindings, disabled, selection_enabled.to_string(), records],
            )
            .map_err(storage_error)?;
        Ok(())
    }

    pub fn reset_clipboard_shortcuts(
        &mut self,
        binding: &str,
        records: &str,
    ) -> Result<(), String> {
        self.connection.execute(
            "INSERT INTO preferences(key,value) VALUES
              ('clipboard_shortcut', ?1), ('clipboard_shortcut_enabled', 'true'), ('clipboard_record_shortcuts', ?2)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![binding, records],
        ).map_err(storage_error)?;
        Ok(())
    }

    pub fn preference_set(&mut self, key: &str, value: &str) -> Result<(), String> {
        const KEYS: &[&str] = &[
            "ui_locale",
            "window_position_mode",
            "clipboard_recording_types",
            "clipboard_shortcut",
            "clipboard_shortcut_enabled",
            "clipboard_record_shortcuts",
            "clipboard_excluded_apps",
            "clipboard_ignored_pasteboard_types",
            "clipboard_ignored_regexps",
            "clipboard_ignore_secrets",
            "clipboard_activation",
            "clipboard_paste_as_plain_text",
            "clipboard_search_options",
            "records_open_focus",
            "clipboard_preview_lines",
            "translation_preview_lines",
            "translation_engine",
            "translation_source",
            "translation_primary",
            "translation_secondary",
            "translation_proxy_url",
            "translation_proxy_enabled",
            "translation_selection_enabled",
            "translation_shortcuts",
            "translation_disabled_shortcuts",
        ];
        if !KEYS.contains(&key) {
            return Err("invalid_argument".into());
        }
        self.connection
            .execute(
                "INSERT INTO preferences(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=?2",
                params![key, value],
            )
            .map_err(storage_error)?;
        Ok(())
    }
}

pub(super) fn ensure_translation_preferences(
    connection: &rusqlite::Connection,
) -> Result<(), String> {
    connection
        .execute_batch(
            "INSERT OR IGNORE INTO preferences(key,value) VALUES
            ('translation_engine','edge'),
            ('translation_source','auto'),
            ('translation_primary','en'),
            ('translation_secondary','zh-CN'),
            ('translation_proxy_url',''),
            ('translation_proxy_enabled','false'),
            ('translation_selection_enabled','true');",
        )
        .map_err(storage_error)
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
    fn clipboard_mixed_search_falls_back_by_section_and_keeps_cursor_stable() {
        let mut db = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        for (id, text, pinned, favorite) in [
            ("pin-literal-old", "a.b literal older", true, false),
            ("pin-literal-new", "a.b literal newer", true, false),
            ("pin-regex", "axb pinned", true, true),
            ("regular-regex", "axb regular", false, true),
            ("regular-fuzzy", "ax.yb fallback", false, false),
            ("invalid-regex-fuzzy", "[Notebook", false, false),
            ("note-literal", "Unrelated body", false, false),
        ] {
            let mut item = record(id, text);
            item.is_pinned = pinned;
            item.is_favorite = favorite;
            db.clipboard_insert(&item).unwrap();
        }
        db.clipboard_edit("note-literal", "a.b in excluded notes", None)
            .unwrap();
        let image = image_record("image-regex");
        db.clipboard_insert(&image).unwrap();
        db.clipboard_index_image(&image.id, &image.fingerprint, "azb in OCR")
            .unwrap();
        let mixed = ClipboardSearchOptions {
            mode: ClipboardSearchMode::Mixed,
            note: false,
            ..ClipboardSearchOptions::default()
        };
        let search = |query, favorites, kind, cursor, limit| {
            db.clipboard_page_with_search(query, favorites, kind, cursor, limit, mixed)
                .unwrap()
        };
        let full_pinned = search("a.b", false, None, None, 2);
        assert!(full_pinned.records.iter().all(|item| item.is_pinned));
        assert!(full_pinned.has_more);
        let after_pinned = search("a.b", false, None, full_pinned.cursor.as_ref(), 2);
        assert_eq!(
            after_pinned
                .records
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["image-regex", "regular-regex"]
        );
        assert!(!after_pinned.has_more);
        let first = search("a.b", false, None, None, 1);
        assert_eq!(first.records[0].id, "pin-literal-new");
        assert_eq!(first.total_records, 4);
        assert!(first.has_more);
        let second = search("a.b", false, None, first.cursor.as_ref(), 2);
        assert_eq!(
            second
                .records
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["pin-literal-old", "image-regex"]
        );
        assert!(second.records[1].image_text_match);
        assert!(second.has_more);
        let last = search("a.b", false, None, second.cursor.as_ref(), 2);
        assert_eq!(last.records[0].id, "regular-regex");
        assert_eq!(last.total_records, 4);
        assert!(!last.has_more);
        let exhausted = search("a.b", false, None, last.cursor.as_ref(), 2);
        assert!(exhausted.records.is_empty());
        assert_eq!(exhausted.total_records, 4);
        assert_eq!(
            search("a.b", true, Some("text"), None, 20)
                .records
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["pin-regex", "regular-regex"]
        );
        let invalid = search("[nbk", false, None, None, 20);
        assert_eq!(invalid.total_records, 1);
        assert_eq!(invalid.records[0].id, "invalid-regex-fuzzy");
        assert_eq!(
            search("a..b", false, Some("image"), None, 20).total_records,
            0
        );
        assert_eq!(search("", false, None, None, 20).total_records, 8);
    }

    #[test]
    fn clipboard_search_rules_scope_pagination_and_ocr_origin() {
        let mut db = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        let mut body = record("body", "Synthetic Notebook 42");
        body.is_favorite = true;
        db.clipboard_insert(&body).unwrap();
        db.clipboard_insert(&record("note", "Plain unrelated body"))
            .unwrap();
        db.clipboard_edit("note", "Notebook 77", None).unwrap();
        let mut image = image_record("ocr");
        image.is_pinned = true;
        db.clipboard_insert(&image).unwrap();
        db.clipboard_index_image(&image.id, &image.fingerprint, "Notebook 99")
            .unwrap();
        let all = ClipboardSearchOptions::default();
        let query = |query, search| {
            db.clipboard_page_with_search(query, false, None, None, 10, search)
                .unwrap()
        };
        let exact = query("NOTEBOOK", all);
        assert_eq!(exact.total_records, 3);
        assert_eq!(
            exact
                .records
                .iter()
                .filter(|r| r.image_text_match)
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["ocr"]
        );
        assert!(query("", all).records.iter().all(|r| !r.image_text_match));
        assert_eq!(
            query(
                "NOTEB",
                ClipboardSearchOptions {
                    note: false,
                    image_text: false,
                    ..all
                }
            )
            .records[0]
                .id,
            "body"
        );
        assert_eq!(
            query(
                "NO",
                ClipboardSearchOptions {
                    body: false,
                    image_text: false,
                    ..all
                }
            )
            .records[0]
                .id,
            "note"
        );
        let fuzzy = ClipboardSearchOptions {
            mode: ClipboardSearchMode::Fuzzy,
            ..all
        };
        assert_eq!(query("NBK", fuzzy).total_records, 3);
        assert_eq!(query("NBK", all).total_records, 0);
        assert_eq!(query("K8N", fuzzy).total_records, 0);
        let regex = ClipboardSearchOptions {
            mode: ClipboardSearchMode::Regex,
            ..all
        };
        assert_eq!(query(r"^Notebook (77|99)$", regex).total_records, 2);
        assert_eq!(query(r"^notebook", regex).total_records, 0);
        let scoped = ClipboardSearchOptions {
            body: false,
            note: false,
            ..regex
        };
        let hits = query(r"^Notebook \d+$", scoped);
        assert_eq!(hits.total_records, 1);
        assert!(hits.records[0].image_text_match);
        assert_eq!(
            db.clipboard_page_with_search(r"\d+$", true, Some("text"), None, 1, regex)
                .unwrap()
                .records[0]
                .id,
            "body"
        );
        let first = db
            .clipboard_page_with_search(r"\d+$", false, None, None, 1, regex)
            .unwrap();
        let second = db
            .clipboard_page_with_search(r"\d+$", false, None, first.cursor.as_ref(), 2, regex)
            .unwrap();
        assert_eq!(first.records[0].id, "ocr");
        assert!(first.has_more);
        assert_eq!(second.total_records, 3);
        assert_eq!(
            second
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["note", "body"]
        );
        assert_eq!(
            db.clipboard_page_with_search("[", false, None, None, 10, regex)
                .unwrap_err(),
            "clipboard_invalid_regex"
        );
        assert_eq!(
            db.clipboard_page_with_search(
                "hello",
                false,
                None,
                None,
                10,
                ClipboardSearchOptions {
                    body: false,
                    note: false,
                    image_text: false,
                    ..all
                }
            )
            .unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(query("notebook", all).total_records, 3);
    }

    #[test]
    fn clipboard_age_policy_persists_and_prunes_search_after_reopen() {
        let path =
            std::env::temp_dir().join(format!("nami-retention-{}.sqlite3", uuid::Uuid::new_v4()));
        let mut db = HistoryDatabase::open(&path).unwrap();
        let view = RecordView::Clipboard;
        let policy = Some(HistoryPolicy {
            limit: 500,
            max_age_days: Some(7),
            max_storage_mb: None,
        });
        let mut expired = record("expired", "synthetic expired searchable body");
        expired.date = "2001-01-01T00:00:00.000Z".into();
        db.clipboard_insert(&expired).unwrap();
        let mut protected = record("favorite", "synthetic favorite");
        protected.date = expired.date.clone();
        protected.is_favorite = true;
        db.clipboard_insert(&protected).unwrap();
        let mut later = record("later", "synthetic will age");
        later.date = chrono::Utc::now().to_rfc3339();
        db.clipboard_insert(&later).unwrap();
        let preview = db.records_cleanup_preview(view, policy).unwrap();
        assert_eq!(preview.count, 1);
        assert_eq!(
            db.clipboard_cleanup(policy, 0, Some(&preview.token))
                .unwrap()
                .0,
            1
        );
        assert_eq!(
            db.clipboard_page("searchable body", false, None, 100)
                .unwrap()
                .total_records,
            0
        );
        assert_eq!(
            db.records_cleanup_preview(RecordView::Translation, policy)
                .err()
                .as_deref(),
            Some("invalid_argument")
        );
        db.connection
            .execute(
                "UPDATE clipboard_records SET date='2001-01-01T00:00:00.000Z' WHERE id='later'",
                [],
            )
            .unwrap();
        drop(db);

        let mut db = HistoryDatabase::open(&path).unwrap();
        let saved = db.records_history_policy(view).unwrap();
        assert_eq!(Some(saved), policy);
        assert_eq!(db.clipboard_cleanup(Some(saved), 0, None).unwrap().0, 1);
        assert_eq!(
            db.clipboard_page("", false, None, 100).unwrap().records[0].id,
            "favorite"
        );
        drop(db);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn clipboard_recent_cleanup_protects_flags_and_rechecks_confirmation() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let view = RecordView::Clipboard;
        let now = chrono::Utc::now();
        let since = now.timestamp_millis() - 15 * 60_000;
        for (id, time, pinned, favorite) in [
            ("before", since - 1, false, false),
            ("boundary", since, false, false),
            ("pinned", since, true, false),
            ("favorite", since, false, true),
        ] {
            let mut item = record(id, id);
            item.date = chrono::DateTime::from_timestamp_millis(time)
                .unwrap()
                .to_rfc3339();
            item.is_pinned = pinned;
            item.is_favorite = favorite;
            db.clipboard_insert(&item).unwrap();
        }
        let preview = db
            .records_cleanup_preview_scoped(view, None, Some(since))
            .unwrap();
        assert_eq!(preview.count, 1);
        assert_eq!(preview.recent_since, Some(since));
        let mut new = record("new", "synthetic added during confirmation");
        new.date = now.to_rfc3339();
        db.clipboard_insert(&new).unwrap();
        assert_eq!(
            db.clipboard_cleanup_scoped(None, 0, Some(&preview.token), Some(since)),
            Err("clipboard_cleanup_changed".into())
        );
        let current = db
            .records_cleanup_preview_scoped(view, None, Some(since))
            .unwrap();
        assert_eq!(current.count, 2);
        assert!(db
            .clipboard_cleanup_scoped(None, 7, Some(&current.token), Some(since))
            .is_err());
        assert_eq!(
            db.clipboard_cleanup_scoped(None, 0, Some(&current.token), Some(since))
                .unwrap()
                .0,
            2
        );
        let remaining = db.clipboard_page("", false, None, 100).unwrap();
        assert_eq!(remaining.total_records, 3);
        assert!(remaining
            .records
            .iter()
            .all(|row| ["before", "pinned", "favorite"].contains(&row.id.as_str())));
    }

    #[test]
    fn clipboard_retention_keeps_old_rows_until_count_is_exceeded_and_protects_flags() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let policy = HistoryPolicy {
            limit: 2,
            ..Default::default()
        };
        let mut old = record("old", "unique searchable body");
        old.date = "2001-01-01T00:00:00.000Z".into();
        db.clipboard_insert(&old).unwrap();
        db.clipboard_insert(&record("newer", "newer")).unwrap();
        assert_eq!(
            db.records_cleanup_preview(RecordView::Clipboard, Some(policy))
                .unwrap()
                .count,
            0
        );
        db.clipboard_insert(&record("newest", "newest")).unwrap();
        for flag in ["favorite", "pin"] {
            db.clipboard_insert(&record(flag, flag)).unwrap();
            db.clipboard_update(flag, flag).unwrap();
        }
        let token = db
            .records_cleanup_preview(RecordView::Clipboard, Some(policy))
            .unwrap()
            .token;
        assert_eq!(
            db.clipboard_cleanup(Some(policy), 0, Some(&token)).unwrap(),
            (1, vec![])
        );
        assert!(db.clipboard_record("old").is_err());
        assert_eq!(
            db.clipboard_page("searchable", false, None, 100)
                .unwrap()
                .total_records,
            0
        );
        assert_eq!(db.clipboard_unprotected_count().unwrap(), 2);
        assert!(db.clipboard_record("favorite").unwrap().is_favorite);
        assert!(db.clipboard_record("pin").unwrap().is_pinned);
        assert_eq!(
            db.records_history_policy(RecordView::Clipboard).unwrap(),
            policy
        );
    }

    #[test]
    fn clipboard_retention_confirmation_rejects_a_changed_set_or_rolled_back_write() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let policy = HistoryPolicy {
            limit: 1,
            ..Default::default()
        };
        db.clipboard_insert(&record("first", "first")).unwrap();
        let original = db
            .records_cleanup_preview(RecordView::Clipboard, Some(policy))
            .unwrap()
            .token;
        db.clipboard_insert(&record("second", "second")).unwrap();
        let preview = db
            .records_cleanup_preview(RecordView::Clipboard, Some(policy))
            .unwrap();
        assert_eq!(preview.count, 1);
        assert_eq!(
            db.clipboard_cleanup(Some(policy), 0, Some(&original))
                .unwrap_err(),
            "clipboard_cleanup_changed"
        );
        db.connection.execute_batch("CREATE TRIGGER refuse_history_policy BEFORE INSERT ON preferences WHEN new.key='clipboard_history_policy' BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(db
            .clipboard_cleanup(Some(policy), 0, Some(&preview.token))
            .is_err());
        assert!(db.clipboard_record("first").is_ok());
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            2
        );
        assert_eq!(db.preference("clipboard_history_policy").unwrap(), None);
    }

    #[test]
    fn clipboard_clear_applies_only_the_selected_category_mask() {
        let labels = ["ordinary", "favorite", "pinned", "both"];
        for mask in 1..=7 {
            let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
            for (category, label) in labels.iter().enumerate() {
                let mut item = record(label, label);
                item.is_favorite = category & 1 != 0;
                item.is_pinned = category & 2 != 0;
                db.clipboard_insert(&item).unwrap();
            }
            let preview = db
                .records_cleanup_preview(RecordView::Clipboard, None)
                .unwrap();
            assert_eq!(preview.categories, Some([1, 2, 2]));
            assert_eq!(preview.count, 4);
            let selected = [1, 2, 4, 6].map(|flags| mask & flags != 0);
            assert_eq!(
                db.clipboard_cleanup(None, mask, Some(&preview.token))
                    .unwrap()
                    .0,
                selected.iter().filter(|&&item| item).count()
            );
            for (label, remove) in labels.iter().zip(selected) {
                assert_eq!(db.clipboard_record(label).is_err(), remove);
            }
            assert_eq!(db.preference("clipboard_history_policy").unwrap(), None);
        }
    }

    #[test]
    fn clipboard_clear_rejects_missing_selection_or_a_changed_snapshot() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        db.clipboard_insert(&record("first", "first")).unwrap();
        db.clipboard_insert(&record("second", "second")).unwrap();
        db.clipboard_update("first", "favorite").unwrap();
        let preview = db
            .records_cleanup_preview(RecordView::Clipboard, None)
            .unwrap();
        let token = Some(preview.token.as_str());
        assert_eq!(
            db.clipboard_cleanup(None, 0, token).unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_cleanup(None, 8, token).unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_cleanup(None, 1, None).unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_cleanup(Some(HistoryPolicy::default()), 1, token)
                .unwrap_err(),
            "invalid_argument"
        );
        db.clipboard_update("first", "favorite").unwrap();
        db.clipboard_update("second", "favorite").unwrap();
        assert_eq!(
            db.records_cleanup_preview(RecordView::Clipboard, None)
                .unwrap()
                .categories,
            preview.categories
        );
        assert_eq!(
            db.clipboard_cleanup(None, 2, token).unwrap_err(),
            "clipboard_cleanup_changed"
        );
        assert!(db.clipboard_record("first").is_ok());
        assert!(db.clipboard_record("second").is_ok());
    }

    #[test]
    fn clipboard_clear_keeps_shared_media_and_translation_and_counts_only_clipboard_data() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        db.connection.execute("INSERT INTO translation_records(id,created_at,record_json,engine,is_favorite,is_pinned,source_folded,primary_folded,secondary_folded) VALUES('translation',0,'{}','edge',0,0,'','','')", []).unwrap();
        assert_eq!(db.clipboard_storage_bytes().unwrap(), 0);
        let image = image_record("image");
        let attachment = representation(&image, "photo.png");
        let mut files = record("files", &serde_json::to_string(&attachment.paths).unwrap());
        files.kind = "files".into();
        files.is_favorite = true;
        db.clipboard_insert(&image).unwrap();
        db.clipboard_insert_with_file_representation(&files, Some(&attachment))
            .unwrap();
        let before = db.clipboard_storage_bytes().unwrap();
        assert!(before > 0);
        let token = db
            .records_cleanup_preview(RecordView::Clipboard, None)
            .unwrap()
            .token;
        assert_eq!(
            db.clipboard_cleanup(None, 1, Some(&token)).unwrap(),
            (1, vec![])
        );
        assert!(db.clipboard_storage_bytes().unwrap() < before);
        assert!(db
            .clipboard_image_referenced(&attachment.image_path)
            .unwrap());
        let token = db
            .records_cleanup_preview(RecordView::Clipboard, None)
            .unwrap()
            .token;
        assert_eq!(
            db.clipboard_cleanup(None, 2, Some(&token)).unwrap(),
            (1, vec![attachment.image_path])
        );
        assert_eq!(db.clipboard_storage_bytes().unwrap(), 0);
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM translation_records", [], |row| row
                    .get::<_, u32>(0))
                .unwrap(),
            1
        );
    }

    #[test]
    fn title_search_matches_literal_unicode_and_full_body_with_paging() {
        let mut db = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        for index in 0..103 {
            let id = format!("title-{index}");
            db.clipboard_insert(&record(&id, &format!("{} body {index}", "x".repeat(300))))
                .unwrap();
            db.clipboard_edit(&id, "PLAN 海浪 100%_literal \"quoted\"", None)
                .unwrap();
        }
        db.clipboard_update("title-0", "favorite").unwrap();
        for needle in ["plan", "海", "海浪", "100%_literal", "\"quoted\"", "body"] {
            let first = db.clipboard_page(needle, false, None, 100).unwrap();
            assert_eq!(first.total_records, 103, "{needle}");
            assert_eq!(first.records.len(), 100);
            assert!(first.has_more);
            let last = db
                .clipboard_page(needle, false, first.cursor.as_ref(), 100)
                .unwrap();
            assert_eq!(last.records.len(), 3);
            assert_eq!(last.total_records, 103);
            assert!(!last.has_more);
            assert_eq!(
                db.clipboard_page(needle, true, None, 100)
                    .unwrap()
                    .total_records,
                1
            );
        }
        assert_eq!(
            db.clipboard_page("100_other", false, None, 100)
                .unwrap()
                .total_records,
            0
        );
        db.clipboard_edit("title-0", "Replacement", None).unwrap();
        assert_eq!(
            db.clipboard_page("plan", true, None, 100)
                .unwrap()
                .total_records,
            0
        );
        assert_eq!(
            db.clipboard_page("replacement", true, None, 100)
                .unwrap()
                .total_records,
            1
        );
        db.clipboard_edit("title-0", "", None).unwrap();
        assert_eq!(
            db.clipboard_page("replacement", true, None, 100)
                .unwrap()
                .total_records,
            0
        );
        assert!(db
            .clipboard_detail("title-0")
            .unwrap()
            .text
            .ends_with("body 0"));
    }

    #[test]
    fn title_search_migrates_v5_additively_and_reopens_without_changing_payloads() {
        let root = std::env::temp_dir().join(format!("nami-title-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.sqlite3");
        let mut db = HistoryDatabase::open(&path).unwrap();
        db.clipboard_insert(&record("preserved", &"Original 🌊 body ".repeat(100)))
            .unwrap();
        db.clipboard_update("preserved", "favorite").unwrap();
        db.clipboard_update("preserved", "pin").unwrap();
        db.clipboard_edit("preserved", "ÉCOLE 海浪 \"100%_\"", None)
            .unwrap();
        let before = serde_json::to_value(db.clipboard_detail("preserved").unwrap()).unwrap();
        db.connection.execute_batch("DROP TABLE clipboard_text_formatting; DROP TABLE clipboard_ocr_search; DROP TABLE clipboard_ocr; DROP TABLE clipboard_title_search; DROP TABLE clipboard_titles; PRAGMA user_version=5;").unwrap();
        drop(db);
        for _ in 0..2 {
            let db = HistoryDatabase::open(&path).unwrap();
            assert_eq!(
                serde_json::to_value(db.clipboard_detail("preserved").unwrap()).unwrap(),
                before
            );
            for query in ["école", "ÉCOLE", "海", "\"100%_\"", "🌊 body"] {
                assert_eq!(
                    db.clipboard_page(query, true, None, 100)
                        .unwrap()
                        .total_records,
                    1,
                    "{query}"
                );
            }
            let plan: String = db.connection.query_row("EXPLAIN QUERY PLAN SELECT rowid FROM clipboard_title_search WHERE clipboard_title_search MATCH '\"école\"'", [], |row| row.get(3)).unwrap();
            assert!(plan.contains("VIRTUAL TABLE INDEX"), "{plan}");
        }
        let mut db = HistoryDatabase::open(&path).unwrap();
        db.clipboard_update("preserved", "favorite").unwrap();
        db.clipboard_update("preserved", "pin").unwrap();
        db.clipboard_update("preserved", "delete").unwrap();
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM clipboard_titles", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.clipboard_page("école", false, None, 100)
                .unwrap()
                .total_records,
            0
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn broad_title_search_counts_union_once_and_preserves_filters_and_cursors() {
        let mut db = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        for index in 0..1030 {
            let id = format!("broad-title-{index}");
            db.clipboard_insert(&record(&id, &format!("shared body {index}")))
                .unwrap();
            db.clipboard_edit(&id, "Shared title", None).unwrap();
            if index % 2 == 0 {
                db.clipboard_update(&id, "favorite").unwrap();
            }
        }
        for (needle, favorite, total) in [
            ("title", false, 1030),
            ("shared", false, 1030),
            ("shared", true, 515),
        ] {
            let mut cursor = None;
            let mut ids = std::collections::HashSet::new();
            loop {
                let page = db
                    .clipboard_page_filtered(needle, favorite, Some("text"), cursor.as_ref(), 100)
                    .unwrap();
                assert_eq!(page.total_records, total);
                for record in page.records {
                    assert!(ids.insert(record.id));
                }
                if !page.has_more {
                    break;
                }
                cursor = page.cursor;
            }
            assert_eq!(ids.len(), total);
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
                "DROP TABLE clipboard_text_formatting; DROP TABLE clipboard_ocr_search; DROP TABLE clipboard_ocr; DROP TABLE clipboard_title_search; DROP TABLE clipboard_titles; DROP TABLE clipboard_file_representations; DROP INDEX clipboard_image_resources; ALTER TABLE clipboard_records DROP COLUMN title; PRAGMA user_version=3;",
            )
            .unwrap();
        drop(db);
        let mut db = HistoryDatabase::open(&path).unwrap();
        let detail = db.clipboard_detail("edit").unwrap();
        assert!(detail.title.is_empty());
        assert_eq!(detail.text, original.text);
        assert_eq!(detail.id, original.id);
        assert!(detail.is_favorite);
        assert_eq!(detail.is_pinned, original.is_pinned);
        assert_eq!(detail.date, original.date);
        assert_eq!(detail.source_app, original.source_app);
        assert_eq!(detail.character_count, Some(original.text.chars().count()));
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

    fn image_record(id: &str) -> ClipboardRecord {
        let mut image = record(id, "");
        image.kind = "image".into();
        image.image_path = Some(format!("origin/ab/{}.png", "ab".repeat(32)));
        image.fingerprint = fingerprint("image", b"synthetic image");
        image
    }

    fn representation(image: &ClipboardRecord, file: &str) -> ClipboardFileRepresentation {
        ClipboardFileRepresentation {
            paths: vec![file_path(file)],
            image_path: image.image_path.clone().unwrap(),
        }
    }

    fn retry_clipboard_image_deletions(db: &mut HistoryDatabase, root: &Path) -> bool {
        let (complete, failed) =
            unlink_pending_clipboard_images(root, db.pending_clipboard_image_deletions().unwrap());
        db.acknowledge_clipboard_image_deletions(&complete).unwrap();
        failed
    }

    #[test]
    fn committed_clipboard_deletions_retry_after_filesystem_failure_and_restart() {
        for bulk in [false, true] {
            let root =
                std::env::temp_dir().join(format!("nami-image-retry-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&root).unwrap();
            let database = root.join("history.sqlite3");
            let mut db = HistoryDatabase::open(&database).unwrap();
            db.connection
                .execute_batch(
                    "DROP TABLE clipboard_pending_image_deletions; PRAGMA user_version=9;",
                )
                .unwrap();
            let image = image_record("image");
            let id = if bulk {
                let attachment = representation(&image, "photo.png");
                let mut files = record("files", &serde_json::to_string(&attachment.paths).unwrap());
                files.kind = "files".into();
                db.clipboard_insert_with_file_representation(&files, Some(&attachment))
                    .unwrap();
                files.id
            } else {
                db.clipboard_insert(&image).unwrap();
                image.id
            };
            drop(db);

            let mut db = HistoryDatabase::open(&database).unwrap();
            let path = root.join(image.image_path.as_ref().unwrap());
            // An otherwise valid cache filename occupied by a directory makes unlink fail on all platforms.
            std::fs::create_dir_all(&path).unwrap();
            if bulk {
                let token = db
                    .records_cleanup_preview(RecordView::Clipboard, None)
                    .unwrap()
                    .token;
                assert_eq!(db.clipboard_cleanup(None, 1, Some(&token)).unwrap().0, 1);
            } else {
                db.clipboard_update(&id, "delete").unwrap();
            }
            assert_eq!(db.clipboard_record(&id).unwrap_err(), "not_found");
            assert!(retry_clipboard_image_deletions(&mut db, &root));
            drop(db);

            std::fs::remove_dir(&path).unwrap();
            std::fs::write(&path, b"synthetic image").unwrap();
            let db = HistoryDatabase::open(&database).unwrap();
            assert_eq!(db.clipboard_record(&id).unwrap_err(), "not_found");
            let (complete, failed) = unlink_pending_clipboard_images(
                &root,
                db.pending_clipboard_image_deletions().unwrap(),
            );
            assert_eq!(complete.len(), 1);
            assert!(!failed);
            assert!(!path.exists(), "orphan survived restart, bulk={bulk}");
            // A restart between unlink and acknowledgement must treat the missing file as success.
            drop(db);
            let mut db = HistoryDatabase::open(&database).unwrap();
            assert!(!retry_clipboard_image_deletions(&mut db, &root));
            std::fs::write(&path, b"synthetic image").unwrap();
            assert!(!retry_clipboard_image_deletions(&mut db, &root));
            assert!(path.is_file(), "successful retry did not clear the queue");
            drop(db);
            std::fs::remove_dir_all(&root).unwrap();
        }
    }

    #[test]
    fn pending_clipboard_image_deletion_rechecks_new_shared_references() {
        let root = std::env::temp_dir().join(format!("nami-image-shared-{}", uuid::Uuid::new_v4()));
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let image = image_record("image");
        let path = root.join(image.image_path.as_ref().unwrap());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"synthetic image").unwrap();
        db.clipboard_insert(&image).unwrap();
        db.clipboard_update(&image.id, "delete").unwrap();

        let attachment = representation(&image, "photo.png");
        let mut files = record("files", &serde_json::to_string(&attachment.paths).unwrap());
        files.kind = "files".into();
        db.clipboard_insert_with_file_representation(&files, Some(&attachment))
            .unwrap();
        assert!(!retry_clipboard_image_deletions(&mut db, &root));
        assert!(path.is_file());

        db.clipboard_update(&files.id, "delete").unwrap();
        assert!(!retry_clipboard_image_deletions(&mut db, &root));
        assert!(!path.exists());
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn clipboard_image_deletion_acknowledgement_is_atomic() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        db.connection
            .execute_batch(
                "INSERT INTO clipboard_pending_image_deletions VALUES ('first'), ('second');
            CREATE TEMP TRIGGER abort_ack BEFORE DELETE ON clipboard_pending_image_deletions
            WHEN old.image_path='second' BEGIN SELECT RAISE(ABORT, 'synthetic failure'); END;",
            )
            .unwrap();
        let paths = ["first".to_owned(), "second".to_owned()];
        assert!(db.acknowledge_clipboard_image_deletions(&paths).is_err());
        assert_eq!(db.pending_clipboard_image_deletions().unwrap().len(), 2);
        db.connection
            .execute_batch("DROP TRIGGER abort_ack;")
            .unwrap();
        db.acknowledge_clipboard_image_deletions(&paths).unwrap();
        assert!(db.pending_clipboard_image_deletions().unwrap().is_empty());
    }

    #[test]
    fn rich_text_smoke_reopens_deduplicates_and_clears_only_after_body_changes() {
        let root = std::env::temp_dir().join(format!("nami-rich-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.sqlite3");
        let legacy = record("legacy", "Same body");
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            db.clipboard_insert(&legacy).unwrap();
            db.connection
                .execute_batch("DROP TABLE clipboard_text_formatting; ALTER TABLE clipboard_ocr DROP COLUMN original_text; PRAGMA user_version=7;")
                .unwrap();
        }
        let first = ClipboardTextFormatting {
            html: Some(b"<b>Same body</b>".to_vec()),
            rtf: Some(br"{\rtf1\b Same body}".to_vec()),
        };
        let second = ClipboardTextFormatting {
            html: Some(b"<i>Same body</i>".to_vec()),
            rtf: None,
        };
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            assert_eq!(db.clipboard_text_formatting("legacy").unwrap(), None);
            let mut rich = record("first", "Same body");
            rich.fingerprint = first.fingerprint(&rich.text);
            assert_ne!(rich.fingerprint, second.fingerprint(&rich.text));
            assert_eq!(
                db.clipboard_insert_capture(&rich, None, Some(&first))
                    .unwrap()
                    .id,
                "first"
            );
            // Even when hashes collide, exact format bytes are checked.
            rich.id = "second".into();
            assert_eq!(
                db.clipboard_insert_capture(&rich, None, Some(&second))
                    .unwrap()
                    .id,
                "second"
            );
            rich.id = "duplicate".into();
            assert_eq!(
                db.clipboard_insert_capture(&rich, None, Some(&first))
                    .unwrap()
                    .id,
                "first"
            );
        }
        let mut db = HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            db.clipboard_text_formatting("first").unwrap(),
            Some(first.clone())
        );
        assert_eq!(
            db.clipboard_text_formatting("second").unwrap(),
            Some(second)
        );
        assert_eq!(
            db.clipboard_page("Same body", false, None, 20)
                .unwrap()
                .total_records,
            3
        );
        db.clipboard_edit("first", "Note", Some("Same body"))
            .unwrap();
        assert_eq!(
            db.clipboard_text_formatting("first").unwrap(),
            Some(first.clone())
        );
        db.clipboard_duplicate("first").unwrap();
        let copies = db
            .clipboard_page("Same body", false, None, 20)
            .unwrap()
            .records;
        let copy = copies
            .iter()
            .find(|row| !["legacy", "first", "second"].contains(&row.id.as_str()))
            .unwrap();
        assert_eq!(db.clipboard_text_formatting(&copy.id).unwrap(), Some(first));
        db.clipboard_edit("first", "Note", Some("Edited body"))
            .unwrap();
        assert_eq!(db.clipboard_text_formatting("first").unwrap(), None);
        assert_eq!(db.clipboard_record("first").unwrap().text, "Edited body");
        db.clipboard_update(&copy.id, "delete").unwrap();
        assert_eq!(
            db.connection
                .query_row(
                    "SELECT count(*) FROM clipboard_text_formatting",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rich_text_smoke_rejects_oversized_or_empty_attachments_without_writing() {
        let mut db = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        for html in [
            None,
            Some(vec![]),
            Some(vec![b'x'; MAX_RICH_TEXT_BYTES + 1]),
        ] {
            let formatting = ClipboardTextFormatting { html, rtf: None };
            assert_eq!(
                db.clipboard_insert_capture(&record("rejected", "Body"), None, Some(&formatting))
                    .unwrap_err(),
                "invalid_argument"
            );
        }
        assert_eq!(
            db.clipboard_page("", false, None, 20)
                .unwrap()
                .total_records,
            0
        );
    }

    #[test]
    fn legacy_ocr_text_survives_restart_and_is_not_overwritten_by_indexing() {
        let root =
            std::env::temp_dir().join(format!("nami-ocr-correction-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("history.sqlite3");
        let image = image_record("ocr-corrected");
        let corrected = "NAMI-OU-016\n已核对";
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            db.clipboard_insert(&image).unwrap();
            let derived = record("previous-extraction", "NAMl-0U-016");
            db.clipboard_insert_ocr_result(&image.id, &derived).unwrap();
            db.clipboard_edit(&image.id, "Keep note", None).unwrap();
            // Seed a legacy correction without retaining the removed mutation API.
            assert!(save_ocr(&db.connection, &image.id, None, corrected).unwrap());
            assert!(!db
                .clipboard_index_image(&image.id, &image.fingerprint, &derived.text)
                .unwrap());
        }
        let db = HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            db.clipboard_ocr_text(&image.id).unwrap().as_deref(),
            Some(corrected)
        );
        assert_eq!(db.clipboard_detail(&image.id).unwrap().title, "Keep note");
        assert_eq!(
            db.clipboard_record(&image.id).unwrap().image_path,
            image.image_path
        );
        assert_eq!(
            db.clipboard_page("", false, None, 10)
                .unwrap()
                .total_records,
            2
        );
        assert_eq!(
            db.clipboard_page("NAMl-0U-016", false, None, 10)
                .unwrap()
                .records[0]
                .id,
            "previous-extraction"
        );
        assert_eq!(
            db.clipboard_page("已核对", false, None, 10)
                .unwrap()
                .records[0]
                .id,
            image.id
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn automatic_ocr_preserves_legacy_search_and_reuses_exact_results() {
        let root = std::env::temp_dir().join(format!("nami-ocr-v8-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("history.sqlite3");
        let image = image_record("ocr-image");
        let blank = image_record("ocr-blank");
        let raw = "Mixed CASE E247\n海浪";
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            db.clipboard_insert_with_file_representation(
                &image,
                Some(&representation(&image, "ÉCOLE E247.png")),
            )
            .unwrap();
            let mut malformed = image_record("malformed-attachment");
            malformed.fingerprint.push_str("malformed");
            db.clipboard_insert_with_file_representation(
                &malformed,
                Some(&representation(&malformed, "invalid.png")),
            )
            .unwrap();
            db.connection.execute("UPDATE clipboard_file_representations SET paths='bad JSON' WHERE sequence=(SELECT sequence FROM clipboard_records WHERE id=?1)", [&malformed.id]).unwrap();
            db.connection.execute_batch("UPDATE clipboard_contents SET search_folded='';
                INSERT INTO clipboard_ocr(sequence,search_folded) SELECT sequence,'legacy keyword' FROM clipboard_records WHERE id='ocr-image';
                ALTER TABLE clipboard_ocr DROP COLUMN original_text;
                DROP TRIGGER clipboard_search_update; PRAGMA user_version=8;").unwrap();
        }
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            assert_eq!(db.clipboard_ocr_text(&image.id).unwrap(), None);
            for term in ["legacy keyword", "école e247.png"] {
                assert_eq!(
                    db.clipboard_page(term, false, None, 10).unwrap().records[0].id,
                    image.id
                );
            }
            assert!(!db.clipboard_index_image(&image.id, "stale", raw).unwrap());
            assert!(db
                .clipboard_index_image(&image.id, &image.fingerprint, "invalid\0text")
                .is_err());
            assert_eq!(db.clipboard_ocr_text(&image.id).unwrap(), None);
            assert!(db
                .clipboard_index_image(&image.id, &image.fingerprint, raw)
                .unwrap());
            assert!(!db
                .clipboard_index_image(&image.id, &image.fingerprint, "duplicate")
                .unwrap());
            let mut blank = blank.clone();
            blank.fingerprint.push_str("blank");
            db.clipboard_insert(&blank).unwrap();
            assert!(db
                .clipboard_index_image(&blank.id, &blank.fingerprint, "")
                .unwrap());
        }
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            assert_eq!(
                db.clipboard_ocr_text(&image.id).unwrap().as_deref(),
                Some(raw)
            );
            assert_eq!(
                db.clipboard_ocr_text(&blank.id).unwrap().as_deref(),
                Some("")
            );
            for term in ["E247", "海浪", "海", "école e247.png"] {
                let page = db.clipboard_page(term, false, None, 10).unwrap();
                assert_eq!(page.total_records, 1, "term {term}");
                assert_eq!(page.records[0].id, image.id);
            }
            db.clipboard_update(&image.id, "delete").unwrap();
            assert_eq!(db.clipboard_ocr_text(&image.id).unwrap(), None);
            assert!(!db
                .clipboard_index_image(&image.id, &image.fingerprint, raw)
                .unwrap());
            assert_eq!(
                db.clipboard_page("E247", false, None, 10)
                    .unwrap()
                    .total_records,
                0
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ocr_search_migrates_v6_and_reopens_without_rewriting_originals() {
        let root = std::env::temp_dir().join(format!("nami-ocr-v6-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.sqlite3");
        let image = image_record("image");
        let extracted = record("extracted", "OCR \"ÉCOLE\" 海浪");
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            db.clipboard_insert(&image).unwrap();
            db.clipboard_edit("image", "OCR title", None).unwrap();
            db.clipboard_update("image", "favorite").unwrap();
            db.clipboard_insert(&extracted).unwrap();
            db.connection.execute_batch("DROP TABLE clipboard_text_formatting; DROP TABLE clipboard_ocr_search; DROP TABLE clipboard_ocr; PRAGMA user_version=6;").unwrap();
        }
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            assert_eq!(
                db.clipboard_page("ocr", false, None, 20)
                    .unwrap()
                    .total_records,
                2
            );
            assert_eq!(
                db.clipboard_page_filtered("école", false, Some("image"), None, 20)
                    .unwrap()
                    .total_records,
                0
            );
            assert_eq!(
                db.clipboard_insert_ocr_result("image", &record("repeat", &extracted.text))
                    .unwrap()
                    .id,
                "extracted"
            );
            let first = db.clipboard_page("ocr", false, None, 1).unwrap();
            let next = db
                .clipboard_page("ocr", false, first.cursor.as_ref(), 1)
                .unwrap();
            assert_eq!(first.total_records, 2);
            assert_eq!(next.records.len(), 1);
            assert_ne!(first.records[0].id, next.records[0].id);
        }
        {
            let mut db = HistoryDatabase::open(&path).unwrap();
            for query in ["\"éCoLe\"", "海浪"] {
                let page = db
                    .clipboard_page_filtered(query, true, Some("image"), None, 20)
                    .unwrap();
                assert_eq!(page.total_records, 1);
                assert_eq!(page.records[0].id, "image");
                assert!(page.records[0].text.is_empty());
            }
            let original = db.clipboard_record("image").unwrap();
            assert_eq!(original.fingerprint, image.fingerprint);
            assert_eq!(original.image_path, image.image_path);
            assert_eq!(db.clipboard_detail("image").unwrap().title, "OCR title");
            db.clipboard_update("image", "delete").unwrap();
        }
        {
            let db = HistoryDatabase::open(&path).unwrap();
            let page = db.clipboard_page("école", false, None, 20).unwrap();
            assert_eq!(page.total_records, 1);
            assert_eq!(page.records[0].id, "extracted");
            assert_eq!(db.connection.query_row("SELECT count(*) FROM clipboard_ocr_search WHERE clipboard_ocr_search MATCH 'école'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn ocr_search_replaces_single_file_snapshot_and_cascades_on_cleanup() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let paths = serde_json::to_string(&[file_path("sample.png")]).unwrap();
        let mut file = record("file", &paths);
        file.kind = "files".into();
        db.clipboard_insert(&file).unwrap();
        let before = db.clipboard_storage_bytes().unwrap();
        for (id, text) in [("old", "Old wording"), ("new", "New wording")] {
            db.clipboard_insert_ocr_result("file", &record(id, text))
                .unwrap();
        }
        for (query, count) in [("old wording", 0), ("new wording", 1), ("sample.png", 1)] {
            assert_eq!(
                db.clipboard_page_filtered(query, false, Some("files"), None, 20)
                    .unwrap()
                    .total_records,
                count
            );
        }
        assert_eq!(db.clipboard_record("file").unwrap().text, paths);
        assert!(db.clipboard_storage_bytes().unwrap() > before);
        let token = db
            .records_cleanup_preview(RecordView::Clipboard, None)
            .unwrap()
            .token;
        assert_eq!(db.clipboard_cleanup(None, 1, Some(&token)).unwrap().0, 3);
        assert_eq!(db.clipboard_storage_bytes().unwrap(), 0);
        assert_eq!(db.connection.query_row("SELECT count(*) FROM clipboard_ocr_search WHERE clipboard_ocr_search MATCH 'wording'", [], |row| row.get::<_, i64>(0)).unwrap(), 0);
    }

    #[test]
    fn ocr_search_rejects_invalid_input_and_rolls_back_if_extracted_text_cannot_be_saved() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        db.clipboard_insert(&image_record("image")).unwrap();
        db.clipboard_insert(&record("text", "Existing text"))
            .unwrap();
        let mut files = record(
            "multiple",
            &serde_json::to_string(&[file_path("a.png"), file_path("b.png")]).unwrap(),
        );
        files.kind = "files".into();
        db.clipboard_insert(&files).unwrap();
        for id in ["missing", "text", "multiple"] {
            assert_eq!(
                db.clipboard_insert_ocr_result(id, &record("new", "Rejected wording"))
                    .unwrap_err(),
                "invalid_argument"
            );
        }
        for text in [" \n", "bad\0text", &"x".repeat(MAX_TEXT_BYTES + 1)] {
            assert_eq!(
                db.clipboard_insert_ocr_result("image", &record("new", text))
                    .unwrap_err(),
                "invalid_argument"
            );
        }
        db.clipboard_insert_ocr_result("image", &record("first", "Previous wording"))
            .unwrap();
        assert!(db
            .clipboard_insert_ocr_result("image", &record("first", "Rejected wording"))
            .is_err());
        assert_eq!(
            db.clipboard_page("rejected wording", false, None, 20)
                .unwrap()
                .total_records,
            0
        );
        assert_eq!(
            db.clipboard_page("previous wording", false, None, 20)
                .unwrap()
                .total_records,
            2
        );
        assert_eq!(
            db.clipboard_page("", false, None, 20)
                .unwrap()
                .total_records,
            4
        );
    }

    #[test]
    fn file_representations_migrate_v4_without_changing_records_and_reopen() {
        let root = std::env::temp_dir().join(format!("nami-files-v4-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("history.sqlite3");
        let mut db = HistoryDatabase::open(&path).unwrap();
        let original = record("text", &"完整正文🌊".repeat(200));
        db.clipboard_insert(&original).unwrap();
        db.clipboard_edit("text", "Preserved title", None).unwrap();
        db.clipboard_update("text", "favorite").unwrap();
        db.clipboard_capture_set(true).unwrap();
        let mut image = image_record("image");
        image.source_app = Some(serde_json::json!({"name":"Synthetic source"}));
        db.clipboard_insert(&image).unwrap();
        let paths = vec![file_path("photo.png")];
        let mut files = record("files", &serde_json::to_string(&paths).unwrap());
        files.kind = "files".into();
        db.clipboard_insert(&files).unwrap();
        db.connection
            .execute_batch("DROP TABLE clipboard_text_formatting; DROP TABLE clipboard_ocr_search; DROP TABLE clipboard_ocr; DROP TABLE clipboard_title_search; DROP TABLE clipboard_titles; DROP TABLE clipboard_file_representations; DROP INDEX clipboard_image_resources; PRAGMA user_version=4;")
            .unwrap();
        drop(db);

        let mut db = HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            db.connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            super::super::VERSION
        );
        assert_eq!(db.clipboard_record("text").unwrap().text, original.text);
        assert!(db.clipboard_record("text").unwrap().is_favorite);
        assert_eq!(
            db.clipboard_detail("text").unwrap().title,
            "Preserved title"
        );
        assert_eq!(
            db.clipboard_record("image").unwrap().source_app,
            image.source_app
        );
        assert_eq!(
            db.clipboard_record("image").unwrap().image_path,
            image.image_path
        );
        assert_eq!(db.clipboard_record("files").unwrap().text, files.text);
        assert!(db.clipboard_capture_enabled().unwrap());
        assert_eq!(db.clipboard_file_representation("image").unwrap(), None);
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            3
        );
        let attachment = representation(&image, "photo.png");
        assert_eq!(
            db.clipboard_insert_with_file_representation(&image, Some(&attachment))
                .unwrap()
                .id,
            "image"
        );
        drop(db);
        let db = HistoryDatabase::open(&path).unwrap();
        assert_eq!(
            db.clipboard_file_representation("image").unwrap(),
            Some(attachment)
        );
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            3
        );
        drop(db);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn translation_copies_append_full_text_without_replacing_old_clipboard_records() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let full_text = "Long translation\n".repeat(400);
        let old = record("existing", &full_text);
        db.clipboard_insert(&old).unwrap();
        db.clipboard_insert_translation_copy(full_text.clone())
            .unwrap();
        db.clipboard_insert_translation_copy(full_text.clone())
            .unwrap();
        let page = db
            .clipboard_page("Long translation", false, None, 100)
            .unwrap();
        assert_eq!(page.total_records, 3);
        assert!(db.clipboard_record("existing").is_ok());
        for copy in &page.records[..2] {
            let details = db.clipboard_record(&copy.id).unwrap();
            assert_eq!(details.text, full_text);
            assert_eq!(details.source_app.unwrap()["name"], "Nami");
            assert!(!details.is_favorite && !details.is_pinned);
        }
    }

    #[test]
    fn clipboard_duplicate_preserves_original_and_restores_full_payload_by_new_id() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for (original, attachment) in [
            (
                record("text", &"Synthetic clipboard text ".repeat(30)),
                None,
            ),
            (
                image_record("image"),
                Some(representation(&image_record("image"), "original.png")),
            ),
        ] {
            db.clipboard_insert_with_file_representation(&original, attachment.as_ref())
                .unwrap();
            db.clipboard_edit(&original.id, "Original note", None)
                .unwrap();
            db.clipboard_update(&original.id, "favorite").unwrap();
            db.clipboard_update(&original.id, "pin").unwrap();
            let before = serde_json::to_value(db.clipboard_detail(&original.id).unwrap()).unwrap();
            db.clipboard_duplicate(&original.id).unwrap();
            db.clipboard_duplicate(&original.id).unwrap();
            let page = db
                .clipboard_page_filtered("", false, Some(&original.kind), None, 100)
                .unwrap();
            assert_eq!(page.total_records, 3);
            assert_eq!(page.records[0].id, original.id);
            assert_ne!(page.records[1].id, page.records[2].id);
            for row in &page.records[1..] {
                let copy = db.clipboard_detail(&row.id).unwrap();
                assert_eq!(copy.text, original.text);
                assert_eq!(copy.image_path, original.image_path);
                assert!(copy.title.is_empty() && !copy.is_favorite && !copy.is_pinned);
                assert_eq!(
                    db.clipboard_file_representation(&row.id).unwrap(),
                    attachment
                );
            }
            assert_eq!(
                serde_json::to_value(db.clipboard_detail(&original.id).unwrap()).unwrap(),
                before
            );
        }
        assert_eq!(
            db.clipboard_page("clipboard text", false, None, 100)
                .unwrap()
                .total_records,
            3
        );
    }

    #[test]
    fn clipboard_duplicate_plain_text_strips_rich_formatting_while_keeping_original() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let formatting = ClipboardTextFormatting {
            html: Some(b"<p><strong>Rich</strong> hello</p>".to_vec()),
            rtf: Some(br#"{\rtf1\ansi \b Rich\b0  hello}"#.to_vec()),
        };
        let mut rich = record("rich-src", "Rich hello");
        rich.fingerprint = formatting.fingerprint(&rich.text);
        db.clipboard_insert_capture(&rich, None, Some(&formatting))
            .unwrap();
        db.clipboard_edit("rich-src", "Styled note", None).unwrap();
        db.clipboard_update("rich-src", "favorite").unwrap();

        // Standard duplicate keeps rich formatting.
        db.clipboard_duplicate("rich-src").unwrap();
        // Plain-text duplicate drops HTML/RTF formatting and uses plain-text fingerprint.
        db.clipboard_duplicate_plain_text("rich-src").unwrap();

        let page = db.clipboard_page("", false, None, 100).unwrap();
        assert_eq!(page.total_records, 3);
        let plain_copy_id = &page.records[0].id;
        let rich_copy_id = &page.records[1].id;
        assert_eq!(page.records[2].id, "rich-src");

        let plain_record = db.clipboard_record(plain_copy_id).unwrap();
        let plain_detail = db.clipboard_detail(plain_copy_id).unwrap();
        assert_eq!(plain_record.text, "Rich hello");
        assert_eq!(
            plain_record.fingerprint,
            crate::clipboard::fingerprint("text", b"Rich hello")
        );
        assert_eq!(db.clipboard_text_formatting(plain_copy_id).unwrap(), None);
        assert_eq!(plain_detail.rich_text_format, None);
        assert_eq!(plain_detail.rich_text_html, None);
        assert!(
            plain_detail.title.is_empty() && !plain_detail.is_favorite && !plain_detail.is_pinned
        );

        let rich_copy_detail = db.clipboard_detail(rich_copy_id).unwrap();
        assert_eq!(
            db.clipboard_text_formatting(rich_copy_id).unwrap(),
            Some(formatting.clone())
        );
        assert_eq!(rich_copy_detail.rich_text_format, Some("html+rtf"));

        // Original rich record remains intact.
        let original_detail = db.clipboard_detail("rich-src").unwrap();
        assert_eq!(original_detail.rich_text_format, Some("html+rtf"));
        assert_eq!(original_detail.title, "Styled note");
        assert!(original_detail.is_favorite);
        assert_eq!(
            db.clipboard_text_formatting("rich-src").unwrap(),
            Some(formatting)
        );

        // Non-text records cannot be duplicated as plain text.
        let img = image_record("img-src");
        db.clipboard_insert(&img).unwrap();
        assert_eq!(
            db.clipboard_duplicate_plain_text("img-src").unwrap_err(),
            "invalid_argument"
        );
    }

    #[test]
    fn clipboard_summary_reports_has_file_paths_for_local_images_only() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let local_image = image_record("local-image");
        let original = representation(&local_image, "original.png");
        let raw_image = ClipboardRecord {
            id: "raw-image".into(),
            image_path: Some("clipboard-images/raw.png".into()),
            fingerprint: crate::clipboard::fingerprint("image", b"raw-image"),
            ..local_image.clone()
        };
        db.clipboard_insert(&raw_image).unwrap();
        db.clipboard_insert_with_file_representation(&local_image, Some(&original))
            .unwrap();
        let page = db
            .clipboard_page_filtered("", false, Some("image"), None, 10)
            .unwrap();
        assert!(
            page.records
                .iter()
                .find(|row| row.id == local_image.id)
                .unwrap()
                .has_file_paths
        );
        assert!(
            !page
                .records
                .iter()
                .find(|row| row.id == raw_image.id)
                .unwrap()
                .has_file_paths
        );
    }

    #[test]
    fn clipboard_summary_reports_has_rich_text_and_groups_with_text_kind() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let plain = record("plain-text", "Plain text");
        let formatting = ClipboardTextFormatting {
            html: Some(b"<p><strong>Rich</strong></p>".to_vec()),
            rtf: Some(br#"{\rtf1\ansi \b Rich}"#.to_vec()),
        };
        let mut rich = record("rich-text", "Rich");
        rich.fingerprint = formatting.fingerprint(&rich.text);
        let img = image_record("img");
        db.clipboard_insert(&plain).unwrap();
        db.clipboard_insert_capture(&rich, None, Some(&formatting))
            .unwrap();
        db.clipboard_insert(&img).unwrap();

        let text_page = db
            .clipboard_page_filtered("", false, Some("text"), None, 10)
            .unwrap();
        assert_eq!(text_page.total_records, 2);
        assert!(
            text_page
                .records
                .iter()
                .find(|row| row.id == "rich-text")
                .unwrap()
                .has_rich_text
        );
        assert!(
            !text_page
                .records
                .iter()
                .find(|row| row.id == "plain-text")
                .unwrap()
                .has_rich_text
        );

        let all_page = db.clipboard_page("", false, None, 10).unwrap();
        assert!(
            !all_page
                .records
                .iter()
                .find(|row| row.id == "img")
                .unwrap()
                .has_rich_text
        );

        // Plain-text copy of a rich record appears as plain text in summaries while original stays rich.
        db.clipboard_duplicate_plain_text("rich-text").unwrap();
        let updated = db
            .clipboard_page_filtered("", false, Some("text"), None, 10)
            .unwrap();
        assert_eq!(updated.total_records, 3);
        assert!(!updated.records[0].has_rich_text);
        assert!(
            updated
                .records
                .iter()
                .find(|row| row.id == "rich-text")
                .unwrap()
                .has_rich_text
        );
    }

    #[test]
    fn file_representations_replace_deduplicated_id_and_roll_back_atomically() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let image = image_record("image");
        let original = representation(&image, "original.png");
        db.clipboard_insert_with_file_representation(&image, Some(&original))
            .unwrap();
        let mut duplicate = image.clone();
        duplicate.id = "incoming-id".into();
        duplicate.date = "2026-11-16T12:00:00.000Z".into();
        duplicate.source_app = Some(serde_json::json!({"name":"Replacement source"}));
        let replacement = representation(&image, "replacement.png");
        db.connection.execute_batch("CREATE TRIGGER reject_attachment_update BEFORE UPDATE ON clipboard_file_representations BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        assert!(db
            .clipboard_insert_with_file_representation(&duplicate, Some(&replacement))
            .is_err());
        assert_eq!(db.clipboard_record("image").unwrap().date, image.date);
        assert_eq!(db.clipboard_record("image").unwrap().source_app, None);
        assert_eq!(
            db.clipboard_file_representation("image").unwrap(),
            Some(original)
        );
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM source_apps", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        db.connection.execute_batch("DROP TRIGGER reject_attachment_update; CREATE TRIGGER reject_attachment_insert BEFORE INSERT ON clipboard_file_representations BEGIN SELECT RAISE(ABORT,'synthetic failure'); END;").unwrap();
        let mut fresh = duplicate.clone();
        fresh.fingerprint = "new-image".into();
        assert!(db
            .clipboard_insert_with_file_representation(&fresh, Some(&replacement))
            .is_err());
        assert_eq!(db.clipboard_record("incoming-id").unwrap_err(), "not_found");
        assert_eq!(
            db.connection
                .query_row("SELECT count(*) FROM source_apps", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            1
        );
        db.connection
            .execute_batch("DROP TRIGGER reject_attachment_insert")
            .unwrap();
        assert_eq!(
            db.clipboard_insert_with_file_representation(&duplicate, Some(&replacement))
                .unwrap()
                .id,
            "image"
        );
        assert_eq!(
            db.clipboard_file_representation("image").unwrap(),
            Some(replacement)
        );
        assert_eq!(
            db.clipboard_file_representation("incoming-id").unwrap(),
            None
        );
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            1
        );
        assert_eq!(
            db.clipboard_page("replacement.png", false, None, 10)
                .unwrap()
                .records[0]
                .id,
            "image"
        );
        assert_eq!(
            db.clipboard_page("original.png", false, None, 10)
                .unwrap()
                .total_records,
            0
        );
        assert_eq!(db.clipboard_insert(&duplicate).unwrap().id, "image");
        assert_eq!(db.clipboard_file_representation("image").unwrap(), None);
        assert_eq!(
            db.clipboard_page("replacement.png", false, None, 10)
                .unwrap()
                .total_records,
            0
        );
    }

    #[test]
    fn file_representations_validate_paths_size_kind_and_stored_corruption() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let image = image_record("image");
        let valid = representation(&image, "photo.png");
        for paths in [
            Vec::new(),
            vec![file_path("first.png"), file_path("second.png")],
            vec![file_path("photo.png"); MAX_FILE_COUNT + 1],
            vec!["relative.png".into()],
            vec![file_path("bad\0.png")],
            vec![format!("/{}", "x".repeat(MAX_TEXT_BYTES))],
        ] {
            let invalid = ClipboardFileRepresentation {
                paths,
                ..valid.clone()
            };
            assert_eq!(
                db.clipboard_insert_with_file_representation(&image, Some(&invalid))
                    .unwrap_err(),
                "invalid_argument"
            );
        }
        for image_path in [
            "origin/ab/abcdef.png",
            "../outside.png",
            "origin/ab/wrong.jpg",
        ] {
            let invalid = ClipboardFileRepresentation {
                image_path: image_path.into(),
                ..valid.clone()
            };
            assert_eq!(
                db.clipboard_insert_with_file_representation(&image, Some(&invalid))
                    .unwrap_err(),
                "invalid_argument"
            );
        }
        let mismatched = ClipboardFileRepresentation {
            image_path: format!("origin/cd/{}.png", "cd".repeat(32)),
            ..valid.clone()
        };
        assert_eq!(
            db.clipboard_insert_with_file_representation(&image, Some(&mismatched))
                .unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_insert_with_file_representation(&record("text", "text"), Some(&valid))
                .unwrap_err(),
            "invalid_argument"
        );
        let mut files = record(
            "files",
            &serde_json::to_string(&vec![file_path("other.png")]).unwrap(),
        );
        files.kind = "files".into();
        assert_eq!(
            db.clipboard_insert_with_file_representation(&files, Some(&valid))
                .unwrap_err(),
            "invalid_argument"
        );
        assert_eq!(
            db.clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            0
        );
        db.clipboard_insert_with_file_representation(&image, Some(&valid))
            .unwrap();
        for corrupt in [
            "not JSON".to_owned(),
            "[]".to_owned(),
            "[\"relative.png\"]".to_owned(),
            "x".repeat(MAX_TEXT_BYTES + 1),
        ] {
            db.connection
                .execute(
                    "UPDATE clipboard_file_representations SET paths=?1",
                    [&corrupt],
                )
                .unwrap();
            assert_eq!(
                db.clipboard_file_representation("image").unwrap_err(),
                "storage_corrupt"
            );
        }
        db.connection
            .execute(
                "UPDATE clipboard_file_representations SET paths=?1,image_path='../invalid.png'",
                [serde_json::to_string(&valid.paths).unwrap()],
            )
            .unwrap();
        assert_eq!(
            db.clipboard_file_representation("image").unwrap_err(),
            "storage_corrupt"
        );
        db.connection
            .execute_batch("DROP TABLE clipboard_file_representations")
            .unwrap();
        let page = db
            .clipboard_page_filtered("", false, Some("image"), None, 100)
            .unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.records[0].kind, "image");
        assert!(db
            .clipboard_page_filtered("", false, Some("files"), None, 100)
            .unwrap()
            .records
            .is_empty());
        let started = std::time::Instant::now();
        for _ in 0..100 {
            assert_eq!(
                db.clipboard_page_filtered("", false, Some("image"), None, 100)
                    .unwrap()
                    .records
                    .len(),
                1
            );
        }
        println!(
            "isolated image first-page query, attachment table absent: 100 queries in {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn deleting_file_representations_cascades_and_preserves_shared_image_references() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let image = image_record("image");
        let attachment = representation(&image, "photo.png");
        let mut files = record("files", &serde_json::to_string(&attachment.paths).unwrap());
        files.kind = "files".into();
        db.clipboard_insert(&image).unwrap();
        db.clipboard_insert_with_file_representation(&files, Some(&attachment))
            .unwrap();
        let mut statement = db.connection.prepare("EXPLAIN QUERY PLAN SELECT EXISTS(SELECT 1 FROM clipboard_records WHERE image_path=?1 UNION ALL SELECT 1 FROM clipboard_file_representations WHERE image_path=?1)").unwrap();
        let plan = statement
            .query_map([&attachment.image_path], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
            .join("\n");
        drop(statement);
        assert!(
            plan.contains("clipboard_image_resources")
                && plan.contains("clipboard_file_image_resources"),
            "resource cleanup must use both indexes: {plan}"
        );
        assert!(db
            .clipboard_image_referenced(&attachment.image_path)
            .unwrap());
        db.clipboard_update("image", "delete").unwrap();
        assert!(db
            .clipboard_image_referenced(&attachment.image_path)
            .unwrap());
        assert_eq!(
            db.clipboard_file_representation("files").unwrap(),
            Some(attachment.clone())
        );
        db.clipboard_update("files", "delete").unwrap();
        assert!(!db
            .clipboard_image_referenced(&attachment.image_path)
            .unwrap());
        assert_eq!(db.clipboard_file_representation("files").unwrap(), None);
        assert_eq!(
            db.connection
                .query_row(
                    "SELECT count(*) FROM clipboard_file_representations",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        assert!(!db
            .clipboard_image_referenced("origin/ab/absent.png")
            .unwrap());
    }

    #[test]
    fn version_two_migration_preserves_all_content_and_rolls_back_on_bad_data() {
        for corrupt in [false, true] {
            let directory =
                std::env::temp_dir().join(format!("nami-clipboard-v2-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&directory).unwrap();
            let path = directory.join("test.sqlite3");
            let mut db = HistoryDatabase::open(&path).unwrap();
            db.connection.execute_batch("DROP TABLE clipboard_text_formatting; DROP TABLE clipboard_ocr_search; DROP TABLE clipboard_ocr; DROP TABLE clipboard_title_search; DROP TABLE clipboard_titles; DROP TABLE clipboard_file_representations; DROP TRIGGER clipboard_insert; DROP TRIGGER clipboard_delete; DROP TABLE clipboard_search; DROP TABLE clipboard_contents; DROP TABLE clipboard_records; DROP TABLE preferences;").unwrap();
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
    fn copying_pinned_records_beats_future_pin_order_for_buttons_and_capture() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let mut future = record("future", "future");
        future.date = "2099-01-01T00:00:00Z".into();
        future.is_pinned = true;
        let mut older = record("older", "older");
        older.is_pinned = true;
        db.clipboard_insert(&future).unwrap();
        db.clipboard_insert(&older).unwrap();
        db.clipboard_promote("older").unwrap();
        assert_eq!(
            db.clipboard_page("", false, None, 10).unwrap().records[0].id,
            "older"
        );
        db.clipboard_insert(&future).unwrap();
        assert_eq!(
            db.clipboard_page("", false, None, 10).unwrap().records[0].id,
            "future"
        );
        assert_eq!(db.clipboard_record("future").unwrap().date, future.date);
    }

    #[test]
    fn latest_pin_precedes_older_pins_without_changing_copy_date() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let mut newer = record("newer", "newer");
        newer.date = "2026-11-15T12:00:00Z".into();
        let older = record("older", "older");
        db.clipboard_insert(&newer).unwrap();
        db.clipboard_insert(&older).unwrap();
        db.clipboard_update("newer", "pin").unwrap();
        db.clipboard_update("older", "pin").unwrap();
        let first = db.clipboard_page("", false, None, 1).unwrap();
        assert_eq!(first.records[0].id, "older");
        assert_eq!(first.records[0].date, older.date);
        let second = db
            .clipboard_page("", false, first.cursor.as_ref(), 1)
            .unwrap();
        assert_eq!(second.records[0].id, "newer");
        assert!(!second.has_more);
        db.clipboard_update("older", "pin").unwrap();
        db.clipboard_update("newer", "pin").unwrap();
        assert_eq!(
            db.clipboard_page("", false, None, 10)
                .unwrap()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
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
    fn copying_moves_a_record_to_the_front_of_its_pin_group() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        for (id, date, pinned) in [
            ("pin-new", "2026-11-15T03:00:00.000Z", true),
            ("pin-old", "2026-11-15T01:00:00.000Z", true),
            ("plain-new", "2026-11-15T04:00:00.000Z", false),
            ("plain-old", "2026-11-15T02:00:00.000Z", false),
        ] {
            let mut item = record(id, id);
            item.date = date.into();
            item.is_pinned = pinned;
            item.is_favorite = pinned;
            db.clipboard_insert(&item).unwrap();
        }
        let before: i64 = db
            .connection
            .query_row(
                "SELECT created_at FROM clipboard_records WHERE id='pin-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        db.clipboard_promote("pin-old").unwrap();
        let promoted = db.clipboard_record("pin-old").unwrap();
        assert!(promoted.is_pinned && promoted.is_favorite);
        assert_eq!(promoted.text, "pin-old");
        assert!(promoted.date.as_str() > "2026-11-15T03:00:00.000Z");
        let created: i64 = db
            .connection
            .query_row(
                "SELECT created_at FROM clipboard_records WHERE id='pin-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(created > before);
        assert_eq!(
            db.clipboard_page("", false, None, 10)
                .unwrap()
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["pin-old", "pin-new", "plain-new", "plain-old"]
        );
        db.clipboard_promote("plain-old").unwrap();
        assert!(!db.clipboard_record("plain-old").unwrap().is_pinned);
        assert_eq!(
            db.clipboard_page("", false, None, 10)
                .unwrap()
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["pin-old", "pin-new", "plain-old", "plain-new"]
        );
        assert_eq!(db.clipboard_promote("missing").unwrap_err(), "not_found");
    }

    #[test]
    fn recopying_an_image_sorts_it_ahead_of_images_copied_earlier() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let mut older = image_record("older");
        older.date = "2026-11-17T01:00:00.000Z".into();
        older.fingerprint = fingerprint("image", b"older-image");
        older.image_path = Some(format!("origin/aa/{}.png", "aa".repeat(32)));
        let mut newer = image_record("newer");
        newer.date = "2026-11-17T02:00:00.000Z".into();
        newer.fingerprint = fingerprint("image", b"newer-image");
        newer.image_path = Some(format!("origin/bb/{}.png", "bb".repeat(32)));
        db.clipboard_insert(&older).unwrap();
        db.clipboard_insert(&newer).unwrap();
        let mut again = older.clone();
        again.id = "older-again".into();
        again.date = "2026-11-17T03:00:00.000Z".into();
        assert_eq!(db.clipboard_insert(&again).unwrap().id, "older");
        let mut fresh = image_record("fresh");
        fresh.date = "2026-11-17T04:00:00.000Z".into();
        fresh.fingerprint = fingerprint("image", b"fresh-image");
        fresh.image_path = Some(format!("origin/cc/{}.png", "cc".repeat(32)));
        assert_eq!(db.clipboard_insert(&fresh).unwrap().id, "fresh");
        assert_eq!(
            db.clipboard_page("", false, None, 10)
                .unwrap()
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["fresh", "older", "newer"]
        );
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
        for size in [10_000usize, 100_000] {
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
                let mut metadata=tx.prepare("INSERT INTO clipboard_records(id,kind,summary,date,created_at,is_favorite,is_pinned,source_app_id,fingerprint,image_path) VALUES(?1,?7,?2,'2026-11-15T00:00:00Z',?3,?4,?5,?6,?1,?8)").unwrap();
                let mut contents=tx.prepare("INSERT INTO clipboard_contents(sequence,text,search_folded) VALUES(?1,?2,?3)").unwrap();
                let mut ocr=tx.prepare("INSERT INTO clipboard_ocr(sequence,search_folded,original_text) VALUES(?1,?2,?3)").unwrap();
                let mut representations=tx.prepare("INSERT INTO clipboard_file_representations(sequence,paths,image_path) VALUES(?1,?2,?3)").unwrap();
                let image_path = format!("origin/00/{}.png", "0".repeat(64));
                let image_files =
                    serde_json::to_string(&vec![file_path("benchmark-image.png")]).unwrap();
                for index in 0..size {
                    let image = index % 7 == 0;
                    let text = if image {
                        String::new()
                    } else {
                        format!("common synthetic clipboard {index:06} {}", "x".repeat(1000))
                    };
                    metadata
                        .execute(params![
                            format!("item-{index}"),
                            text.chars().take(256).collect::<String>(),
                            index as i64,
                            index % 10 == 0,
                            index % 1000 == 0,
                            source_id,
                            if image { "image" } else { "text" },
                            image.then_some(&image_path)
                        ])
                        .unwrap();
                    let sequence = tx.last_insert_rowid();
                    contents
                        .execute(params![
                            sequence,
                            text,
                            if image { "benchmark-image.png" } else { &text }
                        ])
                        .unwrap();
                    if image {
                        let raw = format!("OCR-Token-{index:06} 海浪\nStatus code E247");
                        ocr.execute(params![sequence, raw.to_lowercase(), raw])
                            .unwrap();
                        representations
                            .execute(params![sequence, image_files, image_path])
                            .unwrap();
                    }
                }
                drop(metadata);
                drop(contents);
                drop(ocr);
                drop(representations);
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
                ("ocrUnique", "ocr-token-000007", false, None),
                ("ocrBroad", "e247", false, None),
                ("ocrShort", "海浪", false, None),
                ("imageName", "benchmark-image.png", false, None),
            ] {
                let mut times = Vec::new();
                for _ in 0..30 {
                    let started = std::time::Instant::now();
                    let page = db
                        .clipboard_page_filtered(query, favorite, kind, None, 100)
                        .unwrap();
                    if label == "ocrUnique" {
                        assert_eq!(page.total_records, 1);
                    } else if matches!(label, "ocrBroad" | "ocrShort" | "imageName") {
                        assert_eq!(page.total_records, size.div_ceil(7));
                    }
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
        let report = serde_json::json!({"scope":"isolated synthetic clipboard with one indexed image+file representation per seven rows; database+summary JSON, not click-to-frame or physical disk cold start","runs":reports});
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
    fn clipboard_persistence_deduplication_and_explicit_delete() {
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
        database.clipboard_capture_set(true).unwrap();
        drop(database);
        let mut database = HistoryDatabase::open(&path).unwrap();
        assert!(database.clipboard_capture_enabled().unwrap());
        assert_eq!(database.clipboard_record("first").unwrap().text, item.text);
        for (favorite, pinned) in [(true, false), (false, true), (true, true), (false, false)] {
            let mut item = item.clone();
            item.is_favorite = favorite;
            item.is_pinned = pinned;
            database.clipboard_update("first", "delete").unwrap();
            database.clipboard_insert(&item).unwrap();
            let stored = database.clipboard_record("first").unwrap();
            assert_eq!((stored.is_favorite, stored.is_pinned), (favorite, pinned));
        }
        database.clipboard_update("first", "delete").unwrap();
        assert_eq!(database.clipboard_record("first").unwrap_err(), "not_found");
        assert_eq!(
            database
                .clipboard_page("", false, None, 100)
                .unwrap()
                .total_records,
            0
        );
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
    fn translation_preferences_round_trip_without_replacing_clipboard_capture() {
        let mut database = HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(
            database
                .preference("translation_engine")
                .unwrap()
                .as_deref(),
            Some("edge")
        );
        assert!(!database.clipboard_capture_enabled().unwrap());
        database
            .preference_set("translation_primary", "ja")
            .unwrap();
        database
            .preference_set("translation_proxy_url", "http://127.0.0.1:7890")
            .unwrap();
        database
            .preference_set("translation_proxy_enabled", "true")
            .unwrap();
        database
            .preference_set("translation_shortcuts", "{\"translate\":null}")
            .unwrap();
        assert_eq!(
            database
                .preference("translation_primary")
                .unwrap()
                .as_deref(),
            Some("ja")
        );
        assert_eq!(
            database
                .preference("translation_proxy_url")
                .unwrap()
                .as_deref(),
            Some("http://127.0.0.1:7890")
        );
        assert!(database
            .preference_set("clipboard_capture", "true")
            .is_err());
        assert!(!database.clipboard_capture_enabled().unwrap());
    }

    #[test]
    fn migration_preserves_version_one_translation_records() {
        let directory = std::env::temp_dir().join(format!("nami-upgrade-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("test.sqlite3");
        let mut database = HistoryDatabase::open(&path).unwrap();
        let translation = crate::model::HistoryRecord {
            title: String::new(),
            id: "preserved-translation".into(),
            text: "synthetic original".into(),
            source_language: "auto".into(),
            translation: None,
            target_languages: vec!["en".into()],
            translations: vec!["synthetic translation".into()],
            date: "2026-11-14T12:00:00.000Z".into(),
            is_favorite: true,
            is_pinned: false,
            source_app: None,
        };
        database.insert(&translation, "edge", || true).unwrap();
        database.connection.execute_batch("DROP TABLE clipboard_text_formatting; DROP TABLE clipboard_ocr_search; DROP TABLE clipboard_ocr; DROP TABLE clipboard_title_search; DROP TABLE clipboard_titles; DROP TABLE clipboard_file_representations; DROP TRIGGER clipboard_insert; DROP TRIGGER clipboard_delete; DROP TABLE clipboard_search; DROP TABLE clipboard_contents; DROP TABLE clipboard_records; DROP TABLE preferences; PRAGMA user_version=1;").unwrap();
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

    #[test]
    fn clipboard_detail_includes_sanitized_rich_text_preview_and_clears_after_body_edit() {
        let mut db = HistoryDatabase::open(Path::new(":memory:")).unwrap();
        let html_item = record("rich-html", "Hello World");
        let formatting = ClipboardTextFormatting {
            html: Some(
                br#"<p onclick="alert(1)">Hello <strong>World</strong></p><script>evil()</script><img src="https://tracker.example/x.png">"#
                    .to_vec(),
            ),
            rtf: None,
        };
        db.clipboard_insert_capture(&html_item, None, Some(&formatting))
            .unwrap();
        let detail = db.clipboard_detail("rich-html").unwrap();
        assert_eq!(detail.rich_text_format, Some("html"));
        let preview = detail.rich_text_html.as_deref().unwrap();
        assert!(preview.contains("<strong>World</strong>"));
        assert!(!preview.contains("script"));
        assert!(!preview.contains("onclick"));
        assert!(!preview.contains("<img"));

        let rtf_item = record("rich-rtf", "Bold RTF");
        let rtf_formatting = ClipboardTextFormatting {
            html: None,
            rtf: Some(br#"{\rtf1\ansi \b Bold RTF}"#.to_vec()),
        };
        db.clipboard_insert_capture(&rtf_item, None, Some(&rtf_formatting))
            .unwrap();
        let rtf_detail = db.clipboard_detail("rich-rtf").unwrap();
        assert_eq!(rtf_detail.rich_text_format, Some("rtf"));
        assert!(rtf_detail
            .rich_text_html
            .as_deref()
            .unwrap()
            .contains("<strong>Bold RTF</strong>"));

        db.clipboard_edit("rich-html", "", Some("Edited plain text"))
            .unwrap();
        let edited_detail = db.clipboard_detail("rich-html").unwrap();
        assert_eq!(edited_detail.rich_text_format, None);
        assert_eq!(edited_detail.rich_text_html, None);
    }

    #[test]
    fn clipboard_storage_limit_evicts_oldest_unprotected_and_protects_flags_and_shared_images() {
        let root =
            std::env::temp_dir().join(format!("nami-storage-limit-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let db_path = root.join("history.sqlite3");
        let image_root = root.join("images");
        let mut db = HistoryDatabase::open(&db_path).unwrap();

        let mut shared_bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        shared_bytes.resize(4096, 0x42);
        let shared_path = crate::clipboard::store_image(&image_root, &shared_bytes, "png").unwrap();
        let mut solo_bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        solo_bytes.resize(6144, 0x99);
        let solo_path = crate::clipboard::store_image(&image_root, &solo_bytes, "png").unwrap();

        let mut old_shared = record("old-shared", "");
        old_shared.kind = "image".into();
        old_shared.date = "2026-11-01T00:00:00.000Z".into();
        old_shared.image_path = Some(shared_path.clone());
        old_shared.fingerprint = "fp-old-shared".into();
        db.clipboard_insert(&old_shared).unwrap();

        let mut old_solo = record("old-solo", "");
        old_solo.kind = "image".into();
        old_solo.date = "2026-11-02T00:00:00.000Z".into();
        old_solo.image_path = Some(solo_path.clone());
        old_solo.fingerprint = "fp-old-solo".into();
        db.clipboard_insert(&old_solo).unwrap();

        let mut pinned_shared = record("pinned-shared", "");
        pinned_shared.kind = "image".into();
        pinned_shared.date = "2026-11-03T00:00:00.000Z".into();
        pinned_shared.image_path = Some(shared_path.clone());
        pinned_shared.is_pinned = true;
        pinned_shared.fingerprint = "fp-pinned-shared".into();
        db.clipboard_insert(&pinned_shared).unwrap();

        let mut newest_text = record("newest-text", "keep newest");
        newest_text.date = "2026-11-04T00:00:00.000Z".into();
        db.clipboard_insert(&newest_text).unwrap();

        let policy = HistoryPolicy {
            limit: 500,
            max_age_days: None,
            max_storage_mb: Some(500),
        };
        assert_eq!(
            db.records_cleanup_preview(RecordView::Translation, Some(policy))
                .err()
                .as_deref(),
            Some("invalid_argument")
        );

        // Budget fits shared image (4096) + remaining DB rows (~600B), but NOT solo image (6144).
        // Evicting old-shared alone does not free shared_bytes (pinned-shared still holds a ref),
        // so both old-shared and old-solo must be evicted while pinned-shared and newest-text stay.
        let budget = 5500u64;
        let preview = db
            .records_cleanup_preview_with_budget(policy, Some(&image_root), budget)
            .unwrap();
        assert_eq!(preview.count, 2);
        let (removed, unreferenced) = db
            .clipboard_cleanup_with_budget(
                Some(policy),
                Some(&preview.token),
                Some(&image_root),
                Some(budget),
            )
            .unwrap();
        assert_eq!(removed, 2);
        assert_eq!(unreferenced, vec![solo_path]);
        assert!(db.clipboard_record("old-shared").is_err());
        assert!(db.clipboard_record("old-solo").is_err());
        assert!(db.clipboard_record("pinned-shared").is_ok());
        assert!(db.clipboard_record("newest-text").is_ok());
        drop(db);

        let reopened = HistoryDatabase::open(&db_path).unwrap();
        assert_eq!(
            reopened
                .records_history_policy(RecordView::Clipboard)
                .unwrap(),
            policy
        );
        drop(reopened);
        let _ = std::fs::remove_dir_all(root);
    }

    use std::path::Path;
}
