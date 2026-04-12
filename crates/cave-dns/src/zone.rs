use crate::error::{DnsError, DnsResult};
use crate::types::*;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Clone, Debug)]
pub struct Zone {
    pub origin: String,
    pub soa: ResourceRecord,
    pub records: HashMap<String, Vec<ResourceRecord>>,
}

pub struct ZoneStore {
    zones: Arc<RwLock<HashMap<String, Zone>>>,
}

impl Default for ZoneStore {
    fn default() -> Self {
        Self::new()
    }
}

impl ZoneStore {
    pub fn new() -> Self {
        ZoneStore {
            zones: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn add_zone(&self, zone: Zone) -> DnsResult<()> {
        let mut zones = self.zones.write().unwrap();
        if zones.contains_key(&zone.origin) {
            return Err(DnsError::ZoneExists(zone.origin.clone()));
        }
        zones.insert(zone.origin.clone(), zone);
        Ok(())
    }

    pub fn remove_zone(&self, origin: &str) -> DnsResult<()> {
        let mut zones = self.zones.write().unwrap();
        if zones.remove(origin).is_none() {
            return Err(DnsError::ZoneNotFound(origin.to_string()));
        }
        Ok(())
    }

    pub fn get_zone(&self, origin: &str) -> Option<Zone> {
        let zones = self.zones.read().unwrap();
        zones.get(origin).cloned()
    }

    pub fn list_zones(&self) -> Vec<String> {
        let zones = self.zones.read().unwrap();
        zones.keys().cloned().collect()
    }

    pub fn add_record(&self, zone_origin: &str, record: ResourceRecord) -> DnsResult<()> {
        let mut zones = self.zones.write().unwrap();
        let zone = zones
            .get_mut(zone_origin)
            .ok_or_else(|| DnsError::ZoneNotFound(zone_origin.to_string()))?;
        zone.records
            .entry(record.name.clone())
            .or_default()
            .push(record);
        Ok(())
    }

    pub fn remove_record(&self, zone_origin: &str, name: &str, rtype: &RecordType) -> DnsResult<()> {
        let mut zones = self.zones.write().unwrap();
        let zone = zones
            .get_mut(zone_origin)
            .ok_or_else(|| DnsError::ZoneNotFound(zone_origin.to_string()))?;
        if let Some(records) = zone.records.get_mut(name) {
            let before = records.len();
            records.retain(|r| &r.rtype != rtype);
            if records.len() == before {
                return Err(DnsError::RecordNotFound(format!("{} {:?}", name, rtype)));
            }
            if records.is_empty() {
                zone.records.remove(name);
            }
        } else {
            return Err(DnsError::RecordNotFound(name.to_string()));
        }
        Ok(())
    }

    pub fn get_records(&self, zone_origin: &str, name: &str, rtype: &RecordType) -> Vec<ResourceRecord> {
        let zones = self.zones.read().unwrap();
        let zone = match zones.get(zone_origin) {
            Some(z) => z,
            None => return vec![],
        };
        zone.records
            .get(name)
            .map(|recs| recs.iter().filter(|r| &r.rtype == rtype).cloned().collect())
            .unwrap_or_default()
    }

    /// Find the most specific authoritative zone for a name.
    pub fn find_zone(&self, name: &str) -> Option<Zone> {
        let zones = self.zones.read().unwrap();
        // Normalise name: ensure trailing dot
        let name = if name.ends_with('.') {
            name.to_string()
        } else {
            format!("{}.", name)
        };

        // Find the longest matching zone origin
        let mut best: Option<&Zone> = None;
        for zone in zones.values() {
            if name == zone.origin || name.ends_with(&format!(".{}", zone.origin)) {
                match best {
                    None => best = Some(zone),
                    Some(b) if zone.origin.len() > b.origin.len() => best = Some(zone),
                    _ => {}
                }
            }
        }
        best.cloned()
    }

    pub fn lookup(&self, name: &str, rtype: &RecordType) -> Vec<ResourceRecord> {
        let zones = self.zones.read().unwrap();
        let name = if name.ends_with('.') {
            name.to_string()
        } else {
            format!("{}.", name)
        };
        for zone in zones.values() {
            if let Some(records) = zone.records.get(&name) {
                let filtered: Vec<ResourceRecord> =
                    records.iter().filter(|r| &r.rtype == rtype).cloned().collect();
                if !filtered.is_empty() {
                    return filtered;
                }
            }
        }
        vec![]
    }

    /// Parse a simplified BIND-format zone file.
    pub fn parse_zone_file(&self, content: &str, origin: &str) -> DnsResult<Zone> {
        let origin = if origin.ends_with('.') {
            origin.to_string()
        } else {
            format!("{}.", origin)
        };

        let mut current_origin = origin.clone();
        let mut default_ttl: u32 = 3600;
        let mut records: HashMap<String, Vec<ResourceRecord>> = HashMap::new();
        let mut soa_record: Option<ResourceRecord> = None;

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with(';') {
                continue;
            }

            if let Some(rest) = line.strip_prefix("$ORIGIN") {
                current_origin = rest.trim().to_string();
                if !current_origin.ends_with('.') {
                    current_origin.push('.');
                }
                continue;
            }

            if let Some(rest) = line.strip_prefix("$TTL") {
                let ttl_str = rest.trim();
                default_ttl = ttl_str
                    .parse()
                    .map_err(|_| DnsError::ParseError(format!("invalid TTL: {}", ttl_str)))?;
                continue;
            }

            // Parse RR line: [name] [ttl] IN type rdata...
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 4 {
                continue;
            }

            let mut idx = 0;

            // First field: owner name
            let owner_raw = parts[idx];
            idx += 1;

            // Optionally: TTL
            let ttl: u32 = if let Ok(n) = parts[idx].parse::<u32>() {
                idx += 1;
                n
            } else {
                default_ttl
            };

            // Optionally: class (IN)
            if idx < parts.len() && parts[idx].eq_ignore_ascii_case("IN") {
                idx += 1;
            }

            if idx >= parts.len() {
                continue;
            }
            let rtype_str = parts[idx];
            idx += 1;

            // Expand owner name
            let owner = if owner_raw == "@" {
                current_origin.clone()
            } else if owner_raw.ends_with('.') {
                owner_raw.to_string()
            } else {
                format!("{}.{}", owner_raw, current_origin)
            };

            let rtype = RecordType::from_str(rtype_str);
            let remaining: Vec<&str> = parts[idx..].to_vec();

            let rdata = parse_rdata(&rtype, &remaining, &current_origin)?;

            let rr = ResourceRecord {
                name: owner.clone(),
                rtype: rtype.clone(),
                class: CLASS_IN,
                ttl,
                rdata: rdata.clone(),
            };

            if matches!(rtype, RecordType::SOA) && soa_record.is_none() {
                soa_record = Some(rr.clone());
            }

            records.entry(owner).or_default().push(rr);
        }

        // Build a default SOA if none was parsed
        let soa = soa_record.unwrap_or_else(|| ResourceRecord {
            name: origin.clone(),
            rtype: RecordType::SOA,
            class: CLASS_IN,
            ttl: default_ttl,
            rdata: RData::SOA {
                mname: format!("ns1.{}", origin),
                rname: format!("admin.{}", origin),
                serial: 1,
                refresh: 3600,
                retry: 900,
                expire: 604800,
                minimum: 300,
            },
        });

        let zone = Zone {
            origin: origin.clone(),
            soa,
            records,
        };

        // Store the zone (ignore duplicate error if called repeatedly in tests)
        let _ = self.add_zone(zone.clone());

        Ok(zone)
    }
}

fn parse_rdata(rtype: &RecordType, parts: &[&str], origin: &str) -> DnsResult<RData> {
    match rtype {
        RecordType::A => {
            let ip: std::net::Ipv4Addr = parts
                .first()
                .ok_or_else(|| DnsError::ParseError("missing A address".to_string()))?
                .parse()
                .map_err(|_| DnsError::ParseError("invalid IPv4".to_string()))?;
            Ok(RData::A(ip))
        }
        RecordType::AAAA => {
            let ip: std::net::Ipv6Addr = parts
                .first()
                .ok_or_else(|| DnsError::ParseError("missing AAAA address".to_string()))?
                .parse()
                .map_err(|_| DnsError::ParseError("invalid IPv6".to_string()))?;
            Ok(RData::AAAA(ip))
        }
        RecordType::CNAME => {
            let target = expand_name(parts[0], origin);
            Ok(RData::CNAME(target))
        }
        RecordType::NS => {
            let ns = expand_name(parts[0], origin);
            Ok(RData::NS(ns))
        }
        RecordType::PTR => {
            let ptr = expand_name(parts[0], origin);
            Ok(RData::PTR(ptr))
        }
        RecordType::MX => {
            if parts.len() < 2 {
                return Err(DnsError::ParseError("MX needs priority + exchange".to_string()));
            }
            let priority: u16 = parts[0]
                .parse()
                .map_err(|_| DnsError::ParseError("invalid MX priority".to_string()))?;
            let exchange = expand_name(parts[1], origin);
            Ok(RData::MX { priority, exchange })
        }
        RecordType::TXT => {
            // Concatenate remaining parts, strip quotes
            let text = parts.join(" ").replace('"', "");
            Ok(RData::TXT(vec![text.into_bytes()]))
        }
        RecordType::SRV => {
            if parts.len() < 4 {
                return Err(DnsError::ParseError("SRV needs priority weight port target".to_string()));
            }
            let priority: u16 = parts[0].parse().map_err(|_| DnsError::ParseError("SRV priority".to_string()))?;
            let weight: u16 = parts[1].parse().map_err(|_| DnsError::ParseError("SRV weight".to_string()))?;
            let port: u16 = parts[2].parse().map_err(|_| DnsError::ParseError("SRV port".to_string()))?;
            let target = expand_name(parts[3], origin);
            Ok(RData::SRV { priority, weight, port, target })
        }
        RecordType::SOA => {
            // mname rname serial refresh retry expire minimum
            if parts.len() < 7 {
                return Err(DnsError::ParseError("SOA needs 7 fields".to_string()));
            }
            let mname = expand_name(parts[0], origin);
            let rname = expand_name(parts[1], origin);
            let serial: u32 = parts[2].parse().map_err(|_| DnsError::ParseError("SOA serial".to_string()))?;
            let refresh: u32 = parts[3].parse().map_err(|_| DnsError::ParseError("SOA refresh".to_string()))?;
            let retry: u32 = parts[4].parse().map_err(|_| DnsError::ParseError("SOA retry".to_string()))?;
            let expire: u32 = parts[5].parse().map_err(|_| DnsError::ParseError("SOA expire".to_string()))?;
            let minimum: u32 = parts[6].parse().map_err(|_| DnsError::ParseError("SOA minimum".to_string()))?;
            Ok(RData::SOA { mname, rname, serial, refresh, retry, expire, minimum })
        }
        _ => {
            Ok(RData::Raw(parts.join(" ").into_bytes()))
        }
    }
}

fn expand_name(name: &str, origin: &str) -> String {
    if name == "@" {
        return origin.to_string();
    }
    if name.ends_with('.') {
        return name.to_string();
    }
    format!("{}.{}", name, origin)
}
