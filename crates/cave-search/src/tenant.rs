//! Tenant identity primitives.
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/cluster/metadata/

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct TenantId(String);

impl TenantId {
    pub fn new(id: &str) -> Self {
        TenantId(id.to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone)]
pub struct Tenant {
    pub id: TenantId,
}
