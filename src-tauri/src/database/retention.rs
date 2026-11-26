use super::{storage_error, HistoryDatabase};
use crate::clipboard::{valid_image_name, CleanupPreview, HistoryPolicy};
use rusqlite::Connection;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

const CLIPBOARD_TABLE: &str = "clipboard_records";
const CLIPBOARD_POLICY_KEY: &str = "clipboard_history_policy";

pub(super) fn candidates() -> &'static str {
    "WITH cleanup(sequence) AS (
        SELECT sequence FROM clipboard_records WHERE ?2 != 0 AND (?2 & CASE WHEN is_favorite=0 AND is_pinned=0 THEN 1 ELSE 2 * is_favorite + 4 * is_pinned END) != 0
        UNION ALL
        SELECT sequence FROM (SELECT sequence FROM clipboard_records WHERE ?1 IS NOT NULL AND is_favorite=0 AND is_pinned=0
            ORDER BY created_at DESC,sequence DESC LIMIT -1 OFFSET COALESCE(?1,0))
        UNION
        SELECT sequence FROM clipboard_records WHERE ?3 IS NOT NULL AND is_favorite=0 AND is_pinned=0 AND unixepoch(date,'subsec') < ?3/1000.0
    )"
}

pub(super) fn cleanup_parameters(
    policy: Option<HistoryPolicy>,
    categories: u8,
    now: i64,
) -> (Option<u32>, u8, Option<i64>) {
    (
        policy.and_then(|value| (value.limit > 0).then_some(value.limit)),
        categories,
        policy
            .and_then(|value| value.max_age_days)
            .map(|days| now - i64::from(days) * 86_400_000),
    )
}

#[derive(Clone, Copy)]
pub(super) struct CleanupContext<'a> {
    pub policy: Option<HistoryPolicy>,
    pub categories: u8,
    pub now: i64,
    pub image_root: Option<&'a Path>,
    pub storage_budget_override: Option<u64>,
}

pub(super) fn check_cleanup(
    connection: &Connection,
    ctx: CleanupContext<'_>,
    expected: Option<&str>,
) -> Result<(), String> {
    validate_scope(ctx.policy)?;
    if (ctx.policy.is_some() && ctx.categories != 0)
        || (ctx.policy.is_none() && (expected.is_none() || !(1..=7).contains(&ctx.categories)))
    {
        return Err("invalid_argument".into());
    }
    if let Some(expected) = expected {
        if cleanup_preview(connection, ctx)?.token != expected {
            return Err("clipboard_cleanup_changed".into());
        }
    }
    Ok(())
}

pub(super) fn persist_limit(
    connection: &Connection,
    policy: Option<HistoryPolicy>,
    expected: Option<&str>,
) -> Result<(), String> {
    if let (Some(policy), Some(_)) = (policy, expected) {
        let value = serde_json::to_string(&policy).map_err(|_| "storage_record")?;
        connection
            .execute(
                "INSERT INTO preferences(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                (CLIPBOARD_POLICY_KEY, value),
            )
            .map_err(storage_error)?;
    }
    Ok(())
}

fn validate_scope(policy: Option<HistoryPolicy>) -> Result<(), String> {
    if policy.is_some_and(|value| !value.valid()) {
        return Err("invalid_argument".into());
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
            candidates()
        ))
        .map_err(storage_error)?;
    let mut rows = query
        .query(cleanup_parameters(ctx.policy, ctx.categories, ctx.now))
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
    ctx: CleanupContext<'_>,
) -> Result<CleanupPreview, String> {
    let scoped = ctx.policy.is_some();
    let mut hash = Sha256::new();
    hash.update([u8::from(ctx.policy.is_none())]);
    hash.update(CLIPBOARD_TABLE);
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
    hash.update(
        ctx.policy
            .map_or(0, |value| value.auto_cleanup_interval_hours)
            .to_le_bytes(),
    );
    let mut counts = [0; 3];
    let mut count = 0;

    if scoped {
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
        let mut query = connection
            .prepare_cached("SELECT sequence, is_favorite + 2 * is_pinned FROM clipboard_records ORDER BY sequence")
            .map_err(storage_error)?;
        let mut rows = query.query([]).map_err(storage_error)?;
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
    })
}

impl HistoryDatabase {
    pub fn clipboard_history_policy(&self) -> Result<HistoryPolicy, String> {
        Ok(HistoryPolicy::from_store(
            self.preference(CLIPBOARD_POLICY_KEY)?,
        ))
    }

    pub fn records_cleanup_preview(
        &self,
        policy: Option<HistoryPolicy>,
    ) -> Result<CleanupPreview, String> {
        self.records_cleanup_preview_with_media(policy, None)
    }

    pub fn records_cleanup_preview_with_media(
        &self,
        policy: Option<HistoryPolicy>,
        image_root: Option<&Path>,
    ) -> Result<CleanupPreview, String> {
        validate_scope(policy)?;
        cleanup_preview(
            &self.connection,
            CleanupContext {
                policy,
                categories: 0,
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
        validate_scope(Some(policy))?;
        cleanup_preview(
            &self.connection,
            CleanupContext {
                policy: Some(policy),
                categories: 0,
                now: chrono::Utc::now().timestamp_millis(),
                image_root,
                storage_budget_override: Some(budget_bytes),
            },
        )
    }
}
