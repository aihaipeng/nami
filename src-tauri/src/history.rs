use crate::model::HistoryRecord;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySnapshot {
    pub revision: u64,
    pub records: Vec<HistoryRecord>,
    pub selected_id: Option<String>,
    pub total_records: usize,
    pub has_more: bool,
    pub generation: u64,
}

#[derive(Clone, Default)]
pub struct HistoryPageCache {
    pages: Vec<(crate::database::HistoryQuery, crate::database::HistoryPage)>,
}

impl HistoryPageCache {
    pub fn get(
        &self,
        query: &crate::database::HistoryQuery,
    ) -> Option<crate::database::HistoryPage> {
        self.pages
            .iter()
            .find(|(key, _)| key == query)
            .map(|(_, page)| page.clone())
    }

    pub fn insert(
        &mut self,
        query: crate::database::HistoryQuery,
        page: crate::database::HistoryPage,
    ) {
        // Only retain the two first pages for the current search, never accumulated scroll pages.
        self.pages
            .retain(|(key, _)| key.text == query.text && key != &query);
        self.pages.push((query, page));
    }

    pub fn clear(&mut self) {
        self.pages.clear();
    }
}

pub struct HistoryStore {
    records: Vec<HistoryRecord>,
    selected_id: Option<String>,
    revision: u64,
    indices: HashMap<String, usize>,
    selected_record: Option<HistoryRecord>,
    total_records: usize,
    has_more: bool,
    generation: u64,
}

impl HistoryStore {
    pub fn new(records: Vec<HistoryRecord>) -> Self {
        let total_records = records.len();
        let indices = records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        Self {
            records,
            indices,
            selected_record: None,
            total_records,
            has_more: false,
            generation: 0,
            selected_id: None,
            revision: 0,
        }
    }

    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            revision: self.revision,
            records: self
                .records
                .iter()
                .map(|record| HistoryRecord {
                    id: record.id.clone(),
                    text: record.text.chars().take(256).collect(),
                    source_language: record.source_language.clone(),
                    target_languages: record.target_languages.clone(),
                    translations: record
                        .translations
                        .iter()
                        .map(|text| text.chars().take(256).collect())
                        .collect(),
                    date: record.date.clone(),
                    is_favorite: record.is_favorite,
                    is_pinned: record.is_pinned,
                    source_app: record.source_app.clone(),
                })
                .collect(),
            selected_id: self.selected_id.clone(),
            total_records: self.total_records,
            has_more: self.has_more,
            generation: self.generation,
        }
    }

    pub fn record(&self, id: &str) -> Option<HistoryRecord> {
        self.indices
            .get(id)
            .and_then(|index| self.records.get(*index))
            .or_else(|| self.selected_record.as_ref().filter(|r| r.id == id))
            .cloned()
    }

    pub fn add(&mut self, record: HistoryRecord) -> Result<(), String> {
        if record.id.is_empty() || self.indices.contains_key(&record.id) {
            return Err("history_record".into());
        }
        self.records.insert(0, record);
        self.indices = self
            .records
            .iter()
            .enumerate()
            .map(|(i, record)| (record.id.clone(), i))
            .collect();
        self.total_records += 1;
        self.revision += 1;
        Ok(())
    }

    pub fn add_completed(
        &mut self,
        record: HistoryRecord,
        current: bool,
        query: &crate::database::HistoryQuery,
    ) -> Result<(), String> {
        let id = record.id.clone();
        let visible = query.matches(&record);
        if visible {
            self.add(record.clone())?;
        } else {
            self.total_records += 1;
            self.revision += 1;
        }
        if current {
            self.selected_record = Some(record);
            self.select(Some(&id))?;
        }
        Ok(())
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }
    pub fn selected_id(&self) -> Option<&str> {
        self.selected_id.as_deref()
    }

    pub fn select(&mut self, id: Option<&str>) -> Result<(), String> {
        if id.is_some_and(|id| self.record(id).is_none()) {
            return Err("not_found".into());
        }
        if self.selected_id.as_deref() != id {
            self.selected_record = id.and_then(|id| self.record(id));
            self.selected_id = id.map(str::to_owned);
            self.revision += 1;
        }
        Ok(())
    }

    pub fn apply_page(&mut self, page: crate::database::HistoryPage, reset: bool) {
        if reset {
            self.selected_record = self.selected_id.as_deref().and_then(|id| self.record(id));
            self.records = page.records;
            self.generation += 1;
        } else {
            for record in page.records {
                if !self.indices.contains_key(&record.id) {
                    self.indices.insert(record.id.clone(), self.records.len());
                    self.records.push(record);
                }
            }
        }
        self.indices = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        self.total_records = page.total_records;
        self.has_more = page.has_more;
        self.revision += 1;
    }

    pub fn prepare_update(
        &self,
        id: &str,
        action: &str,
    ) -> Result<(HistoryRecord, Option<HistoryRecord>), String> {
        if !matches!(action, "favorite" | "pin" | "delete") {
            return Err("invalid_argument".into());
        }
        let previous = self.record(id).ok_or("not_found")?;
        let mut next = previous.clone();
        match action {
            "favorite" => next.is_favorite = !next.is_favorite,
            "pin" => next.is_pinned = !next.is_pinned,
            "delete" if next.is_favorite || next.is_pinned => {
                return Err("history_protected".into())
            }
            "delete" => return Ok((previous, None)),
            _ => unreachable!(),
        }
        Ok((previous, Some(next)))
    }

    pub fn update(&mut self, id: &str, action: &str) -> Result<(), String> {
        let (_, next) = self.prepare_update(id, action)?;
        if let Some(index) = self.indices.get(id).copied() {
            if let Some(next) = next {
                self.records[index] = next;
            } else {
                self.records.remove(index);
                self.indices = self
                    .records
                    .iter()
                    .enumerate()
                    .map(|(i, r)| (r.id.clone(), i))
                    .collect();
            }
        } else {
            self.selected_record = next;
        }
        if action == "delete" {
            self.total_records -= 1;
            if self.selected_id.as_deref() == Some(id) {
                self.selected_id = None;
                self.selected_record = None;
            }
        }
        self.revision += 1;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str) -> HistoryRecord {
        HistoryRecord {
            id: id.into(),
            text: "Synthetic source".into(),
            source_language: "en".into(),
            target_languages: vec!["zh-CN".into()],
            translations: vec!["合成译文".into()],
            date: "2026-11-14T00:00:00.000Z".into(),
            is_favorite: false,
            is_pinned: false,
            source_app: None,
        }
    }

    #[test]
    fn filter_cache_keeps_two_database_first_pages_and_invalidates_on_clear_or_search() {
        let page = |id: &str, more| crate::database::HistoryPage {
            records: vec![record(id)],
            cursor: None,
            has_more: more,
            total_records: 101,
        };
        let all = crate::database::HistoryQuery::default();
        let favorite = crate::database::HistoryQuery {
            favorites_only: true,
            ..all.clone()
        };
        let mut cache = HistoryPageCache::default();
        cache.insert(all.clone(), page("newest", true));
        assert!(cache.get(&favorite).is_none());
        cache.insert(favorite.clone(), page("favorite-beyond-first-page", false));
        assert_eq!(cache.get(&all).unwrap().records[0].id, "newest");
        assert!(cache.get(&all).unwrap().has_more);
        assert_eq!(
            cache.get(&favorite).unwrap().records[0].id,
            "favorite-beyond-first-page"
        );
        let search = crate::database::HistoryQuery {
            text: "another search".into(),
            favorites_only: false,
        };
        cache.insert(search.clone(), page("search-hit", false));
        assert!(cache.get(&all).is_none());
        assert!(cache.get(&favorite).is_none());
        assert!(cache.get(&search).is_some());
        cache.clear();
        assert!(cache.get(&search).is_none());
    }

    #[test]
    fn completed_records_respect_active_global_query_and_retain_selected_detail() {
        let mut store = HistoryStore::new(vec![]);
        let query = crate::database::HistoryQuery {
            text: "alpha".into(),
            favorites_only: false,
        };
        let mut alpha = record("alpha");
        alpha.text = "alpha text".into();
        store.add_completed(alpha, true, &query).unwrap();
        let mut beta = record("beta");
        beta.text = "unrelated beta".into();
        store.add_completed(beta.clone(), true, &query).unwrap();
        let snapshot = store.snapshot();
        assert_eq!(snapshot.records.len(), 1);
        assert_eq!(snapshot.total_records, 2);
        assert_eq!(snapshot.selected_id.as_deref(), Some("beta"));
        assert_eq!(store.record("beta").unwrap().text, beta.text);
        store.update("beta", "favorite").unwrap();
        assert!(store.record("beta").unwrap().is_favorite);
        let favorites = crate::database::HistoryQuery {
            text: String::new(),
            favorites_only: true,
        };
        store
            .add_completed(record("not-favorite"), false, &favorites)
            .unwrap();
        assert_eq!(store.snapshot().records.len(), 1);
        assert_eq!(store.snapshot().total_records, 3);
    }

    #[test]
    fn list_snapshots_bound_long_text_while_detail_and_copy_remain_complete() {
        let mut item = record("long");
        item.text = "中文🐋 ".repeat(20_000);
        item.translations = vec!["long translation ".repeat(5000)];
        let store = HistoryStore::new(vec![item.clone()]);
        let snapshot = store.snapshot();
        assert_eq!(snapshot.records[0].text.chars().count(), 256);
        assert_eq!(snapshot.records[0].translations[0].chars().count(), 256);
        assert!(serde_json::to_vec(&snapshot).unwrap().len() < 5000);
        let detail = store.record("long").unwrap();
        assert_eq!(detail.text, item.text);
        assert_eq!(detail.translations, item.translations);
    }

    #[test]
    fn committed_stale_translation_is_kept_without_overriding_new_selection() {
        let mut store = HistoryStore::new(vec![record("current")]);
        store.select(Some("current")).unwrap();
        store
            .add_completed(
                record("stale"),
                false,
                &crate::database::HistoryQuery::default(),
            )
            .unwrap();
        assert!(store.record("stale").is_some());
        assert_eq!(store.snapshot().selected_id.as_deref(), Some("current"));
        store
            .add_completed(
                record("newest"),
                true,
                &crate::database::HistoryQuery::default(),
            )
            .unwrap();
        assert_eq!(store.snapshot().selected_id.as_deref(), Some("newest"));
    }

    #[test]
    fn preparing_a_change_does_not_publish_or_mutate_before_storage_commit() {
        let store = HistoryStore::new(vec![record("one")]);
        let (previous, next) = store.prepare_update("one", "favorite").unwrap();
        assert!(!previous.is_favorite);
        assert!(next.unwrap().is_favorite);
        assert!(!store.record("one").unwrap().is_favorite);
        assert_eq!(store.snapshot().revision, 0);
        let (_, deleted) = store.prepare_update("one", "delete").unwrap();
        assert!(deleted.is_none());
        assert!(store.record("one").is_some());
    }

    #[test]
    fn snapshots_are_owned_and_updates_keep_complete_record() {
        let mut store = HistoryStore::new(Vec::new());
        store.add(record("one")).unwrap();
        let before = store.snapshot();
        store.update("one", "favorite").unwrap();
        store.update("one", "pin").unwrap();
        let after = store.snapshot();
        assert_eq!(after.revision, 3);
        assert!(!before.records[0].is_favorite);
        assert!(!before.records[0].is_pinned);
        assert!(after.records[0].is_favorite && after.records[0].is_pinned);
        assert_eq!(after.records[0].text, before.records[0].text);
        assert_eq!(
            after.records[0].translations,
            before.records[0].translations
        );
        assert!(store.record("missing").is_none());
    }

    #[test]
    fn invalid_operations_leave_records_selection_and_revision_unchanged() {
        let mut store = HistoryStore::new(vec![record("one")]);
        store.select(Some("one")).unwrap();
        let revision = store.snapshot().revision;
        assert!(store.add(record("")).is_err());
        assert!(store.add(record("one")).is_err());
        assert!(store.select(Some("missing")).is_err());
        assert!(store.select(Some("")).is_err());
        assert!(store.update("missing", "delete").is_err());
        assert!(store.update("one", "unknown").is_err());
        store.select(Some("one")).unwrap();
        let after = store.snapshot();
        assert_eq!(after.revision, revision);
        assert_eq!(after.records.len(), 1);
        assert_eq!(after.selected_id.as_deref(), Some("one"));
    }

    #[test]
    fn favorite_and_pinned_records_are_protected_until_both_flags_cleared() {
        let mut store = HistoryStore::new(vec![record("one")]);
        for field in ["favorite", "pin"] {
            store.update("one", field).unwrap();
            let revision = store.snapshot().revision;
            assert_eq!(
                store.update("one", "delete"),
                Err("history_protected".into())
            );
            assert_eq!(store.snapshot().revision, revision);
            store.update("one", field).unwrap();
        }
        store.select(Some("one")).unwrap();
        store.update("one", "delete").unwrap();
        assert!(store.snapshot().records.is_empty());
        assert_eq!(store.snapshot().selected_id, None);
    }

    #[test]
    fn deleting_another_record_keeps_selection_and_clear_selection_is_explicit() {
        let mut store = HistoryStore::new(vec![record("one"), record("two")]);
        store.select(Some("one")).unwrap();
        store.update("two", "delete").unwrap();
        assert_eq!(store.snapshot().selected_id.as_deref(), Some("one"));
        store.select(None).unwrap();
        assert_eq!(store.snapshot().selected_id, None);
    }
}
