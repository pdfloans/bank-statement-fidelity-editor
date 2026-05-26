//! Undo / Redo + Change History System
//! Supports multi-page bank statements with visual snapshots.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangeRecord {
    pub id: u64,
    pub timestamp: String,
    pub page: usize,
    pub old_text: String,
    pub new_text: String,
    pub bbox: [f32; 4],
    pub description: String,
    pub snapshot_path: Option<PathBuf>,
    pub provenance: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChangeHistory {
    history: Vec<ChangeRecord>,
    current_index: usize, // Points to the next change to redo (index in history)
    #[serde(skip)]
    next_id: AtomicU64,
}

// Custom Clone implementation for ChangeHistory due to AtomicU64
impl Clone for ChangeHistory {
    fn clone(&self) -> Self {
        Self {
            history: self.history.clone(),
            current_index: self.current_index,
            next_id: AtomicU64::new(self.next_id.load(Ordering::SeqCst)),
        }
    }
}

impl ChangeHistory {
    pub fn new() -> Self {
        Self {
            history: Vec::new(),
            current_index: 0,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn push_change(&mut self, page: usize, old_text: String, new_text: String, bbox: [f32; 4], description: String) {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let timestamp = chrono::Utc::now().to_rfc3339();

        let record = ChangeRecord {
            id,
            timestamp,
            page,
            old_text,
            new_text,
            bbox,
            description,
            snapshot_path: None,
            provenance: "Manual".into(),
        };

        self.history.truncate(self.current_index);
        self.history.push(record);
        self.current_index = self.history.len();
    }

    pub fn create_record(
        &self,
        page: usize,
        old_text: String,
        new_text: String,
        bbox: [f32; 4],
        description: String,
        snapshot_path: Option<PathBuf>,
    ) -> ChangeRecord {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let timestamp = chrono::Utc::now().to_rfc3339();

        ChangeRecord {
            id,
            timestamp,
            page,
            old_text,
            new_text,
            bbox,
            description,
            snapshot_path,
            provenance: "Manual".into(),
        }
    }

    pub fn push_record(&mut self, record: ChangeRecord) {
        self.history.truncate(self.current_index);
        self.history.push(record);
        self.current_index = self.history.len();
    }

    pub fn push_change_with_snapshot(
        &mut self,
        page: usize,
        old_text: String,
        new_text: String,
        bbox: [f32; 4],
        description: String,
        snapshot_path: PathBuf,
    ) -> ChangeRecord {
        let record = self.create_record(page, old_text, new_text, bbox, description, Some(snapshot_path));
        self.push_record(record.clone());
        record
    }

    pub fn undo(&mut self) -> Option<ChangeRecord> {
        if self.current_index == 0 {
            return None;
        }

        self.current_index -= 1;
        Some(self.history[self.current_index].clone())
    }

    pub fn redo(&mut self) -> Option<ChangeRecord> {
        if self.current_index >= self.history.len() {
            return None;
        }

        let record = self.history[self.current_index].clone();
        self.current_index += 1;
        Some(record)
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "schema_version": 1,
            "changes": self.history
        })
    }

    pub fn to_json_pretty_string(&self) -> String {
        serde_json::to_string_pretty(&self.to_json()).unwrap_or_default()
    }

    pub fn get_history(&self) -> Vec<ChangeRecord> {
        self.history.clone()
    }

    pub fn can_undo(&self) -> bool {
        self.current_index > 0
    }

    pub fn can_redo(&self) -> bool {
        self.current_index < self.history.len()
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_monotonic_within_history() {
        let mut history = ChangeHistory::new();
        history.push_change(0, "old".into(), "new".into(), [0.0; 4], "desc".into());
        let id1 = history.get_history()[0].id;
        
        let rec2 = history.create_record(0, "old2".into(), "new2".into(), [0.0; 4], "desc2".into(), None);
        let id2 = rec2.id;
        
        assert!(id2 > id1);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
    }
}
