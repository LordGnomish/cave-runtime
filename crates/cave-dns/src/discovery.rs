use crate::types::*;
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug)]
pub struct ServiceEndpoint {
    pub name: String,
    pub namespace: String,
    pub cluster_domain: String,
    pub ip: Ipv4Addr,
    pub port: u16,
    pub protocol: String,
    pub ttl: u32,
}

impl ServiceEndpoint {
    /// Returns the FQDN: `name.namespace.svc.cluster_domain.`
    pub fn fqdn(&self) -> String {
        format!("{}.{}.svc.{}.", self.name, self.namespace, self.cluster_domain)
    }

    /// Returns an A record for this endpoint.
    pub fn a_record(&self) -> ResourceRecord {
        ResourceRecord {
            name: self.fqdn(),
            rtype: RecordType::A,
            class: CLASS_IN,
            ttl: self.ttl,
            rdata: RData::A(self.ip),
        }
    }

    /// Returns a SRV record for this endpoint.
    pub fn srv_record(&self) -> ResourceRecord {
        let proto = self.protocol.to_lowercase();
        let srv_name = format!(
            "_{}._{}.{}.{}.svc.{}.",
            self.name, proto, self.name, self.namespace, self.cluster_domain
        );
        ResourceRecord {
            name: srv_name,
            rtype: RecordType::SRV,
            class: CLASS_IN,
            ttl: self.ttl,
            rdata: RData::SRV {
                priority: 0,
                weight: 100,
                port: self.port,
                target: self.fqdn(),
            },
        }
    }
}

#[allow(dead_code)]
pub struct ServiceRegistry {
    services: Arc<RwLock<HashMap<String, ServiceEndpoint>>>,
    cluster_domain: String,
}

impl ServiceRegistry {
    pub fn new(cluster_domain: &str) -> Self {
        ServiceRegistry {
            services: Arc::new(RwLock::new(HashMap::new())),
            cluster_domain: cluster_domain.to_string(),
        }
    }

    pub fn register(&self, endpoint: ServiceEndpoint) {
        let fqdn = endpoint.fqdn();
        let mut services = self.services.write().unwrap();
        services.insert(fqdn, endpoint);
    }

    pub fn deregister(&self, fqdn: &str) {
        let mut services = self.services.write().unwrap();
        services.remove(fqdn);
    }

    pub fn lookup(&self, fqdn: &str) -> Option<ServiceEndpoint> {
        let services = self.services.read().unwrap();
        services.get(fqdn).cloned()
    }

    pub fn lookup_all_in_namespace(&self, namespace: &str) -> Vec<ServiceEndpoint> {
        let services = self.services.read().unwrap();
        services
            .values()
            .filter(|ep| ep.namespace == namespace)
            .cloned()
            .collect()
    }

    pub fn is_service_name(&self, name: &str) -> bool {
        name.contains(".svc.")
    }

    /// Resolve a service discovery query.
    pub fn resolve(&self, name: &str, rtype: &RecordType) -> Vec<ResourceRecord> {
        // Normalise with trailing dot
        let name = if name.ends_with('.') {
            name.to_string()
        } else {
            format!("{}.", name)
        };

        let services = self.services.read().unwrap();
        if let Some(ep) = services.get(&name) {
            return match rtype {
                RecordType::A => vec![ep.a_record()],
                RecordType::SRV => vec![ep.srv_record()],
                _ => vec![],
            };
        }
        vec![]
    }
}
