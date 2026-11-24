use super::{storage_error, HistoryDatabase};
use crate::{
    clipboard::{valid_image_name, CleanupPreview, HistoryPolicy},
    model::{HistoryRecord, RecordView},
};
use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

fn table(view: RecordView) -> &'static str {
    match view {
        RecordView::Clipboard => "clipboard_records",
        RecordView::Translation => "translation_records",
    }
}

fn preference(view: RecordView) -> &'static str {
    match view {
        RecordView::Clipboard => "clipboard_history_policy",
        RecordView::Translation => "translation_history_policy",
    }
}

pub(super) fn candidates(view: RecordView) -> String {
    let table = table(view);
    let copy_time = match view {
        RecordView::Clipboard => "unixepoch(date,'subsec')",
        RecordView::Translation => "NULL", // Translation policies don't accept time windows.
    };
    format!("WITH cleanup(sequence) AS (
        SELECT sequence FROM {table} WHERE ?2 != 0 AND (?2 & CASE WHEN is_favorite=0 AND is_pinned=0 THEN 1 ELSE 2 * is_favorite + 4 * is_pinned END) != 0
        UNION ALL
        SELECT sequence FROM (SELECT sequence FROM {table} WHERE ?1 IS NOT NULL AND is_favorite=0 AND is_pinned=0
            ORDER BY created_at DESC,sequence DESC LIMIT -1 OFFSET COALESCE(?1,0))
        UNION
        SELECT sequence FROM {table} WHERE (?3 IS NOT NULL OR ?4 IS NOT NULL) AND is_favorite=0 AND is_pinned=0 AND ({copy_time} < ?3/1000.0 OR {copy_time} >= ?4/1000.0)
    )")
}

pub(super) fn cleanup_parameters(
    policy: Option<HistoryPolicy>,
    categories: u8,
    recent_since: Option<i64>,
    now: i64,
) -> (Option<u32>, u8, Option<i64>, Option<i64>) {
    (
        policy.map(|value| value.limit),
        categories,
        policy
            .and_then(|value| value.max_age_days)
            .map(|days| now - i64::from(days) * 86_400_000),
        recent_since,
    )
}

#[derive(Clone, Copy)]
pub(super) struct CleanupContext<'a> {
    pub policy: Option<HistoryPolicy>,
    pub categories: u8,
    pub recent_since: Option<i64>,
    pub now: i64,
    pub image_root: Option<&'a Path>,
    pub storage_budget_override: Option<u64>,
}

pub(super) fn check_cleanup(
    connection: &Connection,
    view: RecordView,
    ctx: CleanupContext<'_>,
    expected: Option<&str>,
) -> Result<(), String> {
    validate_scope(view, ctx.policy, ctx.recent_since)?;
    if ((ctx.policy.is_some() || ctx.recent_since.is_some()) && ctx.categories != 0)
        || (ctx.policy.is_none() && expected.is_none())
        || (ctx.policy.is_none()
            && ctx.recent_since.is_none()
            && !(1..=7).contains(&ctx.categories))
    {
        return Err("invalid_argument".into());
    }
    if let Some(expected) = expected {
        if cleanup_preview(connection, view, ctx)?.token != expected {
            return Err("clipboard_cleanup_changed".into());
        }
    }
    Ok(())
}

pub(super) fn persist_limit(
    connection: &Connection,
    view: RecordView,
    policy: Option<HistoryPolicy>,
    expected: Option<&str>,
) -> Result<(), String> {
    if let (Some(policy), Some(_)) = (policy, expected) {
        let value = serde_json::to_string(&policy).map_err(|_| "storage_record")?;
        connection.execute("INSERT INTO preferences(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value", (preference(view), value)).map_err(storage_error)?;
    }
    Ok(())
}

fn validate_scope(
    view: RecordView,
    policy: Option<HistoryPolicy>,
    recent_since: Option<i64>,
) -> Result<(), String> {
    if policy.is_some_and(|value| {
        !value.valid()
            || (view == RecordView::Translation
                && (value.max_age_days.is_some() || value.max_storage_mb.is_some()))
    }) || (recent_since.is_some() && (view != RecordView::Clipboard || policy.is_some()))
    {
        return Err("invalid_argument".into());
    }
    if let Some(since) = recent_since {
        let now = chrono::Utc::now().timestamp_millis();
        if !(now - 20 * 60_000..=now - 15 * 60_000).contains(&since) {
            return Err("clipboard_cleanup_changed".into());
        }
    }
    Ok(())
}

fn disk_image_size(image_root: &Path, relative: &str) -> u64 {
    if !valid_image_name(relative) {
        return 0;
    }
    std::fs::symlink_metadata(image_root.join(relative))
        .ok()
        .filter(|meta| meta.is_file())
        .map_or(0, |meta| meta.len())
}

pub(super) fn clipboard_storage_overflow_sequences(
    connection: &Connection,
    already_removed: &HashSet<i64>,
    max_bytes: u64,
    image_root: Option<&Path>,
) -> Result<Vec<i64>, String> {
    struct RowInfo {
        sequence: i64,
        protected: bool,
        db_bytes: u64,
        images: Vec<String>,
    }

    let mut stmt = connection
        .prepare_cached(
            "SELECT
                r.sequence,
                r.is_favorite,
                r.is_pinned,
                (
                    octet_length(r.id) + octet_length(r.kind) + octet_length(r.summary)
                    + octet_length(r.date) + octet_length(r.title) + octet_length(r.fingerprint)
                    + COALESCE(octet_length(r.image_path), 0)
                    + COALESCE(octet_length(c.text) + octet_length(c.search_folded), 0)
                    + COALESCE(octet_length(t.search_folded), 0)
                    + COALESCE(octet_length(o.search_folded) + COALESCE(octet_length(o.original_text), 0), 0)
                    + COALESCE(octet_length(f.paths) + octet_length(f.image_path), 0)
                    + COALESCE(COALESCE(length(fmt.html), 0) + COALESCE(length(fmt.rtf), 0), 0)
                ) AS db_bytes,
                r.image_path,
                f.image_path
            FROM clipboard_records r
            LEFT JOIN clipboard_contents c ON c.sequence = r.sequence
            LEFT JOIN clipboard_titles t ON t.sequence = r.sequence
            LEFT JOIN clipboard_ocr o ON o.sequence = r.sequence
            LEFT JOIN clipboard_file_representations f ON f.sequence = r.sequence
            LEFT JOIN clipboard_text_formatting fmt ON fmt.sequence = r.sequence
            ORDER BY r.created_at ASC, r.sequence ASC",
        )
        .map_err(storage_error)?;

    let mut rows = stmt.query([]).map_err(storage_error)?;
    let mut remaining = Vec::new();
    let mut image_refs: HashMap<String, usize> = HashMap::new();
    let mut total_bytes = 0u64;

    while let Some(row) = rows.next().map_err(storage_error)? {
        let sequence: i64 = row.get(0).map_err(storage_error)?;
        if already_removed.contains(&sequence) {
            continue;
        }
        let is_favorite: bool = row.get(1).map_err(storage_error)?;
        let is_pinned: bool = row.get(2).map_err(storage_error)?;
        let db_bytes_raw: i64 = row.get(3).map_err(storage_error)?;
        let db_bytes = u64::try_from(db_bytes_raw).map_err(|_| "storage_corrupt")?;
        let primary_image: Option<String> = row.get(4).map_err(storage_error)?;
        let file_image: Option<String> = row.get(5).map_err(storage_error)?;

        let mut images = Vec::new();
        if let Some(path) = primary_image {
            images.push(path);
        }
        if let Some(path) = file_image {
            if !images.iter().any(|existing| existing == &path) {
                images.push(path);
            }
        }
        for path in &images {
            *image_refs.entry(path.clone()).or_insert(0) += 1;
        }
        total_bytes = total_bytes.saturating_add(db_bytes);
        remaining.push(RowInfo {
            sequence,
            protected: is_favorite || is_pinned,
            db_bytes,
            images,
        });
    }

    let mut image_sizes: HashMap<String, u64> = HashMap::new();
    if let Some(root) = image_root {
        for path in image_refs.keys() {
            let size = disk_image_size(root, path);
            image_sizes.insert(path.clone(), size);
            total_bytes = total_bytes.saturating_add(size);
        }
    }

    if total_bytes <= max_bytes {
        return Ok(Vec::new());
    }

    let mut evicted = Vec::new();
    for row in remaining {
        if row.protected {
            continue;
        }
        evicted.push(row.sequence);
        total_bytes = total_bytes.saturating_sub(row.db_bytes);
        for path in row.images {
            if let Some(count) = image_refs.get_mut(&path) {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    if let Some(&size) = image_sizes.get(&path) {
                        total_bytes = total_bytes.saturating_sub(size);
                    }
                }
            }
        }
        if total_bytes <= max_bytes {
            break;
        }
    }

    Ok(evicted)
}

pub(super) fn clipboard_cleanup_sequences(
    connection: &Connection,
    ctx: CleanupContext<'_>,
) -> Result<Vec<i64>, String> {
    let mut query = connection
        .prepare_cached(&format!(
            "{} SELECT sequence FROM cleanup ORDER BY sequence",
            candidates(RecordView::Clipboard)
        ))
        .map_err(storage_error)?;
    let mut rows = query
        .query(cleanup_parameters(
            ctx.policy,
            ctx.categories,
            ctx.recent_since,
            ctx.now,
        ))
        .map_err(storage_error)?;
    let mut sequences = Vec::new();
    let mut seen = HashSet::new();
    while let Some(row) = rows.next().map_err(storage_error)? {
        let sequence: i64 = row.get(0).map_err(storage_error)?;
        if seen.insert(sequence) {
            sequences.push(sequence);
        }
    }
    let storage_budget = ctx
        .storage_budget_override
        .or_else(|| ctx.policy.and_then(|p| p.max_storage_bytes()));
    if let Some(max_bytes) = storage_budget {
        for sequence in
            clipboard_storage_overflow_sequences(connection, &seen, max_bytes, ctx.image_root)?
        {
            if seen.insert(sequence) {
                sequences.push(sequence);
            }
        }
    }
    sequences.sort_unstable();
    Ok(sequences)
}

fn cleanup_preview(
    connection: &Connection,
    view: RecordView,
    ctx: CleanupContext<'_>,
) -> Result<CleanupPreview, String> {
    let scoped = ctx.policy.is_some() || ctx.recent_since.is_some();
    let mut hash = Sha256::new();
    hash.update([u8::from(ctx.policy.is_none())]);
    hash.update(table(view));
    hash.update(ctx.policy.map_or(0, |value| value.limit).to_le_bytes());
    hash.update(
        ctx.policy
            .and_then(|value| value.max_age_days)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    hash.update(
        ctx.policy
            .and_then(|value| value.max_storage_mb)
            .unwrap_or(0)
            .to_le_bytes(),
    );
    hash.update(ctx.recent_since.unwrap_or(0).to_le_bytes());
    let mut counts = [0; 3];
    let mut count = 0;

    if scoped && view == RecordView::Clipboard {
        let sequences = clipboard_cleanup_sequences(
            connection,
            CleanupContext {
                categories: 0,
                ..ctx
            },
        )?;
        for sequence in sequences {
            hash.update(sequence.to_le_bytes());
            hash.update([0u8]);
            counts[0] += 1;
            count += 1;
        }
    } else {
        let mut query = if scoped {
            connection.prepare_cached(&format!(
                "{} SELECT sequence, 0 FROM cleanup ORDER BY sequence",
                candidates(view)
            ))
        } else {
            connection.prepare_cached(&format!(
                "SELECT sequence, is_favorite + 2 * is_pinned FROM {} ORDER BY sequence",
                table(view)
            ))
        }
        .map_err(storage_error)?;
        let mut rows = if scoped {
            query.query(cleanup_parameters(ctx.policy, 0, ctx.recent_since, ctx.now))
        } else {
            query.query([])
        }
        .map_err(storage_error)?;
        // Manual confirmation covers the entire displayed snapshot, including each row's category.
        while let Some(row) = rows.next().map_err(storage_error)? {
            let category = row.get::<_, u8>(1).map_err(storage_error)?;
            hash.update(row.get::<_, i64>(0).map_err(storage_error)?.to_le_bytes());
            hash.update([category]);
            match category {
                0 => counts[0] += 1,
                1 => counts[1] += 1,
                2 => counts[2] += 1,
                3 => {
                    counts[1] += 1;
                    counts[2] += 1;
                }
                _ => return Err("storage_corrupt".into()),
            }
            count += 1;
        }
    }
    Ok(CleanupPreview {
        count,
        token: format!("{:x}", hash.finalize()),
        categories: (!scoped).then_some(counts),
        recent_since: ctx.recent_since,
    })
}

fn delete_translations(
    connection: &Connection,
    policy: Option<HistoryPolicy>,
    categories: u8,
) -> Result<Vec<String>, String> {
    let mut query = connection.prepare_cached(&format!("{} DELETE FROM translation_records WHERE sequence IN (SELECT sequence FROM cleanup) RETURNING id", candidates(RecordView::Translation))).map_err(storage_error)?;
    let rows = query
        .query_map(cleanup_parameters(policy, categories, None, 0), |row| {
            row.get(0)
        })
        .map_err(storage_error)?;
    rows.collect::<rusqlite::Result<_>>().map_err(storage_error)
}

impl HistoryDatabase {
    pub fn records_history_policy(&self, view: RecordView) -> Result<HistoryPolicy, String> {
        Ok(HistoryPolicy::from_store(
            self.preference(preference(view))?,
        ))
    }

    pub fn records_cleanup_preview(
        &self,
        view: RecordView,
        policy: Option<HistoryPolicy>,
    ) -> Result<CleanupPreview, String> {
        self.records_cleanup_preview_scoped(view, policy, None)
    }

    pub fn records_cleanup_preview_scoped(
        &self,
        view: RecordView,
        policy: Option<HistoryPolicy>,
        recent_since: Option<i64>,
    ) -> Result<CleanupPreview, String> {
        self.records_cleanup_preview_with_media(view, policy, recent_since, None)
    }

    pub fn records_cleanup_preview_with_media(
        &self,
        view: RecordView,
        policy: Option<HistoryPolicy>,
        recent_since: Option<i64>,
        image_root: Option<&Path>,
    ) -> Result<CleanupPreview, String> {
        validate_scope(view, policy, recent_since)?;
        cleanup_preview(
            &self.connection,
            view,
            CleanupContext {
                policy,
                categories: 0,
                recent_since,
                now: chrono::Utc::now().timestamp_millis(),
                image_root,
                storage_budget_override: None,
            },
        )
    }

    #[cfg(test)]
    pub(crate) fn records_cleanup_preview_with_budget(
        &self,
        policy: HistoryPolicy,
        image_root: Option<&Path>,
        budget_bytes: u64,
    ) -> Result<CleanupPreview, String> {
        validate_scope(RecordView::Clipboard, Some(policy), None)?;
        cleanup_preview(
            &self.connection,
            RecordView::Clipboard,
            CleanupContext {
                policy: Some(policy),
                categories: 0,
                recent_since: None,
                now: chrono::Utc::now().timestamp_millis(),
                image_root,
                storage_budget_override: Some(budget_bytes),
            },
        )
    }

    pub fn translation_storage_bytes(&self) -> Result<u64, String> {
        self.connection.query_row("SELECT COALESCE(sum(octet_length(id)+octet_length(engine)+octet_length(record_json)+octet_length(source_folded)+octet_length(primary_folded)+octet_length(secondary_folded)),0) FROM translation_records", [], |row| row.get::<_, i64>(0)).map_err(storage_error).and_then(|bytes| u64::try_from(bytes).map_err(|_| "storage_corrupt".into()))
    }

    pub fn translation_cleanup(
        &mut self,
        policy: Option<HistoryPolicy>,
        categories: u8,
        expected: &str,
    ) -> Result<Vec<String>, String> {
        let view = RecordView::Translation;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        check_cleanup(
            &transaction,
            view,
            CleanupContext {
                policy,
                categories,
                recent_since: None,
                now: 0,
                image_root: None,
                storage_budget_override: None,
            },
            Some(expected),
        )?;
        let removed = delete_translations(&transaction, policy, categories)?;
        persist_limit(&transaction, view, policy, Some(expected))?;
        transaction.commit().map_err(storage_error)?;
        self.total_records -= removed.len();
        Ok(removed)
    }

    pub fn insert_retained(
        &mut self,
        record: &HistoryRecord,
        engine: &str,
        current: impl FnOnce() -> bool,
    ) -> Result<Vec<String>, String> {
        let policy = self.records_history_policy(RecordView::Translation)?;
        let transaction = self
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(storage_error)?;
        Self::insert_in(&transaction, record, engine)?;
        let removed = if self.total_records >= policy.limit as usize {
            delete_translations(&transaction, Some(policy), 0)?
        } else {
            vec![]
        };
        if !current() {
            return Err("cancelled".into());
        }
        transaction.commit().map_err(storage_error)?;
        self.total_records = self.total_records + 1 - removed.len();
        Ok(removed)
    }
}
