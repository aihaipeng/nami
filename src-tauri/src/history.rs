use crate::model::HistoryRecord;
use serde::Serialize;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HistorySnapshot {
    pub revision: u64,
    pub records: Vec<HistoryRecord>,
    pub selected_id: Option<String>,
}

pub struct HistoryStore {
    records: Vec<HistoryRecord>,
    selected_id: Option<String>,
    revision: u64,
}

impl HistoryStore {
    pub fn new(records: Vec<HistoryRecord>) -> Self {
        Self {
            records,
            selected_id: None,
            revision: 0,
        }
    }

    pub fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            revision: self.revision,
            records: self.records.clone(),
            selected_id: self.selected_id.clone(),
        }
    }

    pub fn record(&self, id: &str) -> Option<HistoryRecord> {
        self.records.iter().find(|record| record.id == id).cloned()
    }

    pub fn add(&mut self, record: HistoryRecord) -> Result<(), String> {
        if record.id.is_empty() || self.records.iter().any(|item| item.id == record.id) {
            return Err("history_record".into());
        }
        self.records.push(record);
        self.revision += 1;
        Ok(())
    }

    pub fn select(&mut self, id: Option<&str>) -> Result<(), String> {
        if id.is_some_and(|id| !self.records.iter().any(|record| record.id == id)) {
            return Err("not_found".into());
        }
        if self.selected_id.as_deref() != id {
            self.selected_id = id.map(str::to_owned);
            self.revision += 1;
        }
        Ok(())
    }

    pub fn update(&mut self, id: &str, action: &str) -> Result<(), String> {
        if !matches!(action, "favorite" | "pin" | "delete") {
            return Err("invalid_argument".into());
        }
        let index = self
            .records
            .iter()
            .position(|record| record.id == id)
            .ok_or("not_found")?;
        let record = &mut self.records[index];
        match action {
            "favorite" => record.is_favorite = !record.is_favorite,
            "pin" => record.is_pinned = !record.is_pinned,
            "delete" => {
                if record.is_favorite || record.is_pinned {
                    return Err("history_protected".into());
                }
                self.records.remove(index);
                if self.selected_id.as_deref() == Some(id) {
                    self.selected_id = None;
                }
            }
            _ => unreachable!(),
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
