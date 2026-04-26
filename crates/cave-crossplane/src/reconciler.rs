//! Reconcile queue for resource reconciliation.

use std::collections::VecDeque;
use std::sync::Arc;
use crate::error::{CrossplaneError, CrossplaneResult};
use crate::models::{ReconcileItem, ReconcileStatus};
use chrono::Utc;
use tokio::sync::Mutex;
use uuid::Uuid;

pub struct ReconcileQueue {
    inner: Arc<Mutex<VecDeque<ReconcileItem>>>,
}

impl ReconcileQueue {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub async fn enqueue(
        &self,
        resource_kind: String,
        resource_name: String,
        namespace: Option<String>,
    ) -> ReconcileItem {
        let item = ReconcileItem {
            id: Uuid::new_v4(),
            resource_kind,
            resource_name,
            namespace,
            status: ReconcileStatus::Pending,
            attempts: 0,
            last_error: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let mut queue = self.inner.lock().await;
        queue.push_back(item.clone());
        item
    }

    /// Dequeue the next pending item, marking it as Running.
    pub async fn dequeue(&self) -> Option<ReconcileItem> {
        let mut queue = self.inner.lock().await;
        let pos = queue.iter().position(|i| {
            matches!(i.status, ReconcileStatus::Pending)
        })?;
        let mut item = queue.remove(pos)?;
        item.status = ReconcileStatus::Running;
        item.updated_at = Utc::now();
        queue.push_back(item.clone());
        Some(item)
    }

    pub async fn mark_succeeded(&self, id: Uuid) -> CrossplaneResult<()> {
        let mut queue = self.inner.lock().await;
        let item = queue
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| CrossplaneError::ReconcileError(format!("item not found: {}", id)))?;
        item.status = ReconcileStatus::Succeeded;
        item.updated_at = Utc::now();
        Ok(())
    }

    pub async fn mark_failed(&self, id: Uuid, error: &str) -> CrossplaneResult<()> {
        let mut queue = self.inner.lock().await;
        let item = queue
            .iter_mut()
            .find(|i| i.id == id)
            .ok_or_else(|| CrossplaneError::ReconcileError(format!("item not found: {}", id)))?;
        item.attempts += 1;
        item.last_error = Some(error.to_owned());
        item.updated_at = Utc::now();

        if item.attempts < 5 {
            item.status = ReconcileStatus::Pending;
        } else {
            item.status = ReconcileStatus::Failed;
        }

        Ok(())
    }

    pub async fn list_all(&self) -> Vec<ReconcileItem> {
        let queue = self.inner.lock().await;
        queue.iter().cloned().collect()
    }

    pub async fn history(&self, limit: usize) -> Vec<ReconcileItem> {
        let queue = self.inner.lock().await;
        let completed: Vec<ReconcileItem> = queue
            .iter()
            .filter(|i| {
                matches!(
                    i.status,
                    ReconcileStatus::Succeeded | ReconcileStatus::Failed
                )
            })
            .cloned()
            .collect();
        let len = completed.len();
        if len <= limit {
            completed
        } else {
            let start = len - limit;
            completed[start..].to_vec()
        }
    }
}

impl Default for ReconcileQueue {
    fn default() -> Self {
        Self::new()
    }
}
