use super::{storage_error, HistoryDatabase};
use crate::{
    clipboard::{CleanupPreview, HistoryPolicy},
    model::{HistoryRecord, RecordView},
};
use rusqlite::{Connection, TransactionBehavior};
use sha2::{Digest, Sha256};

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
    format!("WITH cleanup(sequence) AS (
        SELECT sequence FROM {table} WHERE ?2 != 0 AND (?2 & CASE WHEN is_favorite=0 AND is_pinned=0 THEN 1 ELSE 2 * is_favorite + 4 * is_pinned END) != 0
        UNION ALL
        SELECT sequence FROM (SELECT sequence FROM {table} WHERE ?1 IS NOT NULL AND is_favorite=0 AND is_pinned=0
            ORDER BY created_at DESC,sequence DESC LIMIT -1 OFFSET COALESCE(?1,0))
    )")
}

pub(super) fn cleanup_parameters(
    policy: Option<HistoryPolicy>,
    categories: u8,
) -> (Option<u32>, u8) {
    (policy.map(|value| value.limit), categories)
}

pub(super) fn check_cleanup(
    connection: &Connection,
    view: RecordView,
    policy: Option<HistoryPolicy>,
    categories: u8,
    expected: Option<&str>,
) -> Result<(), String> {
    if policy.is_some_and(|value| !value.valid())
        || (policy.is_some() && categories != 0)
        || (policy.is_none() && (!(1..=7).contains(&categories) || expected.is_none()))
    {
        return Err("invalid_argument".into());
    }
    if let Some(expected) = expected {
        if cleanup_preview(connection, view, policy)?.token != expected {
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

fn cleanup_preview(
    connection: &Connection,
    view: RecordView,
    policy: Option<HistoryPolicy>,
) -> Result<CleanupPreview, String> {
    let mut query = if policy.is_some() {
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
    let mut rows = if policy.is_some() {
        query.query(cleanup_parameters(policy, 0))
    } else {
        query.query([])
    }
    .map_err(storage_error)?;
    let mut hash = Sha256::new();
    hash.update([u8::from(policy.is_none())]);
    hash.update(table(view));
    hash.update(policy.map_or(0, |value| value.limit).to_le_bytes());
    let mut counts = [0; 3];
    let mut count = 0;
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
    Ok(CleanupPreview {
        count,
        token: format!("{:x}", hash.finalize()),
        categories: policy.is_none().then_some(counts),
    })
}

fn delete_translations(
    connection: &Connection,
    policy: Option<HistoryPolicy>,
    categories: u8,
) -> Result<Vec<String>, String> {
    let mut query = connection.prepare_cached(&format!("{} DELETE FROM translation_records WHERE sequence IN (SELECT sequence FROM cleanup) RETURNING id", candidates(RecordView::Translation))).map_err(storage_error)?;
    let rows = query
        .query_map(cleanup_parameters(policy, categories), |row| row.get(0))
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
        if policy.is_some_and(|value| !value.valid()) {
            return Err("invalid_argument".into());
        }
        cleanup_preview(&self.connection, view, policy)
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
        check_cleanup(&transaction, view, policy, categories, Some(expected))?;
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
