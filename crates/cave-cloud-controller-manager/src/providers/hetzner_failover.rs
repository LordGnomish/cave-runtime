//! Hetzner FloatingIP failover state machine.
//!
//! Mirrors the failover pathway upstream uses for HA control planes — a
//! `FloatingIP` is paired with a primary node and (optionally) a secondary,
//! and `failover()` promotes the secondary when the primary becomes
//! unreachable.

use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailoverState {
    /// `current == primary`. Steady state.
    Healthy,
    /// `current == secondary`. We've failed over but the primary may come
    /// back. The next reconcile call will either heal (back to primary) or
    /// declare the new current as primary if a `commit()` was issued.
    FailedOver,
    /// No assignment yet — `current` is `None`.
    Unbound,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloatingIpFailover {
    pub floating_ip_id: u64,
    pub primary_server_id: u64,
    pub secondary_server_id: Option<u64>,
    pub current_server_id: Option<u64>,
}

impl FloatingIpFailover {
    pub fn new(floating_ip_id: u64, primary: u64, secondary: Option<u64>) -> Self {
        Self {
            floating_ip_id,
            primary_server_id: primary,
            secondary_server_id: secondary,
            current_server_id: Some(primary),
        }
    }

    pub fn state(&self) -> FailoverState {
        match self.current_server_id {
            None => FailoverState::Unbound,
            Some(c) if c == self.primary_server_id => FailoverState::Healthy,
            Some(c) if Some(c) == self.secondary_server_id => FailoverState::FailedOver,
            // Not primary, not secondary — treat as Unbound until reset.
            Some(_) => FailoverState::Unbound,
        }
    }

    /// Promote the secondary to current. Mirrors the `swap` action upstream.
    pub fn failover(&mut self) -> Result<(), CloudError> {
        let secondary = self.secondary_server_id.ok_or(CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: "failover requires a configured secondary server".into(),
        })?;
        if self.current_server_id == Some(secondary) {
            // Already failed over — caller is being paranoid; idempotent.
            return Ok(());
        }
        self.current_server_id = Some(secondary);
        Ok(())
    }

    /// Move back to the original primary. Used when the operator confirms
    /// the original primary is healthy again.
    pub fn heal(&mut self) -> Result<(), CloudError> {
        self.current_server_id = Some(self.primary_server_id);
        Ok(())
    }

    /// Promote the *current* assignment to be the new primary, demoting the
    /// old primary into the secondary slot. Mirrors the upstream "commit"
    /// step that updates the floating IP record after a confirmed failover.
    pub fn commit(&mut self) -> Result<(), CloudError> {
        let new_primary = self.current_server_id.ok_or(CloudError::InvalidConfig {
            provider: ProviderName::Hetzner,
            reason: "cannot commit without an active assignment".into(),
        })?;
        if new_primary == self.primary_server_id {
            // Nothing to do — current is already primary.
            return Ok(());
        }
        let old_primary = self.primary_server_id;
        self.primary_server_id = new_primary;
        self.secondary_server_id = Some(old_primary);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::hetzner::PROVIDER_VERSION;
    use crate::test_ctx;
    use crate::types::TenantId;

    const REPO: &str = "hetznercloud/hcloud-cloud-controller-manager";

    fn ctx(tenant: &'static str, path: &'static str, sym: &'static str) -> TenantId {
        let (cite, t) = test_ctx!(ext: REPO, PROVIDER_VERSION, path, sym, tenant);
        assert_eq!(cite.repo, REPO);
        t
    }

    #[test]
    fn fresh_failover_starts_healthy() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "FloatingIP");
        let f = FloatingIpFailover::new(7, 100, Some(200));
        assert_eq!(f.state(), FailoverState::Healthy);
        assert_eq!(f.current_server_id, Some(100));
    }

    #[test]
    fn failover_without_secondary_is_an_error() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Assign");
        let mut f = FloatingIpFailover::new(7, 100, None);
        assert!(matches!(f.failover().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn failover_promotes_secondary_to_current() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Assign");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.failover().unwrap();
        assert_eq!(f.current_server_id, Some(200));
        assert_eq!(f.state(), FailoverState::FailedOver);
    }

    #[test]
    fn failover_is_idempotent_when_already_failed_over() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Assign");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.failover().unwrap();
        f.failover().unwrap();
        assert_eq!(f.current_server_id, Some(200));
    }

    #[test]
    fn heal_returns_to_primary() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Assign");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.failover().unwrap();
        f.heal().unwrap();
        assert_eq!(f.state(), FailoverState::Healthy);
    }

    #[test]
    fn commit_swaps_primary_and_secondary_after_failover() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Update");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.failover().unwrap();
        f.commit().unwrap();
        assert_eq!(f.primary_server_id, 200);
        assert_eq!(f.secondary_server_id, Some(100));
        assert_eq!(f.state(), FailoverState::Healthy);
    }

    #[test]
    fn commit_is_a_noop_when_current_is_primary() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Update");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.commit().unwrap();
        assert_eq!(f.primary_server_id, 100);
    }

    #[test]
    fn commit_without_current_assignment_is_rejected() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "Update");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.current_server_id = None;
        assert!(matches!(f.commit().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn unbound_state_has_no_current_server() {
        let _ = ctx("acme", "hcloud/floating_ip.go", "FloatingIP");
        let mut f = FloatingIpFailover::new(7, 100, Some(200));
        f.current_server_id = None;
        assert_eq!(f.state(), FailoverState::Unbound);
    }
}
