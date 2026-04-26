//! SPIRE Agent store and attestation.

use crate::error::{SpireError, SpireResult};
use crate::models::*;
use chrono::Utc;
use dashmap::DashMap;
use tracing::info;
use uuid::Uuid;

pub struct AgentStore {
    agents: DashMap<String, SpireAgent>,
}

impl AgentStore {
    pub fn new() -> Self {
        Self { agents: DashMap::new() }
    }

    pub fn attest(&self, req: AttestAgentRequest) -> SpireResult<SpireAgent> {
        let agent_id = format!("spiffe://cluster.local/spire/agent/{}/{}", req.namespace, req.node_name);
        if let Some(existing) = self.agents.get(&agent_id) {
            if existing.status == AgentStatus::Banned {
                return Err(SpireError::AttestationFailed { detail: "agent is banned".into() });
            }
            let mut agent = existing.clone();
            agent.last_seen_at = Some(Utc::now());
            drop(existing);
            self.agents.insert(agent_id, agent.clone());
            return Ok(agent);
        }
        let path = req.spiffe_id_path.unwrap_or_else(|| format!("/spire/agent/{}/{}", req.namespace, req.node_name));
        let agent = SpireAgent {
            id: Uuid::new_v4(),
            agent_id: agent_id.clone(),
            spiffe_id: format!("spiffe://cluster.local{path}"),
            node_name: req.node_name.clone(),
            namespace: req.namespace.clone(),
            attestation_type: req.attestation_type,
            status: AgentStatus::Active,
            serial_number: Uuid::new_v4().to_string(),
            can_reattest: true,
            last_seen_at: Some(Utc::now()),
            created_at: Utc::now(),
        };
        self.agents.insert(agent_id, agent.clone());
        info!(node = %req.node_name, ns = %req.namespace, "SPIRE agent attested");
        Ok(agent)
    }

    pub fn get(&self, agent_id: &str) -> SpireResult<SpireAgent> {
        self.agents.get(agent_id).map(|r| r.clone()).ok_or_else(|| SpireError::AgentNotFound(agent_id.to_owned()))
    }

    pub fn list(&self, namespace: Option<&str>) -> Vec<SpireAgent> {
        self.agents.iter()
            .filter(|r| namespace.map_or(true, |ns| r.value().namespace == ns))
            .map(|r| r.value().clone())
            .collect()
    }

    pub fn ban(&self, agent_id: &str) -> SpireResult<SpireAgent> {
        let key = agent_id.to_owned();
        let mut agent = self.agents.get(&key).map(|r| r.clone()).ok_or_else(|| SpireError::AgentNotFound(key.clone()))?;
        agent.status = AgentStatus::Banned;
        self.agents.insert(key, agent.clone());
        Ok(agent)
    }

    pub fn delete(&self, agent_id: &str) -> SpireResult<()> {
        self.agents.remove(agent_id).ok_or_else(|| SpireError::AgentNotFound(agent_id.to_owned()))?;
        Ok(())
    }
}

impl Default for AgentStore {
    fn default() -> Self { Self::new() }
}
