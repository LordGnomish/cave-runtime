// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cloud service discovery — Hetzner + Azure subset.
//!
//! upstream: prometheus/prometheus — discovery/hetzner + discovery/azure
//!
//! Upstream iterates a list of cloud-provider SDK clients and turns each
//! described host into a Prometheus `Target` (address + labels). We keep
//! the same `Target` shape and the same relabel-meta keys upstream uses,
//! but parse the API responses straight from JSON so cave-metrics does
//! not have to pull every cloud SDK at link time.

use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Target {
    pub address: String,
    pub labels: HashMap<String, String>,
}

impl Target {
    pub fn new(address: impl Into<String>) -> Self {
        Self {
            address: address.into(),
            labels: HashMap::new(),
        }
    }

    pub fn with_label(mut self, k: impl Into<String>, v: impl Into<String>) -> Self {
        self.labels.insert(k.into(), v.into());
        self
    }
}

/// Convert a Hetzner Cloud `GET /v1/servers` JSON payload into targets.
///
/// We do a tiny JSON walker (no serde_json round-trip) so this module
/// stays dependency-free. Only the fields we actually care about are
/// extracted: id, name, status, public_net.ipv4.ip, labels{}, datacenter.
pub fn parse_hetzner_servers(json: &str, scrape_port: u16) -> Vec<Target> {
    let mut out = Vec::new();
    let mut i = 0;
    // Find each `"id":` then walk forward through its server block.
    let bytes = json.as_bytes();
    while let Some(pos) = find_subslice(bytes, i, b"\"id\":") {
        let mut server = Target::new(String::new());
        // id
        let (id, after_id) = read_number(bytes, pos + 5);
        i = after_id;
        if let Some(id_v) = id {
            server
                .labels
                .insert("__meta_hetzner_server_id".into(), id_v.to_string());
        }
        // name
        if let Some(n) = read_string_after_key(bytes, i, b"\"name\":") {
            server.labels.insert("__meta_hetzner_server_name".into(), n);
        }
        // status
        if let Some(s) = read_string_after_key(bytes, i, b"\"status\":") {
            server
                .labels
                .insert("__meta_hetzner_server_status".into(), s);
        }
        // ipv4 ip
        if let Some(ip) = read_string_after_key(bytes, i, b"\"ip\":") {
            server.address = format!("{}:{}", ip, scrape_port);
            server
                .labels
                .insert("__meta_hetzner_public_ipv4".into(), ip);
        }
        // datacenter name (search a window for the inner name)
        if let Some(dc) = read_string_after_key(bytes, i, b"\"datacenter\":") {
            server.labels.insert("__meta_hetzner_datacenter".into(), dc);
        }
        // Server is usable only if we managed to read an address.
        if !server.address.is_empty() {
            out.push(server);
        }
        // advance past this block — best-effort: find next `}`
        if let Some(end) = find_subslice(bytes, i, b"}") {
            i = end + 1;
        } else {
            break;
        }
    }
    out
}

/// Convert an Azure VM list response (subset: name, location, vmSize,
/// privateIp) into targets.
pub fn parse_azure_vms(json: &str, scrape_port: u16) -> Vec<Target> {
    let mut out = Vec::new();
    let bytes = json.as_bytes();
    let mut i = 0;
    while let Some(pos) = find_subslice(bytes, i, b"\"name\":") {
        let mut t = Target::new(String::new());
        if let Some(n) = read_string_after_key(bytes, pos, b"\"name\":") {
            t.labels.insert("__meta_azure_vm_name".into(), n);
        }
        if let Some(loc) = read_string_after_key(bytes, pos, b"\"location\":") {
            t.labels.insert("__meta_azure_region".into(), loc);
        }
        if let Some(sz) = read_string_after_key(bytes, pos, b"\"vmSize\":") {
            t.labels.insert("__meta_azure_vm_size".into(), sz);
        }
        if let Some(ip) = read_string_after_key(bytes, pos, b"\"privateIp\":") {
            t.address = format!("{}:{}", ip, scrape_port);
            t.labels.insert("__meta_azure_private_ipv4".into(), ip);
        }
        if !t.address.is_empty() {
            out.push(t);
        }
        if let Some(end) = find_subslice(bytes, pos, b"}") {
            i = end + 1;
        } else {
            break;
        }
    }
    out
}

// ─── tiny JSON helpers (substring-only, not a real parser) ─────────────────

fn find_subslice(hay: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    'outer: for i in from..=hay.len().saturating_sub(needle.len()) {
        for j in 0..needle.len() {
            if hay[i + j] != needle[j] {
                continue 'outer;
            }
        }
        return Some(i);
    }
    None
}

fn skip_whitespace(hay: &[u8], mut i: usize) -> usize {
    while i < hay.len()
        && (hay[i] == b' '
            || hay[i] == b'\t'
            || hay[i] == b'\n'
            || hay[i] == b'\r'
            || hay[i] == b':')
    {
        i += 1;
    }
    i
}

fn read_string_after_key(hay: &[u8], from: usize, key: &[u8]) -> Option<String> {
    let k = find_subslice(hay, from, key)?;
    let mut i = skip_whitespace(hay, k + key.len());
    if i >= hay.len() || hay[i] != b'"' {
        return None;
    }
    i += 1;
    let start = i;
    while i < hay.len() && hay[i] != b'"' {
        if hay[i] == b'\\' && i + 1 < hay.len() {
            i += 2;
        } else {
            i += 1;
        }
    }
    if i > hay.len() {
        return None;
    }
    Some(String::from_utf8_lossy(&hay[start..i]).to_string())
}

fn read_number(hay: &[u8], from: usize) -> (Option<u64>, usize) {
    let mut i = skip_whitespace(hay, from);
    let start = i;
    while i < hay.len() && hay[i].is_ascii_digit() {
        i += 1;
    }
    if i == start {
        return (None, from);
    }
    let s = std::str::from_utf8(&hay[start..i]).unwrap_or("");
    (s.parse().ok(), i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hetzner_extracts_address_and_port() {
        let body = r#"{"servers":[
          {"id":42,"name":"web-1","status":"running",
            "public_net":{"ipv4":{"ip":"203.0.113.4"}},
            "datacenter":"fsn1-dc14"}]}"#;
        let ts = parse_hetzner_servers(body, 9100);
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].address, "203.0.113.4:9100");
    }

    #[test]
    fn hetzner_records_meta_labels() {
        let body = r#"{"servers":[
          {"id":7,"name":"db-1","status":"running",
            "public_net":{"ipv4":{"ip":"203.0.113.5"}},
            "datacenter":"hel1-dc2"}]}"#;
        let ts = parse_hetzner_servers(body, 9100);
        assert_eq!(
            ts[0]
                .labels
                .get("__meta_hetzner_server_id")
                .map(String::as_str),
            Some("7")
        );
        assert_eq!(
            ts[0]
                .labels
                .get("__meta_hetzner_server_name")
                .map(String::as_str),
            Some("db-1")
        );
        assert_eq!(
            ts[0]
                .labels
                .get("__meta_hetzner_server_status")
                .map(String::as_str),
            Some("running")
        );
        assert_eq!(
            ts[0]
                .labels
                .get("__meta_hetzner_datacenter")
                .map(String::as_str),
            Some("hel1-dc2")
        );
    }

    #[test]
    fn hetzner_skips_servers_without_ipv4() {
        let body = r#"{"servers":[{"id":1,"name":"x"}]}"#;
        let ts = parse_hetzner_servers(body, 9100);
        assert!(ts.is_empty());
    }

    #[test]
    fn hetzner_handles_multiple_servers() {
        let body = r#"[
          {"id":1,"name":"a","public_net":{"ipv4":{"ip":"10.0.0.1"}},"datacenter":"a-dc"},
          {"id":2,"name":"b","public_net":{"ipv4":{"ip":"10.0.0.2"}},"datacenter":"b-dc"}
        ]"#;
        let ts = parse_hetzner_servers(body, 9090);
        assert_eq!(ts.len(), 2);
        assert_eq!(ts[0].address, "10.0.0.1:9090");
        assert_eq!(ts[1].address, "10.0.0.2:9090");
    }

    #[test]
    fn azure_extracts_private_ip_and_size() {
        let body = r#"{"value":[
          {"name":"vm-1","location":"westeurope","vmSize":"Standard_D2s_v5",
           "privateIp":"10.20.30.40"}]}"#;
        let ts = parse_azure_vms(body, 9100);
        assert_eq!(ts.len(), 1);
        assert_eq!(ts[0].address, "10.20.30.40:9100");
        assert_eq!(
            ts[0].labels.get("__meta_azure_region").map(String::as_str),
            Some("westeurope")
        );
        assert_eq!(
            ts[0].labels.get("__meta_azure_vm_size").map(String::as_str),
            Some("Standard_D2s_v5")
        );
    }

    #[test]
    fn azure_skips_entries_without_private_ip() {
        let body = r#"{"value":[{"name":"vm-no-ip","location":"westeurope","vmSize":"x"}]}"#;
        let ts = parse_azure_vms(body, 9100);
        assert!(ts.is_empty());
    }

    #[test]
    fn target_with_label_round_trips() {
        let t = Target::new("10.0.0.1:9100")
            .with_label("env", "prod")
            .with_label("region", "eu-1");
        assert_eq!(t.labels.get("env").map(String::as_str), Some("prod"));
        assert_eq!(t.labels.get("region").map(String::as_str), Some("eu-1"));
    }
}
