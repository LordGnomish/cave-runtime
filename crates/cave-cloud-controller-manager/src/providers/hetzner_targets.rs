// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Hetzner Cloud — deeper provider surface.
//!
//! Upstream: `hetznercloud/hcloud-cloud-controller-manager` @
//! [`super::hetzner::PROVIDER_VERSION`]. Covers:
//!
//! * **LB targets** — `Server` / `LabelSelector` / `IpAddress` variants,
//!   plus per-target health.
//! * **Placement groups** — `spread` groups, max-server cap.
//! * **Primary IP** — standalone resource (separate from FloatingIP).
//! * **Delete protection** — server / volume / floating-ip protection
//!   flags.
//! * **RDNS** — reverse-DNS PTR records on public addresses.
//! * **Action** — long-running operation handle: id, status, progress.

use crate::providers::hetzner_resources::Location;
use crate::types::{CloudError, ProviderName};
use serde::{Deserialize, Serialize};

// ─── LB targets ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LbTargetType {
    Server,
    LabelSelector,
    IpAddress,
}

impl LbTargetType {
    pub const fn name(self) -> &'static str {
        match self {
            LbTargetType::Server => "server",
            LbTargetType::LabelSelector => "label_selector",
            LbTargetType::IpAddress => "ip",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LbTargetHealth {
    Healthy,
    Unhealthy,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LbTarget {
    pub kind: LbTargetType,
    /// Server ID for `Server`, selector string for `LabelSelector`, IPv4
    /// address for `IpAddress`.
    pub identifier: String,
    pub health: LbTargetHealth,
    /// Whether this target should be reached over the private network
    /// (Hetzner LB attribute `use_private_ip`).
    pub use_private_ip: bool,
}

impl LbTarget {
    pub fn server(server_id: u64) -> Self {
        Self {
            kind: LbTargetType::Server,
            identifier: server_id.to_string(),
            health: LbTargetHealth::Unknown,
            use_private_ip: false,
        }
    }
    pub fn label_selector(sel: impl Into<String>) -> Self {
        Self {
            kind: LbTargetType::LabelSelector,
            identifier: sel.into(),
            health: LbTargetHealth::Unknown,
            use_private_ip: false,
        }
    }
    pub fn ip(addr: impl Into<String>) -> Self {
        Self {
            kind: LbTargetType::IpAddress,
            identifier: addr.into(),
            health: LbTargetHealth::Unknown,
            use_private_ip: false,
        }
    }

    pub fn validate(&self) -> Result<(), CloudError> {
        if self.identifier.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "lb target identifier must not be empty".into(),
            });
        }
        if self.kind == LbTargetType::Server && self.identifier.parse::<u64>().is_err() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "lb target type=server requires a numeric server id, got {:?}",
                    self.identifier
                ),
            });
        }
        if self.kind == LbTargetType::IpAddress && !self.identifier.contains('.') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "lb target type=ip requires a v4 address, got {:?}",
                    self.identifier
                ),
            });
        }
        Ok(())
    }
}

/// Mirrors the LB target `set_health()` flow on a target list.
pub fn set_target_health(
    targets: &mut [LbTarget],
    matcher: &str,
    health: LbTargetHealth,
) -> u32 {
    let mut updated = 0u32;
    for t in targets.iter_mut() {
        if t.identifier == matcher {
            t.health = health;
            updated += 1;
        }
    }
    updated
}

// ─── Placement groups ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PlacementGroupType {
    Spread,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlacementGroup {
    pub id: u64,
    pub name: String,
    pub kind: PlacementGroupType,
    pub server_ids: Vec<u64>,
}

impl PlacementGroup {
    pub const SPREAD_MAX_SERVERS: usize = 10;

    pub fn new(id: u64, name: &str) -> Self {
        Self {
            id,
            name: name.into(),
            kind: PlacementGroupType::Spread,
            server_ids: Vec::new(),
        }
    }

    pub fn add_server(&mut self, server_id: u64) -> Result<(), CloudError> {
        if self.server_ids.contains(&server_id) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("server {server_id} already in placement group {}", self.name),
            });
        }
        if self.server_ids.len() >= Self::SPREAD_MAX_SERVERS {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "placement group {} has reached its {} server cap",
                    self.name,
                    Self::SPREAD_MAX_SERVERS
                ),
            });
        }
        self.server_ids.push(server_id);
        Ok(())
    }

    pub fn remove_server(&mut self, server_id: u64) {
        self.server_ids.retain(|s| *s != server_id);
    }
}

// ─── Primary IP ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PrimaryIpType {
    V4,
    V6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrimaryIp {
    pub id: u64,
    pub kind: PrimaryIpType,
    pub address: String,
    pub assignee_server_id: Option<u64>,
    pub home_location: Location,
    pub auto_delete: bool,
    pub delete_protected: bool,
}

impl PrimaryIp {
    pub fn assign(&mut self, server_id: u64) -> Result<(), CloudError> {
        if let Some(existing) = self.assignee_server_id {
            if existing != server_id {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!(
                        "primary ip {} already assigned to {}",
                        self.address, existing
                    ),
                });
            }
        }
        self.assignee_server_id = Some(server_id);
        Ok(())
    }

    pub fn unassign(&mut self) -> Result<(), CloudError> {
        if self.assignee_server_id.is_none() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("primary ip {} not currently assigned", self.address),
            });
        }
        self.assignee_server_id = None;
        Ok(())
    }
}

// ─── Delete protection ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct ResourceProtection {
    pub delete: bool,
    pub rebuild: bool,
}

impl ResourceProtection {
    pub fn ensure_delete_allowed(&self, kind: &'static str) -> Result<(), CloudError> {
        if self.delete {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("{kind} is delete-protected"),
            });
        }
        Ok(())
    }
}

// ─── RDNS ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReverseDns {
    pub ip: String,
    pub dns_ptr: String,
}

impl ReverseDns {
    pub fn validate(&self) -> Result<(), CloudError> {
        if self.ip.is_empty() {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "rdns ip must not be empty".into(),
            });
        }
        if !self.dns_ptr.contains('.') || self.dns_ptr.starts_with('.') || self.dns_ptr.ends_with('.') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("rdns ptr {:?} is not a valid hostname", self.dns_ptr),
            });
        }
        if self.dns_ptr.len() > 253 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "rdns ptr exceeds 253 characters".into(),
            });
        }
        Ok(())
    }
}

// ─── Action (long-running operation) ─────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HetznerActionStatus {
    Running,
    Success,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HetznerAction {
    pub id: u64,
    pub command: String,
    pub status: HetznerActionStatus,
    /// 0..=100. Reaches 100 when status flips to Success.
    pub progress: u8,
    pub error_code: Option<String>,
}

impl HetznerAction {
    pub fn started(id: u64, command: &str) -> Self {
        Self {
            id,
            command: command.into(),
            status: HetznerActionStatus::Running,
            progress: 0,
            error_code: None,
        }
    }

    pub fn advance(&mut self, progress: u8) -> Result<(), CloudError> {
        if progress < self.progress {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!(
                    "action {} cannot regress from {}% to {}%",
                    self.id, self.progress, progress
                ),
            });
        }
        if progress > 100 {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("action {} progress {} exceeds 100", self.id, progress),
            });
        }
        self.progress = progress;
        if progress == 100 && self.status == HetznerActionStatus::Running {
            self.status = HetznerActionStatus::Success;
        }
        Ok(())
    }

    pub fn fail(&mut self, code: impl Into<String>) {
        self.status = HetznerActionStatus::Error;
        self.error_code = Some(code.into());
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            HetznerActionStatus::Success | HetznerActionStatus::Error
        )
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

    // ─── LB targets ──────────────────────────────────────────────────────────

    #[test]
    fn lb_target_type_names_match_api_strings() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerTargetType");
        assert_eq!(LbTargetType::Server.name(), "server");
        assert_eq!(LbTargetType::LabelSelector.name(), "label_selector");
        assert_eq!(LbTargetType::IpAddress.name(), "ip");
    }

    #[test]
    fn lb_target_constructors_set_kind_and_identifier() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "LoadBalancerTarget");
        assert_eq!(LbTarget::server(7).identifier, "7");
        assert_eq!(LbTarget::label_selector("role=worker").identifier, "role=worker");
        assert_eq!(LbTarget::ip("203.0.113.5").identifier, "203.0.113.5");
    }

    #[test]
    fn lb_target_validate_requires_non_empty_identifier() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateTarget");
        let mut t = LbTarget::server(7);
        t.identifier.clear();
        assert!(matches!(t.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lb_target_server_kind_requires_numeric_id() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateTarget");
        let mut t = LbTarget::server(7);
        t.identifier = "not-a-number".into();
        assert!(matches!(t.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lb_target_ip_kind_requires_v4_dotted_form() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateTarget");
        let mut t = LbTarget::ip("203.0.113.5");
        t.identifier = "no-dots".into();
        assert!(matches!(t.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn lb_target_label_selector_validates() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "validateTarget");
        assert!(LbTarget::label_selector("env=prod").validate().is_ok());
    }

    #[test]
    fn set_target_health_updates_matching_targets() {
        let _ = ctx("acme", "hcloud/load_balancer.go", "TargetHealth");
        let mut targets = vec![LbTarget::server(7), LbTarget::server(8)];
        let n = set_target_health(&mut targets, "7", LbTargetHealth::Healthy);
        assert_eq!(n, 1);
        assert_eq!(targets[0].health, LbTargetHealth::Healthy);
        assert_eq!(targets[1].health, LbTargetHealth::Unknown);
    }

    // ─── Placement groups ────────────────────────────────────────────────────

    #[test]
    fn placement_group_caps_at_ten_servers() {
        let _ = ctx("acme", "hcloud/placement_group.go", "PlacementGroup");
        let mut pg = PlacementGroup::new(1, "k8s-control-plane");
        for i in 0..PlacementGroup::SPREAD_MAX_SERVERS as u64 {
            pg.add_server(i + 1).unwrap();
        }
        let err = pg.add_server(99).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn placement_group_rejects_duplicate_server() {
        let _ = ctx("acme", "hcloud/placement_group.go", "AddServer");
        let mut pg = PlacementGroup::new(1, "k8s");
        pg.add_server(7).unwrap();
        let err = pg.add_server(7).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn placement_group_remove_server_is_idempotent() {
        let _ = ctx("acme", "hcloud/placement_group.go", "RemoveServer");
        let mut pg = PlacementGroup::new(1, "k8s");
        pg.add_server(7).unwrap();
        pg.remove_server(7);
        pg.remove_server(7); // idempotent
        assert!(pg.server_ids.is_empty());
    }

    // ─── Primary IP ──────────────────────────────────────────────────────────

    fn primary_ip(id: u64, addr: &str) -> PrimaryIp {
        PrimaryIp {
            id,
            kind: PrimaryIpType::V4,
            address: addr.into(),
            assignee_server_id: None,
            home_location: Location::Fsn1,
            auto_delete: false,
            delete_protected: false,
        }
    }

    #[test]
    fn primary_ip_assignment_round_trips_through_unassign() {
        let _ = ctx("acme", "hcloud/primary_ip.go", "Assign");
        let mut p = primary_ip(1, "203.0.113.5");
        p.assign(7).unwrap();
        assert_eq!(p.assignee_server_id, Some(7));
        p.unassign().unwrap();
        assert!(p.assignee_server_id.is_none());
    }

    #[test]
    fn primary_ip_assign_to_second_server_is_refused() {
        let _ = ctx("acme", "hcloud/primary_ip.go", "Assign");
        let mut p = primary_ip(1, "203.0.113.5");
        p.assign(7).unwrap();
        let err = p.assign(8).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn primary_ip_assign_is_idempotent_for_same_server() {
        let _ = ctx("acme", "hcloud/primary_ip.go", "Assign");
        let mut p = primary_ip(1, "203.0.113.5");
        p.assign(7).unwrap();
        p.assign(7).unwrap();
        assert_eq!(p.assignee_server_id, Some(7));
    }

    #[test]
    fn primary_ip_unassign_without_assignment_is_an_error() {
        let _ = ctx("acme", "hcloud/primary_ip.go", "Unassign");
        let mut p = primary_ip(1, "203.0.113.5");
        let err = p.unassign().unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    // ─── Delete protection ───────────────────────────────────────────────────

    #[test]
    fn resource_protection_default_allows_delete() {
        let _ = ctx("acme", "hcloud/server.go", "Protection");
        let p = ResourceProtection::default();
        assert!(p.ensure_delete_allowed("Server").is_ok());
    }

    #[test]
    fn resource_protection_with_delete_set_rejects_delete() {
        let _ = ctx("acme", "hcloud/server.go", "Protection");
        let p = ResourceProtection { delete: true, rebuild: false };
        assert!(matches!(
            p.ensure_delete_allowed("Server").unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    // ─── RDNS ────────────────────────────────────────────────────────────────

    #[test]
    fn rdns_validate_accepts_well_formed_hostname() {
        let _ = ctx("acme", "hcloud/server.go", "ChangeDNSPtr");
        assert!(ReverseDns {
            ip: "203.0.113.5".into(),
            dns_ptr: "node-1.example.com".into(),
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn rdns_validate_rejects_leading_dot() {
        let _ = ctx("acme", "hcloud/server.go", "ChangeDNSPtr");
        let r = ReverseDns { ip: "203.0.113.5".into(), dns_ptr: ".example.com".into() };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn rdns_validate_rejects_trailing_dot() {
        let _ = ctx("acme", "hcloud/server.go", "ChangeDNSPtr");
        let r = ReverseDns { ip: "203.0.113.5".into(), dns_ptr: "example.com.".into() };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn rdns_validate_rejects_empty_ip() {
        let _ = ctx("acme", "hcloud/server.go", "ChangeDNSPtr");
        let r = ReverseDns { ip: "".into(), dns_ptr: "h.example.com".into() };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn rdns_validate_rejects_overlong_hostname() {
        let _ = ctx("acme", "hcloud/server.go", "ChangeDNSPtr");
        let long = "a.".repeat(150) + "example.com";
        let r = ReverseDns { ip: "203.0.113.5".into(), dns_ptr: long };
        assert!(matches!(r.validate().unwrap_err(), CloudError::InvalidConfig { .. }));
    }

    // ─── Action ──────────────────────────────────────────────────────────────

    #[test]
    fn action_started_starts_at_zero_progress() {
        let _ = ctx("acme", "hcloud/action.go", "Action");
        let a = HetznerAction::started(42, "create_server");
        assert_eq!(a.progress, 0);
        assert_eq!(a.status, HetznerActionStatus::Running);
        assert!(!a.is_terminal());
    }

    #[test]
    fn action_advance_promotes_to_success_at_100() {
        let _ = ctx("acme", "hcloud/action.go", "WaitForFunc");
        let mut a = HetznerAction::started(42, "create_server");
        a.advance(50).unwrap();
        assert_eq!(a.status, HetznerActionStatus::Running);
        a.advance(100).unwrap();
        assert_eq!(a.status, HetznerActionStatus::Success);
        assert!(a.is_terminal());
    }

    #[test]
    fn action_advance_rejects_regression() {
        let _ = ctx("acme", "hcloud/action.go", "WaitForFunc");
        let mut a = HetznerAction::started(42, "x");
        a.advance(50).unwrap();
        let err = a.advance(10).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn action_advance_rejects_progress_above_100() {
        let _ = ctx("acme", "hcloud/action.go", "WaitForFunc");
        let mut a = HetznerAction::started(42, "x");
        let err = a.advance(101).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn action_fail_records_error_code_and_terminates() {
        let _ = ctx("acme", "hcloud/action.go", "Error");
        let mut a = HetznerAction::started(42, "x");
        a.fail("rate_limit_exceeded");
        assert_eq!(a.status, HetznerActionStatus::Error);
        assert_eq!(a.error_code.as_deref(), Some("rate_limit_exceeded"));
        assert!(a.is_terminal());
    }
}
