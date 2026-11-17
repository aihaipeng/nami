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
    invalid: Vec<crate::database::HistoryQuery>,
}

impl HistoryPageCache {
    pub fn get(
        &self,
        query: &crate::database::HistoryQuery,
    ) -> Option<crate::database::HistoryPage> {
        self.pages
            .iter()
            .find(|(key, _)| key == query && !self.invalid.contains(key))
            .map(|(_, page)| page.clone())
    }

    pub fn loaded_count(&self, query: &crate::database::HistoryQuery) -> usize {
        self.pages
            .iter()
            .find(|(key, _)| key == query)
            .map_or(0, |(_, page)| page.records.len())
    }

    pub fn insert(
        &mut self,
        query: crate::database::HistoryQuery,
        page: crate::database::HistoryPage,
    ) {
        // Retain the two already-browsed views for the current search without reading more rows.
        self.pages
            .retain(|(key, _)| key.text == query.text && key != &query);
        self.invalid
            .retain(|key| key.text == query.text && key != &query);
        self.pages.push((query, page));
    }

    pub fn clear(&mut self) {
        self.pages.clear();
        self.invalid.clear();
    }

    pub fn update_record(&mut self, previous: &HistoryRecord, next: Option<&HistoryRecord>) {
        for (query, page) in &mut self.pages {
            if self.invalid.contains(query) {
                continue;
            }
            if previous.is_pinned
                && next.is_some_and(|record| record.is_pinned && query.matches(record))
                && !query.matches(previous)
                && !page.records.iter().any(|record| record.id == previous.id)
            {
                self.invalid.push(query.clone());
                continue;
            }
            let was_match = query.matches(previous);
            let now_match = next.is_some_and(|record| query.matches(record));
            let loaded = page
                .records
                .iter()
                .position(|record| record.id == previous.id);
            if was_match && !now_match {
                page.total_records -= 1;
            } else if !was_match && now_match {
                page.total_records += 1;
            }
            if let Some(index) = loaded {
                if now_match {
                    page.records[index] = next.unwrap().clone();
                } else {
                    page.records.remove(index);
                }
            } else if let Some(record) = next.filter(|record| {
                now_match && (!was_match || record.is_pinned && !previous.is_pinned)
            }) {
                page.records.insert(0, record.clone());
            }
            if next.is_some_and(|record| record.is_pinned && !previous.is_pinned) {
                if let Some(index) = page
                    .records
                    .iter()
                    .position(|record| record.id == previous.id)
                {
                    let record = page.records.remove(index);
                    page.records.insert(0, record);
                }
            }
            order_records(&mut page.records);
            page.has_more &= page.records.len() < page.total_records;
        }
    }

    pub fn insert_record(&mut self, record: &HistoryRecord) {
        for (query, page) in &mut self.pages {
            if !query.matches(record) || page.records.iter().any(|item| item.id == record.id) {
                continue;
            }
            page.total_records += 1;
            page.records.insert(0, record.clone());
            order_records(&mut page.records);
        }
    }
}

fn order_records(records: &mut [HistoryRecord]) {
    records.sort_by_cached_key(|record| {
        let time = if record.is_pinned {
            0
        } else {
            chrono::DateTime::parse_from_rfc3339(&record.date)
                .map(|date| date.timestamp_millis())
                .unwrap_or_default()
        };
        (std::cmp::Reverse(record.is_pinned), std::cmp::Reverse(time))
    });
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
                    // Full detection details stay in the record fetched by ID.
                    translation: record.translation.as_ref().map(|metadata| {
                        crate::model::TranslationMetadata {
                            requested_source: metadata.requested_source.clone(),
                            engine: metadata.engine.clone(),
                            strategy_version: metadata.strategy_version,
                            detections: Vec::new(),
                        }
                    }),
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

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn record(&self, id: &str) -> Option<HistoryRecord> {
        self.indices
            .get(id)
            .and_then(|index| self.records.get(*index))
            .or_else(|| self.selected_record.as_ref().filter(|r| r.id == id))
            .cloned()
    }

    pub fn loaded_page(
        &self,
        cursor: Option<crate::database::HistoryCursor>,
    ) -> crate::database::HistoryPage {
        crate::database::HistoryPage {
            records: self.records.clone(),
            cursor,
            has_more: self.has_more,
            total_records: self.total_records,
        }
    }

    pub fn add(&mut self, record: HistoryRecord) -> Result<(), String> {
        if record.id.is_empty() || self.indices.contains_key(&record.id) {
            return Err("history_record".into());
        }
        self.records.insert(0, record);
        order_records(&mut self.records);
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
        if id.is_empty() || self.indices.contains_key(&id) {
            return Err("history_record".into());
        }
        let visible = query.matches(&record);
        if visible {
            self.add(record.clone())?;
        } else {
            self.revision += 1;
        }
        if current {
            self.selected_record = Some(record);
            if visible {
                self.select(Some(&id))?;
            } else {
                self.clear_visual_selection();
            }
        }
        Ok(())
    }

    pub fn has_more(&self) -> bool {
        self.has_more
    }
    pub fn selected_id(&self) -> Option<&str> {
        self.selected_id.as_deref()
    }

    pub fn clear_visual_selection(&mut self) {
        if self.selected_id.take().is_some() {
            self.revision += 1;
        }
    }

    pub fn restore_visual_selection(&mut self) {
        let id = self
            .selected_record
            .as_ref()
            .filter(|record| self.indices.contains_key(&record.id))
            .map(|record| record.id.clone());
        if self.selected_id != id {
            self.selected_id = id;
            self.revision += 1;
        }
    }

    pub fn select(&mut self, id: Option<&str>) -> Result<(), String> {
        if id.is_some_and(|id| self.record(id).is_none()) {
            return Err("not_found".into());
        }
        if self.selected_id.as_deref() != id || (id.is_none() && self.selected_record.is_some()) {
            self.selected_record = id.and_then(|id| self.record(id));
            self.selected_id = id.map(str::to_owned);
            self.revision += 1;
        }
        Ok(())
    }

    pub fn apply_page(&mut self, page: crate::database::HistoryPage, reset: bool) {
        if reset {
            self.selected_record = self
                .selected_id
                .as_deref()
                .and_then(|id| self.record(id))
                .or_else(|| self.selected_record.clone());
            self.records = page.records;
            self.generation += 1;
        } else {
            for record in page.records {
                if !self.indices.contains_key(&record.id) {
                    self.indices.insert(record.id.clone(), self.records.len());
                    self.records.push(record);
                }
            }
            order_records(&mut self.records);
        }
        self.indices = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        self.total_records = page.total_records;
        self.has_more = page.has_more;
        if self
            .selected_id
            .as_ref()
            .is_some_and(|id| !self.indices.contains_key(id))
        {
            self.clear_visual_selection();
        }
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
        self.update_for_query(id, action, &crate::database::HistoryQuery::default())
    }

    pub fn requires_update_query(
        &self,
        previous: &HistoryRecord,
        next: Option<&HistoryRecord>,
        query: &crate::database::HistoryQuery,
    ) -> bool {
        previous.is_pinned
            && !query.matches(previous)
            && next.is_some_and(|record| record.is_pinned && query.matches(record))
            && !self.indices.contains_key(&previous.id)
    }

    pub fn apply_updated_page(
        &mut self,
        page: crate::database::HistoryPage,
        id: &str,
        next: Option<HistoryRecord>,
    ) {
        self.apply_page(page, true);
        if self
            .selected_record
            .as_ref()
            .is_some_and(|record| record.id == id)
        {
            self.selected_record = next;
        }
    }

    pub fn update_for_query(
        &mut self,
        id: &str,
        action: &str,
        query: &crate::database::HistoryQuery,
    ) -> Result<(), String> {
        let (previous, next) = self.prepare_update(id, action)?;
        if self.requires_update_query(&previous, next.as_ref(), query) {
            return Err("history_refresh_required".into());
        }
        let was_match = query.matches(&previous);
        let now_match = next.as_ref().is_some_and(|record| query.matches(record));
        if let Some(index) = self.indices.get(id).copied() {
            if now_match {
                self.records[index] = next.clone().unwrap();
            } else {
                self.records.remove(index);
            }
        } else if !was_match && now_match {
            self.records.insert(0, next.clone().unwrap());
        }
        if next
            .as_ref()
            .is_some_and(|record| record.is_pinned && !previous.is_pinned)
        {
            if let Some(index) = self.records.iter().position(|record| record.id == id) {
                let record = self.records.remove(index);
                self.records.insert(0, record);
            }
        }
        order_records(&mut self.records);
        self.indices = self
            .records
            .iter()
            .enumerate()
            .map(|(i, r)| (r.id.clone(), i))
            .collect();
        if self
            .selected_record
            .as_ref()
            .is_some_and(|record| record.id == id)
        {
            self.selected_record = next;
        }
        if was_match && !now_match {
            self.total_records -= 1;
        } else if !was_match && now_match {
            self.total_records += 1;
        }
        if !now_match && self.selected_id.as_deref() == Some(id) {
            self.clear_visual_selection();
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
            translation: None,
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
    fn switching_filters_restores_deep_loaded_records_full_content_and_cursor() {
        let cursor = |sequence| {
            Some(crate::database::HistoryCursor {
                pinned: false,
                sequence,
                created_at: sequence,
            })
        };
        let all = crate::database::HistoryQuery::default();
        let favorites = crate::database::HistoryQuery {
            favorites_only: true,
            ..all.clone()
        };
        let mut store = HistoryStore::new(vec![]);
        store.apply_page(
            crate::database::HistoryPage {
                records: (0..100)
                    .map(|index| record(&format!("all-{index}")))
                    .collect(),
                cursor: cursor(100),
                has_more: true,
                total_records: 220,
            },
            true,
        );
        let mut deep = record("deep");
        deep.text = "full deep content ".repeat(100);
        let text = deep.text.clone();
        let mut next: Vec<_> = (100..199)
            .map(|index| record(&format!("all-{index}")))
            .collect();
        next.push(deep);
        store.apply_page(
            crate::database::HistoryPage {
                records: next,
                cursor: cursor(200),
                has_more: true,
                total_records: 220,
            },
            false,
        );
        let mut cache = HistoryPageCache::default();
        cache.insert(all.clone(), store.loaded_page(cursor(200)));
        cache.insert(
            favorites,
            crate::database::HistoryPage {
                records: vec![record("favorite")],
                cursor: cursor(1),
                has_more: false,
                total_records: 1,
            },
        );
        let restored = cache.get(&all).unwrap();
        assert_eq!(restored.cursor.as_ref().unwrap().sequence, 200);
        assert_eq!(restored.records.len(), 200);
        assert_eq!(restored.total_records, 220);
        assert!(restored.has_more);
        store.apply_page(restored, true);
        assert_eq!(store.record("deep").unwrap().text, text);
        assert_eq!(store.snapshot().records.len(), 200);

        let previous = store.record("all-150").unwrap();
        let mut favorite = previous.clone();
        favorite.is_favorite = true;
        cache.update_record(&previous, Some(&favorite));
        let all_after = cache.get(&all).unwrap();
        assert_eq!(all_after.records.len(), 200);
        assert_eq!(all_after.total_records, 220);
        assert_eq!(all_after.cursor.unwrap().sequence, 200);
        assert_eq!(all_after.records.last().unwrap().text, text);
        let favorite_query = crate::database::HistoryQuery {
            favorites_only: true,
            ..all.clone()
        };
        let favorites_after = cache.get(&favorite_query).unwrap();
        assert_eq!(favorites_after.total_records, 2);
        assert!(favorites_after
            .records
            .iter()
            .any(|record| record.id == favorite.id));
        cache.update_record(&favorite, Some(&previous));
        assert_eq!(cache.get(&favorite_query).unwrap().total_records, 1);
        assert!(!cache
            .get(&favorite_query)
            .unwrap()
            .records
            .iter()
            .any(|record| record.id == favorite.id));

        let mut new = record("new-result");
        new.date = "2026-11-16T12:00:00Z".into();
        cache.insert_record(&new);
        cache.insert_record(&new);
        let restored = cache.get(&all).unwrap();
        assert_eq!(restored.records.len(), 201);
        assert_eq!(restored.total_records, 221);
        assert_eq!(restored.cursor.as_ref().unwrap().sequence, 200);
        assert_eq!(restored.records[0].id, new.id);
        assert_eq!(cache.get(&favorite_query).unwrap().total_records, 1);
        store.apply_page(restored, true);
        assert_eq!(store.record("deep").unwrap().text, text);
    }

    #[test]
    fn requerying_new_favorite_pins_preserves_deep_page_and_continuation() {
        let mut database =
            crate::database::HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        let query = crate::database::HistoryQuery {
            favorites_only: true,
            ..Default::default()
        };
        for index in 0..205 {
            let mut item = record(&format!("item-{index:03}"));
            item.date = format!("2026-11-15T00:{:02}:{:02}Z", index / 60, index % 60);
            database.insert(&item, "edge", || true).unwrap();
            let mut next = item.clone();
            next.is_favorite = index > 0;
            database.update(&item, Some(&next)).unwrap();
        }
        for id in ["item-000", "item-204"] {
            let previous = database.record(id).unwrap().unwrap();
            let mut next = previous.clone();
            next.is_pinned = true;
            database.update(&previous, Some(&next)).unwrap();
        }
        let mut store = HistoryStore::new(vec![]);
        store.apply_page(database.page_prefix(&query, 200).unwrap(), true);
        let old = database.record("item-000").unwrap().unwrap();
        store.selected_record = Some(old.clone());
        store.selected_id = Some(old.id.clone());
        let mut next = old.clone();
        next.is_favorite = true;
        database.update(&old, Some(&next)).unwrap();
        let page = database
            .page_prefix(&query, store.snapshot().records.len() + 1)
            .unwrap();
        let cursor = page.cursor.clone();
        store.apply_updated_page(page, &old.id, Some(next));
        let snapshot = store.snapshot();
        assert_eq!(snapshot.records.len(), 201);
        assert_eq!(snapshot.records[0].id, "item-204");
        assert_eq!(snapshot.records[1].id, "item-000");
        assert!(store.record("item-000").unwrap().is_favorite);
        assert!(snapshot.has_more);
        store.apply_page(database.page(&query, cursor.as_ref(), 100).unwrap(), false);
        assert_eq!(store.snapshot().records.len(), 205);
        assert!(!store.has_more());
    }

    #[test]
    fn newly_favorited_existing_pin_requires_authoritative_order() {
        let favorites = crate::database::HistoryQuery {
            favorites_only: true,
            ..Default::default()
        };
        let mut old = record("old-pin");
        old.is_pinned = true;
        let mut new = record("new-pin");
        new.is_pinned = true;
        new.is_favorite = true;
        let mut store = HistoryStore::new(vec![new.clone()]);
        store.selected_id = Some(old.id.clone());
        store.selected_record = Some(old.clone());
        let mut cache = HistoryPageCache::default();
        cache.insert(
            crate::database::HistoryQuery::default(),
            store.loaded_page(None),
        );
        cache.insert(favorites.clone(), store.loaded_page(None));
        let mut favorite = old.clone();
        favorite.is_favorite = true;
        cache.update_record(&old, Some(&favorite));
        assert!(cache.get(&favorites).is_none());
        assert_eq!(cache.loaded_count(&favorites), 1);
        assert_eq!(
            cache
                .get(&crate::database::HistoryQuery::default())
                .unwrap()
                .records
                .iter()
                .map(|record| record.id.as_str())
                .collect::<Vec<_>>(),
            ["new-pin"]
        );
        assert_eq!(
            store.update_for_query(&old.id, "favorite", &favorites),
            Err("history_refresh_required".into())
        );
        assert_eq!(store.snapshot().records[0].id, "new-pin");
    }

    #[test]
    fn flag_updates_preserve_database_time_zone_order() {
        let mut newer = record("newer");
        newer.date = "2026-11-15T01:00:00Z".into();
        let mut older = record("older");
        older.date = "2026-11-15T08:00:00+08:00".into();
        let mut store = HistoryStore::new(vec![newer, older]);
        store.update("older", "favorite").unwrap();
        assert_eq!(
            store
                .snapshot()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );
    }

    #[test]
    fn paging_after_unpinning_the_cursor_keeps_every_record_in_order() {
        let mut database =
            crate::database::HistoryDatabase::open(std::path::Path::new(":memory:")).unwrap();
        for (id, date) in [
            ("newer", "2026-11-17T00:00:00Z"),
            ("middle", "2026-11-16T00:00:00Z"),
            ("older", "2026-11-15T00:00:00Z"),
        ] {
            let mut item = record(id);
            item.date = date.into();
            database.insert(&item, "edge", || true).unwrap();
        }
        for id in ["newer", "older"] {
            let previous = database.record(id).unwrap().unwrap();
            let mut next = previous.clone();
            next.is_pinned = true;
            database.update(&previous, Some(&next)).unwrap();
        }
        let query = crate::database::HistoryQuery::default();
        let first = database.page(&query, None, 1).unwrap();
        let mut cursor = first.cursor.clone();
        let mut store = HistoryStore::new(vec![]);
        store.apply_page(first, true);
        let (previous, next) = store.prepare_update("older", "pin").unwrap();
        database.update(&previous, next.as_ref()).unwrap();
        store.update("older", "pin").unwrap();
        while store.has_more() {
            let page = database.page(&query, cursor.as_ref(), 1).unwrap();
            cursor = page.cursor.clone();
            store.apply_page(page, false);
        }
        assert_eq!(
            store
                .snapshot()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "middle", "older"]
        );
        let mut newest = record("newest");
        newest.date = "2026-11-18T00:00:00Z".into();
        store.add_completed(newest, true, &query).unwrap();
        assert_eq!(
            store
                .snapshot()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "newest", "middle", "older"]
        );
    }

    #[test]
    fn latest_pin_order_survives_store_cache_favorites_and_new_records() {
        let query = crate::database::HistoryQuery::default();
        let mut newer = record("newer");
        newer.date = "2026-11-16T12:00:00Z".into();
        let older = record("older");
        let mut store = HistoryStore::new(vec![newer.clone(), older.clone()]);
        let mut cache = HistoryPageCache::default();
        cache.insert(query.clone(), store.loaded_page(None));
        for id in ["newer", "older"] {
            let (previous, next) = store.prepare_update(id, "pin").unwrap();
            store.update(id, "pin").unwrap();
            cache.update_record(&previous, next.as_ref());
        }
        assert_eq!(
            store
                .snapshot()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["older", "newer"]
        );
        assert_eq!(
            cache
                .get(&query)
                .unwrap()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["older", "newer"]
        );
        let (previous, next) = store.prepare_update("newer", "favorite").unwrap();
        store.update("newer", "favorite").unwrap();
        cache.update_record(&previous, next.as_ref());
        let mut newest = record("newest");
        newest.date = "2026-11-17T12:00:00Z".into();
        cache.insert_record(&newest);
        assert_eq!(
            cache
                .get(&query)
                .unwrap()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["older", "newer", "newest"]
        );
        for id in ["older", "newer"] {
            let (previous, next) = store.prepare_update(id, "pin").unwrap();
            store.update(id, "pin").unwrap();
            cache.update_record(&previous, next.as_ref());
        }
        assert_eq!(
            store
                .snapshot()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["newer", "older"]
        );
        assert_eq!(
            cache
                .get(&query)
                .unwrap()
                .records
                .iter()
                .map(|r| r.id.as_str())
                .collect::<Vec<_>>(),
            ["newest", "newer", "older"]
        );
    }

    #[test]
    fn cached_mutations_preserve_cursor_and_account_for_offscreen_flags_and_deletes() {
        let query = crate::database::HistoryQuery::default();
        let mut cache = HistoryPageCache::default();
        cache.insert(
            query.clone(),
            crate::database::HistoryPage {
                records: vec![record("loaded")],
                cursor: Some(crate::database::HistoryCursor {
                    pinned: false,
                    sequence: 10,
                    created_at: 10,
                }),
                has_more: true,
                total_records: 3,
            },
        );
        let offscreen = record("offscreen");
        let mut pinned = offscreen.clone();
        pinned.is_pinned = true;
        cache.update_record(&offscreen, Some(&pinned));
        let page = cache.get(&query).unwrap();
        assert_eq!(page.records[0].id, "offscreen");
        assert_eq!(page.total_records, 3);
        assert_eq!(page.cursor.unwrap().sequence, 10);
        cache.update_record(&record("not-loaded"), None);
        let page = cache.get(&query).unwrap();
        assert_eq!(page.records.len(), 2);
        assert_eq!(page.total_records, 2);
        assert!(!page.has_more);
        assert_eq!(page.cursor.unwrap().sequence, 10);
        cache.update_record(&record("loaded"), None);
        let page = cache.get(&query).unwrap();
        assert_eq!(page.records.len(), 1);
        assert_eq!(page.total_records, 1);
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
        assert_eq!(snapshot.total_records, 1);
        assert_eq!(snapshot.selected_id, None);
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
        assert_eq!(store.snapshot().total_records, 1);
    }

    #[test]
    fn snapshot_keeps_request_context_without_copying_segment_detections() {
        let mut value = serde_json::to_value(record("metadata")).unwrap();
        let detections = (0..2000)
            .map(|_| {
                serde_json::json!({
                    "rawLanguage":"zh-Hans", "language":"zh-CN", "score":1.0
                })
            })
            .collect::<Vec<_>>();
        value["translation"] = serde_json::json!({
            "requestedSource":"auto", "engine":"edge", "strategyVersion":1,
            "detections":detections
        });
        let item: HistoryRecord = serde_json::from_value(value).unwrap();
        let store = HistoryStore::new(vec![item]);
        let snapshot = serde_json::to_value(store.snapshot()).unwrap();
        assert_eq!(
            snapshot["records"][0]["translation"],
            serde_json::json!({
                "requestedSource":"auto", "engine":"edge", "strategyVersion":1,
                "detections":[]
            })
        );
        assert!(serde_json::to_vec(&snapshot).unwrap().len() < 2000);
        let full = serde_json::to_value(store.record("metadata").unwrap()).unwrap();
        assert_eq!(
            full["translation"]["detections"].as_array().unwrap().len(),
            2000
        );
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
    fn visual_selection_can_close_and_reopen_without_losing_full_detail() {
        let mut store = HistoryStore::new(vec![record("one"), record("two")]);
        store.select(Some("one")).unwrap();
        store.clear_visual_selection();
        assert_eq!(store.snapshot().selected_id, None);
        store.restore_visual_selection();
        assert_eq!(store.selected_id(), Some("one"));
        store.apply_page(
            crate::database::HistoryPage {
                records: vec![record("two")],
                cursor: None,
                has_more: false,
                total_records: 1,
            },
            true,
        );
        assert_eq!(store.selected_id(), None);
        assert!(store.record("one").is_some());
        store.restore_visual_selection();
        assert_eq!(store.selected_id(), None);
        store.apply_page(
            crate::database::HistoryPage {
                records: vec![record("one"), record("two")],
                cursor: None,
                has_more: false,
                total_records: 2,
            },
            true,
        );
        store.restore_visual_selection();
        assert_eq!(store.selected_id(), Some("one"));
        store.clear_visual_selection();
        store.select(None).unwrap();
        store.restore_visual_selection();
        assert_eq!(store.selected_id(), None);
    }

    #[test]
    fn favorite_filter_updates_rows_counts_and_associated_detail_without_requery() {
        let mut item = record("favorite");
        item.is_favorite = true;
        let mut store = HistoryStore::new(vec![item]);
        store.select(Some("favorite")).unwrap();
        let generation = store.snapshot().generation;
        let query = crate::database::HistoryQuery {
            text: String::new(),
            favorites_only: true,
        };
        store
            .update_for_query("favorite", "favorite", &query)
            .unwrap();
        let snapshot = store.snapshot();
        assert!(snapshot.records.is_empty());
        assert_eq!(snapshot.total_records, 0);
        assert_eq!(snapshot.selected_id, None);
        assert_eq!(snapshot.generation, generation);
        assert!(!store.record("favorite").unwrap().is_favorite);
        store
            .update_for_query("favorite", "favorite", &query)
            .unwrap();
        assert_eq!(store.snapshot().records.len(), 1);
        assert_eq!(store.snapshot().total_records, 1);
        store.restore_visual_selection();
        assert_eq!(store.selected_id(), Some("favorite"));
        store
            .add_completed(record("excluded"), true, &query)
            .unwrap();
        assert_eq!(store.snapshot().total_records, 1);
        assert_eq!(store.selected_id(), None);
        store
            .update_for_query("excluded", "delete", &query)
            .unwrap();
        assert_eq!(store.snapshot().total_records, 1);
        assert!(store.record("excluded").is_none());
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
