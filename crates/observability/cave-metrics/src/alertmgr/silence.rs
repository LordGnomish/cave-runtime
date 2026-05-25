// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! In-process silence store.

use super::model::{Silence, SilenceMatcher, SilenceStatus};
use crate::model::Labels;
use parking_lot::RwLock;
use std::collections::HashMap;
use uuid::Uuid;

pub struct SilenceStore {
    silences: RwLock<HashMap<String, Silence>>,
}

impl SilenceStore {
    pub fn new() -> Self {
        Self {
            silences: RwLock::new(HashMap::new()),
        }
    }

    pub fn create(&self, mut silence: Silence) -> String {
        if silence.id.is_empty() {
            silence.id = Uuid::new_v4().to_string();
        }
        let id = silence.id.clone();
        self.silences.write().insert(id.clone(), silence);
        id
    }

    pub fn get(&self, id: &str) -> Option<Silence> {
        self.silences.read().get(id).cloned()
    }

    pub fn list(&self) -> Vec<Silence> {
        self.silences.read().values().cloned().collect()
    }

    pub fn expire(&self, id: &str) -> bool {
        let mut store = self.silences.write();
        if let Some(s) = store.get_mut(id) {
            s.status.state = "expired".to_string();
            true
        } else {
            false
        }
    }

    /// Check if any active silence matches these labels.
    pub fn is_silenced(&self, labels: &Labels, now_rfc3339: &str) -> bool {
        self.silences
            .read()
            .values()
            .any(|s| s.is_active(now_rfc3339) && s.matches(labels))
    }
}

impl Default for SilenceStore {
    fn default() -> Self {
        Self::new()
    }
}
