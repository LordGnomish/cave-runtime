//! CloudProvider trait — pluggable cloud-specific operations.
//!
//! Mirrors `staging/src/k8s.io/cloud-provider/cloud.go::Interface` upstream.
//! Each cloud (aws, gcp, azure, ...) supplies an implementation; controllers
//! depend only on the trait so the same binary can target any cloud.

use crate::models::{InstanceMetadata, NodeAddress};

/// A pluggable cloud provider.
pub trait CloudProvider: Send + Sync {
    /// Provider name ("aws", "gcp", "fake", ...).
    fn name(&self) -> &str;

    /// Look up an instance by node name. Returns the metadata kube needs to
    /// initialize the Node object.
    ///
    /// Maps to upstream `Instances.InstanceMetadata`.
    fn instance_metadata(&self, node_name: &str) -> Option<InstanceMetadata>;

    /// Look up a node's failure-domain zone.
    ///
    /// Maps to upstream `Zones.GetZoneByNodeName`.
    fn node_zone(&self, node_name: &str) -> Option<String>;
}

/// In-memory `CloudProvider` used by tests and developer setups. Backed by a
/// pre-seeded map of node_name → metadata.
pub struct FakeProvider {
    pub name: String,
    pub instances: std::collections::HashMap<String, InstanceMetadata>,
}

impl Default for FakeProvider {
    fn default() -> Self {
        Self::new("fake")
    }
}

impl FakeProvider {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            instances: std::collections::HashMap::new(),
        }
    }

    pub fn add_node(
        &mut self,
        node_name: &str,
        instance_type: &str,
        zone: &str,
        ip: &str,
    ) {
        self.instances.insert(
            node_name.to_string(),
            InstanceMetadata {
                provider_id: format!("{}://{}/{}", self.name, zone, node_name),
                instance_type: instance_type.to_string(),
                zone: zone.to_string(),
                region: zone[..zone.len().saturating_sub(1)].to_string(),
                addresses: vec![NodeAddress {
                    kind: "InternalIP".into(),
                    value: ip.to_string(),
                }],
            },
        );
    }
}

impl CloudProvider for FakeProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn instance_metadata(&self, node_name: &str) -> Option<InstanceMetadata> {
        self.instances.get(node_name).cloned()
    }

    fn node_zone(&self, node_name: &str) -> Option<String> {
        self.instances.get(node_name).map(|m| m.zone.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake() -> FakeProvider {
        let mut p = FakeProvider::new("fake");
        p.add_node("n1", "m5.large", "us-east-1a", "10.0.0.1");
        p
    }

    /// Port of `TestProviderInstanceMetadata` — provider returns the seeded
    /// InstanceMetadata for a known node and None for an unknown one.
    #[test]
    fn test_provider_instance_metadata() {
        let p = fake();
        let meta = p.instance_metadata("n1").expect("known node");
        assert_eq!(meta.instance_type, "m5.large");
        assert_eq!(meta.zone, "us-east-1a");
        assert_eq!(meta.addresses.len(), 1);
        assert_eq!(meta.addresses[0].value, "10.0.0.1");
        assert!(p.instance_metadata("missing").is_none());
    }

    /// Port of `TestProviderZoneLookup` — zone lookup yields the same value
    /// embedded in InstanceMetadata.
    #[test]
    fn test_provider_zone_lookup() {
        let p = fake();
        assert_eq!(p.node_zone("n1").as_deref(), Some("us-east-1a"));
        assert!(p.node_zone("missing").is_none());
    }
}
