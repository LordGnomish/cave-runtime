//! Hetzner Cloud resource catalog + tenant-scoped inventory.
//!
//! Extends [`super::hetzner::HetznerProvider`] with the resource shapes the
//! controllers actually touch beyond the four-trait core surface, all pinned
//! to `hetznercloud/hcloud-cloud-controller-manager` @
//! [`super::hetzner::PROVIDER_VERSION`].
//!
//! * [`ServerType`] — `cx21`/`cx31`/`cx41`/`cpx*`/`cax*` catalog with
//!   cores/memory/disk and CPU architecture.
//! * [`Location`] — Falkenstein/Nuremberg/Helsinki/Ashburn with country,
//!   city, datacenter, network zone.
//! * [`PrivateNetwork`] + [`Subnet`] — IPv4 CIDR allocator.
//! * [`LoadBalancerType`] — `lb11`/`lb21`/`lb31` with limits.
//! * [`FloatingIp`] — IPv4/IPv6 pool with assign/unassign.
//! * [`SshKey`] — registry + fingerprint.
//! * [`Image`] — system / snapshot / backup / app catalog.
//! * [`Volume`] — custom block storage attached to a server.
//!
//! Multi-tenancy is enforced by [`HetznerInventory`]: every mutator takes a
//! [`TenantId`] and refuses to act on a different tenant's resources, mirroring
//! the per-tenant scoping in the upstream cloud config.

use crate::types::{CloudError, ProviderName, TenantId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ─── Server types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CpuArch {
    X86_64,
    Arm64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ServerType {
    Cx21,
    Cx31,
    Cx41,
    Cpx21,
    Cpx31,
    Cpx41,
    Cax11,
    Cax21,
}

impl ServerType {
    pub const fn name(self) -> &'static str {
        match self {
            ServerType::Cx21 => "cx21",
            ServerType::Cx31 => "cx31",
            ServerType::Cx41 => "cx41",
            ServerType::Cpx21 => "cpx21",
            ServerType::Cpx31 => "cpx31",
            ServerType::Cpx41 => "cpx41",
            ServerType::Cax11 => "cax11",
            ServerType::Cax21 => "cax21",
        }
    }

    pub fn from_name(s: &str) -> Option<Self> {
        match s {
            "cx21" => Some(ServerType::Cx21),
            "cx31" => Some(ServerType::Cx31),
            "cx41" => Some(ServerType::Cx41),
            "cpx21" => Some(ServerType::Cpx21),
            "cpx31" => Some(ServerType::Cpx31),
            "cpx41" => Some(ServerType::Cpx41),
            "cax11" => Some(ServerType::Cax11),
            "cax21" => Some(ServerType::Cax21),
            _ => None,
        }
    }

    pub const fn cores(self) -> u32 {
        match self {
            ServerType::Cx21 | ServerType::Cax11 => 2,
            ServerType::Cx31 | ServerType::Cpx21 | ServerType::Cax21 => 3,
            ServerType::Cx41 | ServerType::Cpx31 => 4,
            ServerType::Cpx41 => 8,
        }
    }

    pub const fn memory_gb(self) -> u32 {
        match self {
            ServerType::Cx21 | ServerType::Cax11 => 4,
            ServerType::Cx31 | ServerType::Cpx21 => 4,
            ServerType::Cax21 => 8,
            ServerType::Cx41 => 16,
            ServerType::Cpx31 => 8,
            ServerType::Cpx41 => 16,
        }
    }

    pub const fn disk_gb(self) -> u32 {
        match self {
            ServerType::Cx21 | ServerType::Cax11 => 40,
            ServerType::Cx31 => 80,
            ServerType::Cpx21 => 80,
            ServerType::Cax21 => 80,
            ServerType::Cx41 | ServerType::Cpx31 => 160,
            ServerType::Cpx41 => 240,
        }
    }

    pub const fn architecture(self) -> CpuArch {
        match self {
            ServerType::Cax11 | ServerType::Cax21 => CpuArch::Arm64,
            _ => CpuArch::X86_64,
        }
    }
}

// ─── Locations ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Location {
    Fsn1,
    Nbg1,
    Hel1,
    Ash,
}

impl Location {
    pub const fn code(self) -> &'static str {
        match self {
            Location::Fsn1 => "fsn1",
            Location::Nbg1 => "nbg1",
            Location::Hel1 => "hel1",
            Location::Ash => "ash",
        }
    }

    pub const fn city(self) -> &'static str {
        match self {
            Location::Fsn1 => "Falkenstein",
            Location::Nbg1 => "Nuremberg",
            Location::Hel1 => "Helsinki",
            Location::Ash => "Ashburn",
        }
    }

    pub const fn country(self) -> &'static str {
        match self {
            Location::Fsn1 | Location::Nbg1 => "DE",
            Location::Hel1 => "FI",
            Location::Ash => "US",
        }
    }

    pub const fn datacenter(self) -> &'static str {
        match self {
            Location::Fsn1 => "fsn1-dc14",
            Location::Nbg1 => "nbg1-dc3",
            Location::Hel1 => "hel1-dc2",
            Location::Ash => "ash-dc1",
        }
    }

    pub const fn network_zone(self) -> &'static str {
        match self {
            Location::Fsn1 | Location::Nbg1 | Location::Hel1 => "eu-central",
            Location::Ash => "us-east",
        }
    }

    pub fn from_code(s: &str) -> Option<Self> {
        match s {
            "fsn1" => Some(Location::Fsn1),
            "nbg1" => Some(Location::Nbg1),
            "hel1" => Some(Location::Hel1),
            "ash" => Some(Location::Ash),
            _ => None,
        }
    }
}

// ─── Load-balancer types ─────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LoadBalancerType {
    Lb11,
    Lb21,
    Lb31,
}

impl LoadBalancerType {
    pub const fn name(self) -> &'static str {
        match self {
            LoadBalancerType::Lb11 => "lb11",
            LoadBalancerType::Lb21 => "lb21",
            LoadBalancerType::Lb31 => "lb31",
        }
    }

    pub const fn max_services(self) -> u32 {
        match self {
            LoadBalancerType::Lb11 => 5,
            LoadBalancerType::Lb21 => 20,
            LoadBalancerType::Lb31 => 50,
        }
    }

    pub const fn max_targets(self) -> u32 {
        match self {
            LoadBalancerType::Lb11 => 25,
            LoadBalancerType::Lb21 => 75,
            LoadBalancerType::Lb31 => 200,
        }
    }

    pub const fn throughput_gbps(self) -> u32 {
        match self {
            LoadBalancerType::Lb11 => 5,
            LoadBalancerType::Lb21 => 10,
            LoadBalancerType::Lb31 => 20,
        }
    }
}

// ─── Networks ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SubnetKind {
    Cloud,
    Server,
    Vswitch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subnet {
    pub network_zone: String,
    pub cidr: String,
    pub kind: SubnetKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrivateNetwork {
    pub id: u64,
    pub name: String,
    pub cidr: String,
    pub subnets: Vec<Subnet>,
}

impl PrivateNetwork {
    pub fn allocate_subnet(&mut self, sub: Subnet) -> Result<(), CloudError> {
        if !sub.cidr.contains('/') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("subnet cidr must be CIDR, got {:?}", sub.cidr),
            });
        }
        if self.subnets.iter().any(|s| s.cidr == sub.cidr) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("subnet cidr {} already allocated", sub.cidr),
            });
        }
        self.subnets.push(sub);
        Ok(())
    }
}

// ─── Floating IP ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum FloatingIpType {
    V4,
    V6,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FloatingIp {
    pub id: u64,
    pub kind: FloatingIpType,
    pub ip: String,
    pub server_id: Option<u64>,
    pub home_location: Location,
}

// ─── SSH keys ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshKey {
    pub id: u64,
    pub name: String,
    pub public_key: String,
    pub fingerprint: String,
}

impl SshKey {
    /// Mirrors the shape upstream uses — colon-separated MD5 of the public
    /// key. Real provider runs `ssh-keygen -lf`; the test fake matches the
    /// length-and-shape so the controllers can index keys by fingerprint.
    pub fn compute_fingerprint(public_key: &str) -> String {
        // Deterministic fake: 16 bytes of djb2 → MD5-shape colon string.
        let mut h: u64 = 5381;
        for b in public_key.as_bytes() {
            h = h.wrapping_mul(33).wrapping_add(*b as u64);
        }
        let mut bytes = [0u8; 16];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = ((h >> ((i % 8) * 8)) ^ ((i as u64).wrapping_mul(31))) as u8;
        }
        bytes
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(":")
    }
}

// ─── Images / snapshots / backups ────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageKind {
    System,
    Snapshot,
    Backup,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ImageStatus {
    Available,
    Creating,
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Image {
    pub id: u64,
    pub kind: ImageKind,
    pub name: String,
    pub size_gb: u32,
    pub status: ImageStatus,
    /// Server this image was created from (`Snapshot`/`Backup` only).
    pub created_from_server: Option<u64>,
}

// ─── Custom volumes ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    pub id: u64,
    pub name: String,
    pub size_gb: u32,
    pub location: Location,
    pub server_id: Option<u64>,
}

// ─── Server creation request ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCreate {
    pub name: String,
    pub server_type: ServerType,
    pub location: Location,
    pub image_id: u64,
    pub ssh_key_ids: Vec<u64>,
    pub network_id: Option<u64>,
    pub enable_backup: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerCreated {
    pub id: u64,
    pub name: String,
    pub server_type: ServerType,
    pub location: Location,
    pub image_id: u64,
    pub backups_enabled: bool,
}

// ─── Tenant-scoped inventory ─────────────────────────────────────────────────

#[derive(Debug)]
pub struct HetznerInventory {
    tenant: TenantId,
    networks: HashMap<u64, PrivateNetwork>,
    floating_ips: HashMap<u64, FloatingIp>,
    ssh_keys: HashMap<u64, SshKey>,
    images: HashMap<u64, Image>,
    volumes: HashMap<u64, Volume>,
    backups_by_server: HashMap<u64, Vec<u64>>,
    next_id: u64,
}

impl HetznerInventory {
    pub fn for_tenant(tenant: TenantId) -> Self {
        Self {
            tenant,
            networks: HashMap::new(),
            floating_ips: HashMap::new(),
            ssh_keys: HashMap::new(),
            images: HashMap::new(),
            volumes: HashMap::new(),
            backups_by_server: HashMap::new(),
            next_id: 1,
        }
    }

    pub fn tenant(&self) -> &TenantId {
        &self.tenant
    }

    fn check_tenant(&self, caller: &TenantId, kind: &'static str, name: &str) -> Result<(), CloudError> {
        if caller != &self.tenant {
            return Err(CloudError::TenantDenied {
                tenant: caller.clone(),
                kind,
                name: name.to_string(),
            });
        }
        Ok(())
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    // SSH keys

    pub fn register_ssh_key(
        &mut self,
        caller: &TenantId,
        name: &str,
        public_key: &str,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "SSHKey", name)?;
        if !public_key.starts_with("ssh-") {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: "public_key must start with ssh-".into(),
            });
        }
        if self.ssh_keys.values().any(|k| k.name == name) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("ssh key name {name} already registered"),
            });
        }
        let id = self.alloc_id();
        let fingerprint = SshKey::compute_fingerprint(public_key);
        self.ssh_keys.insert(
            id,
            SshKey { id, name: name.into(), public_key: public_key.into(), fingerprint },
        );
        Ok(id)
    }

    pub fn ssh_key(&self, id: u64) -> Option<&SshKey> {
        self.ssh_keys.get(&id)
    }

    // Images

    pub fn register_image(
        &mut self,
        caller: &TenantId,
        name: &str,
        kind: ImageKind,
        size_gb: u32,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "Image", name)?;
        let id = self.alloc_id();
        self.images.insert(
            id,
            Image {
                id,
                kind,
                name: name.into(),
                size_gb,
                status: ImageStatus::Available,
                created_from_server: None,
            },
        );
        Ok(id)
    }

    pub fn image(&self, id: u64) -> Option<&Image> {
        self.images.get(&id)
    }

    pub fn image_by_name(&self, name: &str) -> Option<&Image> {
        self.images.values().find(|i| i.name == name)
    }

    // Networks

    pub fn create_network(
        &mut self,
        caller: &TenantId,
        name: &str,
        cidr: &str,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "Network", name)?;
        if !cidr.contains('/') {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("network cidr must be CIDR, got {cidr:?}"),
            });
        }
        let id = self.alloc_id();
        self.networks.insert(
            id,
            PrivateNetwork { id, name: name.into(), cidr: cidr.into(), subnets: Vec::new() },
        );
        Ok(id)
    }

    pub fn add_subnet(
        &mut self,
        caller: &TenantId,
        network_id: u64,
        sub: Subnet,
    ) -> Result<(), CloudError> {
        let net = self.networks.get(&network_id).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: format!("network {network_id} not found"),
        })?;
        self.check_tenant(caller, "Subnet", &net.name)?;
        let net = self.networks.get_mut(&network_id).unwrap();
        net.allocate_subnet(sub)
    }

    pub fn network(&self, id: u64) -> Option<&PrivateNetwork> {
        self.networks.get(&id)
    }

    // Floating IPs

    pub fn allocate_floating_ip(
        &mut self,
        caller: &TenantId,
        kind: FloatingIpType,
        home: Location,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "FloatingIP", "")?;
        let id = self.alloc_id();
        let ip = match kind {
            FloatingIpType::V4 => format!("198.51.100.{}", id % 250 + 1),
            FloatingIpType::V6 => format!("2001:db8::{:x}", id),
        };
        self.floating_ips.insert(
            id,
            FloatingIp { id, kind, ip, server_id: None, home_location: home },
        );
        Ok(id)
    }

    pub fn assign_floating_ip(
        &mut self,
        caller: &TenantId,
        floating_ip_id: u64,
        server_id: u64,
    ) -> Result<(), CloudError> {
        let fip = self.floating_ips.get_mut(&floating_ip_id).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: format!("floating ip {floating_ip_id} not found"),
        })?;
        if caller != &self.tenant {
            return Err(CloudError::TenantDenied {
                tenant: caller.clone(),
                kind: "FloatingIP",
                name: fip.ip.clone(),
            });
        }
        if let Some(existing) = fip.server_id {
            if existing != server_id {
                return Err(CloudError::InvalidConfig {
                    provider: ProviderName::Hetzner,
                    reason: format!("floating ip {} already assigned to {}", fip.ip, existing),
                });
            }
        }
        fip.server_id = Some(server_id);
        Ok(())
    }

    pub fn unassign_floating_ip(
        &mut self,
        caller: &TenantId,
        floating_ip_id: u64,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "FloatingIP", "")?;
        let fip = self.floating_ips.get_mut(&floating_ip_id).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: format!("floating ip {floating_ip_id} not found"),
        })?;
        fip.server_id = None;
        Ok(())
    }

    pub fn floating_ip(&self, id: u64) -> Option<&FloatingIp> {
        self.floating_ips.get(&id)
    }

    // Volumes

    pub fn create_volume(
        &mut self,
        caller: &TenantId,
        name: &str,
        size_gb: u32,
        location: Location,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "Volume", name)?;
        if !(10..=10240).contains(&size_gb) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("volume size {size_gb} GiB outside [10..=10240]"),
            });
        }
        let id = self.alloc_id();
        self.volumes.insert(
            id,
            Volume { id, name: name.into(), size_gb, location, server_id: None },
        );
        Ok(id)
    }

    pub fn attach_volume(
        &mut self,
        caller: &TenantId,
        volume_id: u64,
        server_id: u64,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "Volume", "")?;
        let v = self.volumes.get_mut(&volume_id).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: format!("volume {volume_id} not found"),
        })?;
        if let Some(existing) = v.server_id {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("volume {} already attached to {}", v.name, existing),
            });
        }
        v.server_id = Some(server_id);
        Ok(())
    }

    pub fn volume(&self, id: u64) -> Option<&Volume> {
        self.volumes.get(&id)
    }

    // Server creation

    pub fn create_server(
        &mut self,
        caller: &TenantId,
        req: ServerCreate,
    ) -> Result<ServerCreated, CloudError> {
        self.check_tenant(caller, "Server", &req.name)?;
        if !self.images.contains_key(&req.image_id) {
            return Err(CloudError::Upstream {
                provider: ProviderName::Hetzner,
                reason: format!("image {} not found", req.image_id),
            });
        }
        for kid in &req.ssh_key_ids {
            if !self.ssh_keys.contains_key(kid) {
                return Err(CloudError::Upstream {
                    provider: ProviderName::Hetzner,
                    reason: format!("ssh key {kid} not found"),
                });
            }
        }
        if let Some(nid) = req.network_id {
            if !self.networks.contains_key(&nid) {
                return Err(CloudError::Upstream {
                    provider: ProviderName::Hetzner,
                    reason: format!("network {nid} not found"),
                });
            }
        }
        let id = self.alloc_id();
        Ok(ServerCreated {
            id,
            name: req.name,
            server_type: req.server_type,
            location: req.location,
            image_id: req.image_id,
            backups_enabled: req.enable_backup,
        })
    }

    // Snapshots & backups

    pub fn snapshot_server(
        &mut self,
        caller: &TenantId,
        server_id: u64,
        description: &str,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "Snapshot", description)?;
        let id = self.alloc_id();
        self.images.insert(
            id,
            Image {
                id,
                kind: ImageKind::Snapshot,
                name: description.into(),
                size_gb: 0,
                status: ImageStatus::Creating,
                created_from_server: Some(server_id),
            },
        );
        Ok(id)
    }

    pub fn finish_snapshot(&mut self, id: u64, size_gb: u32) -> Result<(), CloudError> {
        let img = self.images.get_mut(&id).ok_or_else(|| CloudError::Upstream {
            provider: ProviderName::Hetzner,
            reason: format!("image {id} not found"),
        })?;
        if img.kind != ImageKind::Snapshot {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("image {id} is not a snapshot"),
            });
        }
        img.size_gb = size_gb;
        img.status = ImageStatus::Available;
        Ok(())
    }

    pub fn enable_backups(
        &mut self,
        caller: &TenantId,
        server_id: u64,
    ) -> Result<(), CloudError> {
        self.check_tenant(caller, "Backup", "")?;
        self.backups_by_server.entry(server_id).or_default();
        Ok(())
    }

    pub fn create_backup(
        &mut self,
        caller: &TenantId,
        server_id: u64,
        size_gb: u32,
    ) -> Result<u64, CloudError> {
        self.check_tenant(caller, "Backup", "")?;
        if !self.backups_by_server.contains_key(&server_id) {
            return Err(CloudError::InvalidConfig {
                provider: ProviderName::Hetzner,
                reason: format!("backups not enabled on server {server_id}"),
            });
        }
        let id = self.alloc_id();
        self.images.insert(
            id,
            Image {
                id,
                kind: ImageKind::Backup,
                name: format!("backup-{server_id}-{id}"),
                size_gb,
                status: ImageStatus::Available,
                created_from_server: Some(server_id),
            },
        );
        self.backups_by_server.get_mut(&server_id).unwrap().push(id);
        Ok(id)
    }

    pub fn backups_for(&self, server_id: u64) -> &[u64] {
        self.backups_by_server.get(&server_id).map(|v| v.as_slice()).unwrap_or(&[])
    }
}

// Stable, read-only fields exposed to other modules + tests.
impl HetznerInventory {
    pub fn ssh_key_count(&self) -> usize {
        self.ssh_keys.len()
    }
    pub fn image_count(&self) -> usize {
        self.images.len()
    }
    pub fn floating_ip_count(&self) -> usize {
        self.floating_ips.len()
    }
    pub fn volume_count(&self) -> usize {
        self.volumes.len()
    }
    pub fn networks(&self) -> &HashMap<u64, PrivateNetwork> {
        &self.networks
    }
}

/// Sorted location codes — handy for tests asserting full enumeration
/// without depending on enum iteration order.
pub fn all_locations_sorted() -> Vec<&'static str> {
    let mut v = [Location::Fsn1, Location::Nbg1, Location::Hel1, Location::Ash]
        .iter()
        .map(|l| l.code())
        .collect::<Vec<_>>();
    v.sort();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::providers::hetzner::PROVIDER_VERSION;
    use crate::test_ctx;

    const REPO: &str = "hetznercloud/hcloud-cloud-controller-manager";

    fn tenant_ctx(tenant: &'static str, path: &'static str, symbol: &'static str) -> TenantId {
        let (cite, t) = test_ctx!(ext: REPO, PROVIDER_VERSION, path, symbol, tenant);
        assert_eq!(cite.repo, REPO);
        assert_eq!(cite.version, PROVIDER_VERSION);
        t
    }

    // ─── Server type tests ───────────────────────────────────────────────────

    #[test]
    fn cx_intel_line_round_trips_through_name() {
        let _ = tenant_ctx("acme", "hcloud/server_type.go", "ServerType", );
        for st in [ServerType::Cx21, ServerType::Cx31, ServerType::Cx41] {
            assert_eq!(ServerType::from_name(st.name()), Some(st));
            assert_eq!(st.architecture(), CpuArch::X86_64);
        }
    }

    #[test]
    fn cpx_amd_line_round_trips_through_name() {
        let _ = tenant_ctx("acme", "hcloud/server_type.go", "ServerType");
        for st in [ServerType::Cpx21, ServerType::Cpx31, ServerType::Cpx41] {
            assert_eq!(ServerType::from_name(st.name()), Some(st));
            assert_eq!(st.architecture(), CpuArch::X86_64);
        }
    }

    #[test]
    fn cax_arm_line_uses_arm64_architecture() {
        let _ = tenant_ctx("acme", "hcloud/server_type.go", "Architecture");
        assert_eq!(ServerType::Cax11.architecture(), CpuArch::Arm64);
        assert_eq!(ServerType::Cax21.architecture(), CpuArch::Arm64);
    }

    #[test]
    fn server_type_specs_are_monotonic_within_family() {
        let _ = tenant_ctx("acme", "hcloud/server_type.go", "Cores");
        assert!(ServerType::Cx21.cores() <= ServerType::Cx31.cores());
        assert!(ServerType::Cx31.cores() <= ServerType::Cx41.cores());
        assert!(ServerType::Cpx21.disk_gb() <= ServerType::Cpx31.disk_gb());
        assert!(ServerType::Cpx31.disk_gb() <= ServerType::Cpx41.disk_gb());
    }

    #[test]
    fn unknown_server_type_name_returns_none() {
        let _ = tenant_ctx("acme", "hcloud/server_type.go", "ParseServerType");
        assert!(ServerType::from_name("does-not-exist").is_none());
    }

    // ─── Location tests ──────────────────────────────────────────────────────

    #[test]
    fn falkenstein_location_resolves_country_and_dc() {
        let _ = tenant_ctx("acme", "hcloud/location.go", "Location");
        assert_eq!(Location::Fsn1.code(), "fsn1");
        assert_eq!(Location::Fsn1.city(), "Falkenstein");
        assert_eq!(Location::Fsn1.country(), "DE");
        assert_eq!(Location::Fsn1.datacenter(), "fsn1-dc14");
        assert_eq!(Location::Fsn1.network_zone(), "eu-central");
    }

    #[test]
    fn nuremberg_helsinki_belong_to_eu_central_zone() {
        let _ = tenant_ctx("acme", "hcloud/location.go", "NetworkZone");
        assert_eq!(Location::Nbg1.network_zone(), "eu-central");
        assert_eq!(Location::Hel1.network_zone(), "eu-central");
        assert_eq!(Location::Hel1.country(), "FI");
    }

    #[test]
    fn ashburn_is_a_us_east_zone() {
        let _ = tenant_ctx("acme", "hcloud/location.go", "Location");
        assert_eq!(Location::Ash.country(), "US");
        assert_eq!(Location::Ash.network_zone(), "us-east");
    }

    #[test]
    fn location_parse_round_trips_for_all_codes() {
        let _ = tenant_ctx("acme", "hcloud/location.go", "ParseLocation");
        for code in all_locations_sorted() {
            let parsed = Location::from_code(code).unwrap();
            assert_eq!(parsed.code(), code);
        }
    }

    // ─── LB-type tests ───────────────────────────────────────────────────────

    #[test]
    fn lb11_through_lb31_are_monotonic_in_capacity() {
        let _ = tenant_ctx("acme", "hcloud/load_balancer_type.go", "LoadBalancerType");
        assert!(
            LoadBalancerType::Lb11.max_services() < LoadBalancerType::Lb21.max_services()
        );
        assert!(
            LoadBalancerType::Lb21.max_services() < LoadBalancerType::Lb31.max_services()
        );
        assert!(
            LoadBalancerType::Lb11.throughput_gbps()
                < LoadBalancerType::Lb21.throughput_gbps()
        );
        assert_eq!(LoadBalancerType::Lb11.name(), "lb11");
    }

    #[test]
    fn lb_target_caps_match_upstream_table() {
        let _ = tenant_ctx("acme", "hcloud/load_balancer_type.go", "MaxTargets");
        assert_eq!(LoadBalancerType::Lb11.max_targets(), 25);
        assert_eq!(LoadBalancerType::Lb21.max_targets(), 75);
        assert_eq!(LoadBalancerType::Lb31.max_targets(), 200);
    }

    // ─── SSH-key tests ───────────────────────────────────────────────────────

    #[test]
    fn ssh_key_registration_assigns_id_and_fingerprint() {
        let tenant = tenant_ctx("acme", "hcloud/ssh_key.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv
            .register_ssh_key(&tenant, "deploy", "ssh-ed25519 AAAA deploy@cave")
            .unwrap();
        let key = inv.ssh_key(id).unwrap();
        assert_eq!(key.name, "deploy");
        // Fingerprint shape: 16 hex bytes joined by ':'.
        assert_eq!(key.fingerprint.split(':').count(), 16);
    }

    #[test]
    fn ssh_key_rejects_missing_ssh_prefix() {
        let tenant = tenant_ctx("acme", "hcloud/ssh_key.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let err = inv.register_ssh_key(&tenant, "deploy", "AAAA no-prefix").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn ssh_key_name_collision_is_refused() {
        let tenant = tenant_ctx("acme", "hcloud/ssh_key.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        inv.register_ssh_key(&tenant, "deploy", "ssh-ed25519 AAAA").unwrap();
        let err = inv
            .register_ssh_key(&tenant, "deploy", "ssh-ed25519 BBBB")
            .unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn ssh_key_cross_tenant_register_is_refused() {
        let tenant = tenant_ctx("acme", "hcloud/ssh_key.go", "Create");
        let attacker = TenantId::new("attacker");
        let mut inv = HetznerInventory::for_tenant(tenant);
        let err = inv
            .register_ssh_key(&attacker, "deploy", "ssh-ed25519 AAAA")
            .unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    // ─── Image / snapshot / backup tests ─────────────────────────────────────

    #[test]
    fn image_lookup_by_name_returns_expected_record() {
        let tenant = tenant_ctx("acme", "hcloud/image.go", "Image");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.register_image(&tenant, "ubuntu-24.04", ImageKind::System, 8).unwrap();
        assert_eq!(inv.image(id).unwrap().kind, ImageKind::System);
        assert_eq!(inv.image_by_name("ubuntu-24.04").unwrap().id, id);
    }

    #[test]
    fn snapshot_creation_starts_in_creating_status() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "CreateImage");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let snap = inv.snapshot_server(&tenant, 42, "before-upgrade").unwrap();
        let img = inv.image(snap).unwrap();
        assert_eq!(img.kind, ImageKind::Snapshot);
        assert_eq!(img.status, ImageStatus::Creating);
        assert_eq!(img.created_from_server, Some(42));
    }

    #[test]
    fn finish_snapshot_writes_size_and_flips_status() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "CreateImage");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let snap = inv.snapshot_server(&tenant, 42, "snap").unwrap();
        inv.finish_snapshot(snap, 12).unwrap();
        let img = inv.image(snap).unwrap();
        assert_eq!(img.size_gb, 12);
        assert_eq!(img.status, ImageStatus::Available);
    }

    #[test]
    fn backups_must_be_enabled_before_creating_one() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "EnableBackup");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let err = inv.create_backup(&tenant, 99, 8).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        inv.enable_backups(&tenant, 99).unwrap();
        let bid = inv.create_backup(&tenant, 99, 8).unwrap();
        assert_eq!(inv.image(bid).unwrap().kind, ImageKind::Backup);
        assert_eq!(inv.backups_for(99), &[bid]);
    }

    #[test]
    fn finish_snapshot_refuses_non_snapshot_image() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "CreateImage");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.register_image(&tenant, "ubuntu", ImageKind::System, 8).unwrap();
        let err = inv.finish_snapshot(id, 4).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    // ─── Network / subnet tests ──────────────────────────────────────────────

    #[test]
    fn network_create_requires_cidr_form() {
        let tenant = tenant_ctx("acme", "hcloud/network.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let err = inv.create_network(&tenant, "k8s", "10.0.0.0").unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        let id = inv.create_network(&tenant, "k8s", "10.0.0.0/16").unwrap();
        assert_eq!(inv.network(id).unwrap().cidr, "10.0.0.0/16");
    }

    #[test]
    fn subnet_allocation_rejects_duplicates() {
        let tenant = tenant_ctx("acme", "hcloud/network.go", "AddSubnet");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let nid = inv.create_network(&tenant, "k8s", "10.0.0.0/16").unwrap();
        let sub = Subnet {
            network_zone: "eu-central".into(),
            cidr: "10.0.0.0/24".into(),
            kind: SubnetKind::Cloud,
        };
        inv.add_subnet(&tenant, nid, sub.clone()).unwrap();
        let err = inv.add_subnet(&tenant, nid, sub).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn subnet_allocation_records_kind_and_zone() {
        let tenant = tenant_ctx("acme", "hcloud/network.go", "AddSubnet");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let nid = inv.create_network(&tenant, "k8s", "10.0.0.0/16").unwrap();
        inv.add_subnet(
            &tenant,
            nid,
            Subnet {
                network_zone: "eu-central".into(),
                cidr: "10.0.1.0/24".into(),
                kind: SubnetKind::Server,
            },
        )
        .unwrap();
        let net = inv.network(nid).unwrap();
        assert_eq!(net.subnets[0].kind, SubnetKind::Server);
        assert_eq!(net.subnets[0].network_zone, "eu-central");
    }

    #[test]
    fn cross_tenant_network_create_is_refused() {
        let tenant = tenant_ctx("acme", "hcloud/network.go", "Create");
        let attacker = TenantId::new("attacker");
        let mut inv = HetznerInventory::for_tenant(tenant);
        let err = inv.create_network(&attacker, "k8s", "10.0.0.0/16").unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    // ─── Floating IP tests ───────────────────────────────────────────────────

    #[test]
    fn floating_ipv4_allocates_a_198_51_100_address() {
        let tenant = tenant_ctx("acme", "hcloud/floating_ip.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.allocate_floating_ip(&tenant, FloatingIpType::V4, Location::Fsn1).unwrap();
        let fip = inv.floating_ip(id).unwrap();
        assert!(fip.ip.starts_with("198.51.100."));
        assert_eq!(fip.kind, FloatingIpType::V4);
        assert_eq!(fip.home_location, Location::Fsn1);
    }

    #[test]
    fn floating_ipv6_uses_documentation_prefix() {
        let tenant = tenant_ctx("acme", "hcloud/floating_ip.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.allocate_floating_ip(&tenant, FloatingIpType::V6, Location::Hel1).unwrap();
        assert!(inv.floating_ip(id).unwrap().ip.starts_with("2001:db8:"));
    }

    #[test]
    fn floating_ip_assign_then_unassign_clears_server() {
        let tenant = tenant_ctx("acme", "hcloud/floating_ip.go", "Assign");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.allocate_floating_ip(&tenant, FloatingIpType::V4, Location::Fsn1).unwrap();
        inv.assign_floating_ip(&tenant, id, 7).unwrap();
        assert_eq!(inv.floating_ip(id).unwrap().server_id, Some(7));
        inv.unassign_floating_ip(&tenant, id).unwrap();
        assert_eq!(inv.floating_ip(id).unwrap().server_id, None);
    }

    #[test]
    fn floating_ip_cannot_be_reassigned_while_held() {
        let tenant = tenant_ctx("acme", "hcloud/floating_ip.go", "Assign");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.allocate_floating_ip(&tenant, FloatingIpType::V4, Location::Fsn1).unwrap();
        inv.assign_floating_ip(&tenant, id, 7).unwrap();
        let err = inv.assign_floating_ip(&tenant, id, 8).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
    }

    #[test]
    fn floating_ip_cross_tenant_assign_is_refused() {
        let tenant = tenant_ctx("acme", "hcloud/floating_ip.go", "Assign");
        let attacker = TenantId::new("attacker");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let id = inv.allocate_floating_ip(&tenant, FloatingIpType::V4, Location::Fsn1).unwrap();
        let err = inv.assign_floating_ip(&attacker, id, 7).unwrap_err();
        assert!(matches!(err, CloudError::TenantDenied { .. }));
    }

    // ─── Volume tests ────────────────────────────────────────────────────────

    #[test]
    fn volume_size_outside_quota_is_rejected() {
        let tenant = tenant_ctx("acme", "hcloud/volume.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        assert!(matches!(
            inv.create_volume(&tenant, "small", 5, Location::Fsn1).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
        assert!(matches!(
            inv.create_volume(&tenant, "huge", 99_999, Location::Fsn1).unwrap_err(),
            CloudError::InvalidConfig { .. }
        ));
    }

    #[test]
    fn volume_attach_is_idempotent_only_for_same_server() {
        let tenant = tenant_ctx("acme", "hcloud/volume.go", "Attach");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let v = inv.create_volume(&tenant, "data", 100, Location::Fsn1).unwrap();
        inv.attach_volume(&tenant, v, 7).unwrap();
        let err = inv.attach_volume(&tenant, v, 8).unwrap_err();
        assert!(matches!(err, CloudError::InvalidConfig { .. }));
        assert_eq!(inv.volume(v).unwrap().server_id, Some(7));
    }

    // ─── Server creation tests ───────────────────────────────────────────────

    #[test]
    fn server_create_requires_known_image() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let err = inv
            .create_server(
                &tenant,
                ServerCreate {
                    name: "node-1".into(),
                    server_type: ServerType::Cpx21,
                    location: Location::Fsn1,
                    image_id: 999,
                    ssh_key_ids: vec![],
                    network_id: None,
                    enable_backup: false,
                },
            )
            .unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[test]
    fn server_create_validates_ssh_keys_and_network() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let img = inv.register_image(&tenant, "u24", ImageKind::System, 8).unwrap();
        let key = inv.register_ssh_key(&tenant, "deploy", "ssh-ed25519 AAAA").unwrap();
        let net = inv.create_network(&tenant, "k8s", "10.0.0.0/16").unwrap();
        let s = inv
            .create_server(
                &tenant,
                ServerCreate {
                    name: "node-1".into(),
                    server_type: ServerType::Cpx21,
                    location: Location::Fsn1,
                    image_id: img,
                    ssh_key_ids: vec![key],
                    network_id: Some(net),
                    enable_backup: true,
                },
            )
            .unwrap();
        assert_eq!(s.server_type, ServerType::Cpx21);
        assert!(s.backups_enabled);
        assert_eq!(s.location, Location::Fsn1);
    }

    #[test]
    fn server_create_with_unknown_ssh_key_is_refused() {
        let tenant = tenant_ctx("acme", "hcloud/server.go", "Create");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        let img = inv.register_image(&tenant, "u24", ImageKind::System, 8).unwrap();
        let err = inv
            .create_server(
                &tenant,
                ServerCreate {
                    name: "node-1".into(),
                    server_type: ServerType::Cpx21,
                    location: Location::Fsn1,
                    image_id: img,
                    ssh_key_ids: vec![123],
                    network_id: None,
                    enable_backup: false,
                },
            )
            .unwrap_err();
        assert!(matches!(err, CloudError::Upstream { .. }));
    }

    #[test]
    fn inventory_counts_track_inserts() {
        let tenant = tenant_ctx("acme", "hcloud/cloud.go", "Inventory");
        let mut inv = HetznerInventory::for_tenant(tenant.clone());
        assert_eq!(inv.ssh_key_count(), 0);
        inv.register_ssh_key(&tenant, "k", "ssh-ed25519 AAAA").unwrap();
        inv.register_image(&tenant, "u24", ImageKind::System, 8).unwrap();
        inv.allocate_floating_ip(&tenant, FloatingIpType::V4, Location::Fsn1).unwrap();
        inv.create_volume(&tenant, "d", 50, Location::Fsn1).unwrap();
        assert_eq!(inv.ssh_key_count(), 1);
        assert_eq!(inv.image_count(), 1);
        assert_eq!(inv.floating_ip_count(), 1);
        assert_eq!(inv.volume_count(), 1);
    }
}
