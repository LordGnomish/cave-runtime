// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Topic management — create, delete, alter, describe topics and their configs.

use crate::error::{StreamError, StreamResult};
use crate::models::{PartitionLog, TopicConfig, TopicInfo};
use crate::storage::StreamStorage;
use chrono::Utc;

/// High-level topic management facade backed by any [`StreamStorage`].
pub struct TopicManager<S: StreamStorage> {
    storage: S,
}

impl<S: StreamStorage> TopicManager<S> {
    pub fn new(storage: S) -> Self {
        Self { storage }
    }

    /// Create a new topic.
    ///
    /// `partition_count` must be ≥ 1.  `replication_factor` must be ≥ 1.
    pub fn create(
        &self,
        name: impl Into<String>,
        partition_count: u32,
        replication_factor: u16,
        config: Option<TopicConfig>,
    ) -> StreamResult<TopicInfo> {
        let name = name.into();
        if name.is_empty() {
            return Err(StreamError::Validation("Topic name must not be empty".into()));
        }
        if partition_count == 0 {
            return Err(StreamError::Validation(
                "partition_count must be ≥ 1".into(),
            ));
        }
        if replication_factor == 0 {
            return Err(StreamError::Validation(
                "replication_factor must be ≥ 1".into(),
            ));
        }
        let topic = TopicInfo {
            name: name.clone(),
            partitions: partition_count,
            replication_factor,
            config: config.unwrap_or_default(),
            created_at: Utc::now(),
        };
        self.storage.create_topic(topic.clone())?;
        Ok(topic)
    }

    /// Delete an existing topic and all its partition logs.
    pub fn delete(&self, name: &str) -> StreamResult<()> {
        self.storage.delete_topic(name)
    }

    /// Fetch full topic info including current config.
    pub fn describe(&self, name: &str) -> StreamResult<TopicInfo> {
        self.storage
            .get_topic(name)?
            .ok_or_else(|| StreamError::TopicNotFound(name.into()))
    }

    /// List all topics.
    pub fn list(&self) -> StreamResult<Vec<TopicInfo>> {
        self.storage.list_topics()
    }

    /// Alter a topic's config.  Only supplied fields are changed.
    pub fn alter_config(
        &self,
        name: &str,
        update: TopicConfigPatch,
    ) -> StreamResult<TopicInfo> {
        let mut topic = self
            .storage
            .get_topic(name)?
            .ok_or_else(|| StreamError::TopicNotFound(name.into()))?;

        if let Some(v) = update.retention_ms {
            topic.config.retention_ms = Some(v);
        }
        if let Some(v) = update.retention_bytes {
            topic.config.retention_bytes = Some(v);
        }
        if let Some(v) = update.max_message_bytes {
            topic.config.max_message_bytes = v;
        }
        if let Some(v) = update.cleanup_policy {
            topic.config.cleanup_policy = v;
        }
        if let Some(v) = update.min_insync_replicas {
            topic.config.min_insync_replicas = v;
        }

        self.storage.update_topic(topic.clone())?;
        Ok(topic)
    }

    /// Increase the partition count (partitions can only be added, not removed).
    pub fn add_partitions(&self, name: &str, new_total: u32) -> StreamResult<TopicInfo> {
        let mut topic = self
            .storage
            .get_topic(name)?
            .ok_or_else(|| StreamError::TopicNotFound(name.into()))?;

        if new_total <= topic.partitions {
            return Err(StreamError::Validation(format!(
                "new_total ({new_total}) must be greater than current ({cur})",
                cur = topic.partitions
            )));
        }

        for p in topic.partitions..new_total {
            let log = PartitionLog::new(topic.name.clone(), p);
            self.storage.replace_partition_log(name, p, log)?;
        }
        topic.partitions = new_total;
        self.storage.update_topic(topic.clone())?;
        Ok(topic)
    }

    /// Return per-partition high-watermarks.
    pub fn watermarks(&self, name: &str) -> StreamResult<Vec<(u32, i64)>> {
        let topic = self
            .storage
            .get_topic(name)?
            .ok_or_else(|| StreamError::TopicNotFound(name.into()))?;
        (0..topic.partitions)
            .map(|p| {
                let hwm = self.storage.high_watermark(name, p)?;
                Ok((p, hwm))
            })
            .collect()
    }
}

/// Partial update for topic config (all fields optional).
#[derive(Debug, Default, serde::Deserialize)]
pub struct TopicConfigPatch {
    pub retention_ms: Option<i64>,
    pub retention_bytes: Option<i64>,
    pub max_message_bytes: Option<usize>,
    pub cleanup_policy: Option<crate::models::CleanupPolicy>,
    pub min_insync_replicas: Option<u16>,
}
