use std::sync::Arc;

use dashmap::DashMap;
use hickory_proto::rr::{Name, RecordType};
use tokio::sync::RwLock;
use tracing::{info, warn};

use crate::{
    config::{ZoneConfig, ZoneType},
    error::{DnsError, DnsResult},
    zone::{LookupResult, Zone},
};

/// Central registry of all DNS zones.
///
/// Zones are keyed by their origin name. Lookups perform longest-suffix
/// matching so that sub-zones are preferred over parent zones.
#[derive(Default)]
pub struct ZoneManager {
    zones: DashMap<Name, Arc<RwLock<Zone>>>,
}

impl ZoneManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Load a zone from config, optionally reading a zone file.
    pub async fn load_zone(&self, config: &ZoneConfig) -> DnsResult<()> {
        let origin: Name = config
            .name
            .parse()
            .map_err(|e: hickory_proto::error::ProtoError| DnsError::Parse(e.to_string()))?;

        let zone = if let Some(path) = &config.file {
            super::file::load_zone_file(std::path::Path::new(path), &origin)?
        } else {
            // Create an empty primary zone
            Zone::new(
                origin.clone(),
                crate::zone::file::make_default_soa(&origin),
                config.zone_type.clone(),
            )
        };

        info!(zone = %origin, "loaded zone");
        self.zones
            .insert(origin, Arc::new(RwLock::new(zone)));
        Ok(())
    }

    /// Add or replace a zone.
    pub async fn add_zone(&self, zone: Zone) -> DnsResult<()> {
        let name = zone.origin.clone();
        info!(zone = %name, "adding zone");
        self.zones
            .insert(name, Arc::new(RwLock::new(zone)));
        Ok(())
    }

    /// Remove a zone by origin.
    pub async fn remove_zone(&self, name: &Name) -> DnsResult<()> {
        match self.zones.remove(name) {
            Some(_) => {
                info!(zone = %name, "removed zone");
                Ok(())
            }
            None => Err(DnsError::NotFound(name.to_string())),
        }
    }

    /// Find the most specific (longest suffix) zone for the given name.
    pub fn find_zone(&self, name: &Name) -> Option<Arc<RwLock<Zone>>> {
        let mut best: Option<(usize, Arc<RwLock<Zone>>)> = None;
        for entry in self.zones.iter() {
            let zone_origin = entry.key();
            if zone_origin.zone_of(name) {
                let len = zone_origin.iter().count();
                if best.as_ref().map(|(bl, _)| len > *bl).unwrap_or(true) {
                    best = Some((len, Arc::clone(entry.value())));
                }
            }
        }
        best.map(|(_, z)| z)
    }

    /// Perform a full lookup (including wildcard) across all zones.
    pub async fn lookup(&self, name: &Name, qtype: RecordType) -> Option<LookupResult> {
        let zone_arc = self.find_zone(name)?;
        let zone = zone_arc.read().await;
        let result = zone.lookup_with_wildcards(name, qtype);
        if result.authoritative || !result.records.is_empty() {
            Some(result)
        } else {
            None
        }
    }

    /// List all zone origins.
    pub fn zone_names(&self) -> Vec<Name> {
        self.zones.iter().map(|e| e.key().clone()).collect()
    }

    /// Number of loaded zones.
    pub fn len(&self) -> usize {
        self.zones.len()
    }

    pub fn is_empty(&self) -> bool {
        self.zones.is_empty()
    }

    /// Get a direct reference for zone management operations.
    pub fn get_zone(&self, name: &Name) -> Option<Arc<RwLock<Zone>>> {
        self.zones.get(name).map(|e| Arc::clone(e.value()))
    }
}
