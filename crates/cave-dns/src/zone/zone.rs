use std::collections::HashMap;

use hickory_proto::rr::{DNSClass, Name, RData, Record, RecordType};

use crate::config::ZoneType;

/// A DNS zone — the authoritative source of truth for a domain.
#[derive(Debug, Clone)]
pub struct Zone {
    pub origin: Name,
    pub soa: Record,
    pub zone_type: ZoneType,
    /// Records keyed by (owner name, record type).
    records: HashMap<(Name, RecordType), Vec<Record>>,
}

/// Result of a zone lookup.
#[derive(Debug, Clone)]
pub struct LookupResult {
    pub records: Vec<Record>,
    pub authoritative: bool,
    pub was_wildcard: bool,
}

impl Zone {
    pub fn new(origin: Name, soa: Record, zone_type: ZoneType) -> Self {
        let mut z = Self {
            origin,
            soa: soa.clone(),
            zone_type,
            records: HashMap::new(),
        };
        z.add_record(soa);
        z
    }

    /// Zone serial from SOA MINIMUM field.
    pub fn serial(&self) -> u32 {
        match self.soa.data() {
            Some(RData::SOA(soa)) => soa.serial(),
            _ => 0,
        }
    }

    /// Add or replace a resource record.
    pub fn add_record(&mut self, record: Record) {
        let key = (record.name().clone(), record.record_type());
        self.records.entry(key).or_default().push(record);
    }

    /// Remove all records matching name + type; optionally match rdata too.
    pub fn remove_record(
        &mut self,
        name: &Name,
        rtype: RecordType,
        rdata: Option<&RData>,
    ) {
        let key = (name.clone(), rtype);
        if let Some(rdata_filter) = rdata {
            if let Some(vec) = self.records.get_mut(&key) {
                vec.retain(|r| r.data() != Some(rdata_filter));
                if vec.is_empty() {
                    self.records.remove(&key);
                }
            }
        } else {
            self.records.remove(&key);
        }
    }

    /// Remove all records for a name (all types).
    pub fn remove_name(&mut self, name: &Name) {
        self.records.retain(|(n, _), _| n != name);
    }

    /// Exact lookup for (name, qtype).
    pub fn lookup(&self, name: &Name, qtype: RecordType) -> Vec<Record> {
        match qtype {
            RecordType::ANY => {
                // Collect all records for this name
                self.records
                    .iter()
                    .filter(|((n, _), _)| n == name)
                    .flat_map(|(_, v)| v.iter().cloned())
                    .collect()
            }
            _ => self
                .records
                .get(&(name.clone(), qtype))
                .cloned()
                .unwrap_or_default(),
        }
    }

    /// Lookup with wildcard synthesis (RFC 4592).
    pub fn lookup_with_wildcards(
        &self,
        name: &Name,
        qtype: RecordType,
    ) -> LookupResult {
        // 1. Exact match
        let exact = self.lookup(name, qtype);
        if !exact.is_empty() {
            return LookupResult {
                records: exact,
                authoritative: true,
                was_wildcard: false,
            };
        }

        // 2. CNAME chase at exact name
        let cnames = self.lookup(name, RecordType::CNAME);
        if !cnames.is_empty() {
            return LookupResult {
                records: cnames,
                authoritative: true,
                was_wildcard: false,
            };
        }

        // 3. Wildcard match — walk up from `*.name` to `*.origin`
        if let Some(wildcard_records) = self.wildcard_lookup(name, qtype) {
            let synthesized = wildcard_records
                .into_iter()
                .map(|mut r| {
                    r.set_name(name.clone());
                    r
                })
                .collect();
            return LookupResult {
                records: synthesized,
                authoritative: true,
                was_wildcard: true,
            };
        }

        LookupResult {
            records: vec![],
            authoritative: self.origin.zone_of(name),
            was_wildcard: false,
        }
    }

    fn wildcard_lookup(&self, name: &Name, qtype: RecordType) -> Option<Vec<Record>> {
        // Build wildcard names: *.label.example.com, *.example.com, …
        let labels = name.iter().collect::<Vec<_>>();
        for i in 1..labels.len() {
            let wildcard_str = format!(
                "*.{}",
                labels[i..]
                    .iter()
                    .map(|l| String::from_utf8_lossy(l))
                    .collect::<Vec<_>>()
                    .join(".")
            );
            if let Ok(wname) = wildcard_str.parse::<Name>() {
                let recs = self.lookup(&wname, qtype);
                if !recs.is_empty() {
                    return Some(recs);
                }
            }
        }
        None
    }

    /// All records in the zone (for AXFR).
    pub fn all_records(&self) -> Vec<Record> {
        self.records.values().flat_map(|v| v.iter().cloned()).collect()
    }

    /// All records in canonical RFC 5936 AXFR order: SOA, others, SOA again.
    pub fn axfr_records(&self) -> Vec<Record> {
        let mut out = vec![self.soa.clone()];
        for r in self.all_records() {
            if r.record_type() != RecordType::SOA {
                out.push(r);
            }
        }
        out.push(self.soa.clone());
        out
    }

    /// Return true if this zone is authoritative for the given name.
    pub fn is_authoritative_for(&self, name: &Name) -> bool {
        self.origin.zone_of(name)
    }

    /// Increment the SOA serial and update internal SOA record.
    pub fn bump_serial(&mut self) {
        if let Some(RData::SOA(soa)) = self.soa.data().cloned() {
            let new_serial = soa.serial().wrapping_add(1);
            let new_soa = hickory_proto::rr::rdata::SOA::new(
                soa.mname().clone(),
                soa.rname().clone(),
                new_serial,
                soa.refresh(),
                soa.retry(),
                soa.expire(),
                soa.minimum(),
            );
            self.soa.set_data(Some(RData::SOA(new_soa.clone())));
            // Update stored SOA
            let key = (self.origin.clone(), RecordType::SOA);
            if let Some(v) = self.records.get_mut(&key) {
                for r in v.iter_mut() {
                    r.set_data(Some(RData::SOA(new_soa.clone())));
                }
            }
        }
    }
}

impl Default for Zone {
    fn default() -> Self {
        let origin: Name = "example.com.".parse().unwrap();
        let soa_data = hickory_proto::rr::rdata::SOA::new(
            "ns1.example.com.".parse().unwrap(),
            "hostmaster.example.com.".parse().unwrap(),
            2024010100,
            3600,
            900,
            604800,
            300,
        );
        let mut soa = Record::new();
        soa.set_name(origin.clone());
        soa.set_ttl(300);
        soa.set_record_type(RecordType::SOA);
        soa.set_dns_class(DNSClass::IN);
        soa.set_data(Some(RData::SOA(soa_data)));
        Zone::new(origin, soa, ZoneType::Primary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hickory_proto::rr::rdata::A;
    use std::net::Ipv4Addr;

    fn make_zone() -> Zone {
        Zone::default()
    }

    fn a_record(name: &str, addr: Ipv4Addr) -> Record {
        let mut r = Record::new();
        r.set_name(name.parse().unwrap());
        r.set_ttl(300);
        r.set_record_type(RecordType::A);
        r.set_dns_class(DNSClass::IN);
        r.set_data(Some(RData::A(A(addr))));
        r
    }

    #[test]
    fn add_and_lookup_record() {
        let mut zone = make_zone();
        let rec = a_record("www.example.com.", Ipv4Addr::new(1, 2, 3, 4));
        zone.add_record(rec.clone());

        let found = zone.lookup(&"www.example.com.".parse().unwrap(), RecordType::A);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].name(), rec.name());
    }

    #[test]
    fn remove_record_by_name_and_type() {
        let mut zone = make_zone();
        zone.add_record(a_record("www.example.com.", Ipv4Addr::new(1, 2, 3, 4)));
        zone.remove_record(&"www.example.com.".parse().unwrap(), RecordType::A, None);
        let found = zone.lookup(&"www.example.com.".parse().unwrap(), RecordType::A);
        assert!(found.is_empty());
    }

    #[test]
    fn axfr_records_starts_and_ends_with_soa() {
        let zone = make_zone();
        let records = zone.axfr_records();
        assert!(records.first().map(|r| r.record_type() == RecordType::SOA).unwrap_or(false));
        assert!(records.last().map(|r| r.record_type() == RecordType::SOA).unwrap_or(false));
    }

    #[test]
    fn lookup_with_wildcard_synthesises() {
        let mut zone = make_zone();
        // Add a wildcard record
        zone.add_record(a_record("*.example.com.", Ipv4Addr::new(9, 9, 9, 9)));

        let result = zone.lookup_with_wildcards(
            &"anything.example.com.".parse().unwrap(),
            RecordType::A,
        );
        assert!(!result.records.is_empty());
        assert!(result.was_wildcard);
        // Synthesised record name should be the query name
        assert_eq!(
            result.records[0].name(),
            &"anything.example.com.".parse::<Name>().unwrap()
        );
    }
}
