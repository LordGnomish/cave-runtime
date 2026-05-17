// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! etcd Cluster API — member management.

use crate::error::{StoreError, StoreResult};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    pub id: u64,
    pub name: String,
    pub peer_ur_ls: Vec<String>,
    pub client_ur_ls: Vec<String>,
    pub is_learner: bool,
}

pub struct ClusterManager {
    members: RwLock<HashMap<u64, Member>>,
    cluster_id: u64,
}

impl Default for ClusterManager {
    fn default() -> Self {
        let mut members = HashMap::new();
        let self_id = 0xcafe_cafe_cafe_cafe_u64;
        members.insert(
            self_id,
            Member {
                id: self_id,
                name: "cave-store-0".to_string(),
                peer_ur_ls: vec!["http://127.0.0.1:2380".to_string()],
                client_ur_ls: vec!["http://127.0.0.1:2379".to_string()],
                is_learner: false,
            },
        );
        Self {
            members: RwLock::new(members),
            cluster_id: 0xdead_beef_dead_beef_u64,
        }
    }
}

impl ClusterManager {
    pub fn cluster_id(&self) -> u64 {
        self.cluster_id
    }

    pub async fn member_add(&self, peer_urls: Vec<String>, is_learner: bool) -> StoreResult<Member> {
        let id = rand_member_id();
        let member = Member {
            id,
            name: String::new(), // assigned by the new node on join
            peer_ur_ls: peer_urls,
            client_ur_ls: Vec::new(),
            is_learner,
        };
        self.members.write().await.insert(id, member.clone());
        Ok(member)
    }

    pub async fn member_remove(&self, id: u64) -> StoreResult<()> {
        self.members
            .write()
            .await
            .remove(&id)
            .ok_or(StoreError::MemberNotFound(id))?;
        Ok(())
    }

    pub async fn member_update(&self, id: u64, peer_urls: Vec<String>) -> StoreResult<Member> {
        let mut members = self.members.write().await;
        let m = members
            .get_mut(&id)
            .ok_or(StoreError::MemberNotFound(id))?;
        m.peer_ur_ls = peer_urls;
        Ok(m.clone())
    }

    pub async fn member_list(&self) -> Vec<Member> {
        self.members.read().await.values().cloned().collect()
    }

    pub async fn member_promote(&self, id: u64) -> StoreResult<Member> {
        let mut members = self.members.write().await;
        let m = members
            .get_mut(&id)
            .ok_or(StoreError::MemberNotFound(id))?;
        m.is_learner = false;
        Ok(m.clone())
    }
}

fn rand_member_id() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
}
