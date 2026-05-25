// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::cache::DnsCache;
use crate::types::*;
use crate::zone::{Zone, ZoneStore};
use std::sync::Arc;

pub struct Resolver {
    pub zones: Arc<ZoneStore>,
    pub cache: Arc<DnsCache>,
}

impl Resolver {
    pub fn new(zones: Arc<ZoneStore>, cache: Arc<DnsCache>) -> Self {
        Resolver { zones, cache }
    }

    /// Resolve a DNS query message and return a response message.
    pub fn resolve(&self, msg: &DnsMessage) -> DnsMessage {
        let mut response = DnsMessage {
            header: Header {
                id: msg.header.id,
                qr: true,
                opcode: msg.header.opcode,
                aa: false,
                tc: false,
                rd: msg.header.rd,
                ra: false,
                z: 0,
                rcode: RCODE_OK,
            },
            questions: msg.questions.clone(),
            answers: vec![],
            authority: vec![],
            additional: vec![],
        };

        let question = match msg.questions.first() {
            Some(q) => q,
            None => {
                response.header.rcode = RCODE_FORMAT;
                return response;
            }
        };

        // Check cache first
        let cache_key_type = question.qtype.to_u16();
        if let Some(cached) = self.cache.get(&question.name, cache_key_type) {
            response.header.aa = false;
            response.answers = cached;
            return response;
        }

        // Find authoritative zone
        match self.zones.find_zone(&question.name) {
            Some(zone) => {
                let answers = self.resolve_from_zone(question, &zone);
                if answers.is_empty() {
                    // Check if the name itself exists (for NXDOMAIN vs NODATA)
                    let name_exists = zone.records.contains_key(&question.name)
                        || zone.records.contains_key(
                            &if question.name.ends_with('.') {
                                question.name.clone()
                            } else {
                                format!("{}.", question.name)
                            },
                        );
                    if name_exists {
                        // NODATA — name exists but no records of this type
                        response.header.aa = true;
                        if let Some(soa) = self.soa_for_zone(&zone) {
                            response.authority.push(soa);
                        }
                    } else {
                        return self.nxdomain_response(msg);
                    }
                } else {
                    response.header.aa = true;
                    response.answers = answers.clone();
                    // Cache the result
                    let ttl = answers.first().map(|r| r.ttl).unwrap_or(300);
                    self.cache.insert(&question.name, cache_key_type, answers, ttl);
                }
            }
            None => {
                // Not authoritative — return REFUSED (no recursion)
                response.header.rcode = RCODE_REFUSED;
            }
        }

        response
    }

    fn resolve_from_zone(&self, question: &Question, zone: &Zone) -> Vec<ResourceRecord> {
        let name = if question.name.ends_with('.') {
            question.name.clone()
        } else {
            format!("{}.", question.name)
        };

        // Look for the requested type
        if let Some(records) = zone.records.get(&name) {
            let typed: Vec<ResourceRecord> = records
                .iter()
                .filter(|r| r.rtype == question.qtype)
                .cloned()
                .collect();
            if !typed.is_empty() {
                return typed;
            }

            // Check for CNAME
            let cnames: Vec<ResourceRecord> = records
                .iter()
                .filter(|r| r.rtype == RecordType::CNAME)
                .cloned()
                .collect();
            if !cnames.is_empty() {
                let mut result = cnames.clone();
                if let RData::CNAME(ref target) = cnames[0].rdata {
                    let followed = self.follow_cname(target, &question.qtype);
                    result.extend(followed);
                }
                return result;
            }
        }

        vec![]
    }

    fn follow_cname(&self, cname: &str, rtype: &RecordType) -> Vec<ResourceRecord> {
        let name = if cname.ends_with('.') {
            cname.to_string()
        } else {
            format!("{}.", cname)
        };

        // Look in zone store
        let records = self.zones.lookup(&name, rtype);
        if !records.is_empty() {
            return records;
        }

        // Try following another CNAME (limit depth to 5)
        let cname_records = self.zones.lookup(&name, &RecordType::CNAME);
        if let Some(rr) = cname_records.first() {
            if let RData::CNAME(ref next) = rr.rdata {
                let mut result = vec![rr.clone()];
                result.extend(self.follow_cname(next, rtype));
                return result;
            }
        }

        vec![]
    }

    fn nxdomain_response(&self, query: &DnsMessage) -> DnsMessage {
        let mut response = DnsMessage {
            header: Header {
                id: query.header.id,
                qr: true,
                opcode: query.header.opcode,
                aa: true,
                tc: false,
                rd: query.header.rd,
                ra: false,
                z: 0,
                rcode: RCODE_NXDOMAIN,
            },
            questions: query.questions.clone(),
            answers: vec![],
            authority: vec![],
            additional: vec![],
        };

        // Add SOA to authority for negative caching
        if let Some(question) = query.questions.first() {
            if let Some(zone) = self.zones.find_zone(&question.name) {
                if let Some(soa) = self.soa_for_zone(&zone) {
                    response.authority.push(soa);
                }
            }
        }

        response
    }

    fn soa_for_zone(&self, zone: &Zone) -> Option<ResourceRecord> {
        Some(zone.soa.clone())
    }
}
