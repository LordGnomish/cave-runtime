// SPDX-License-Identifier: AGPL-3.0-or-later
//! `ip-restriction` plugin — CIDR allowlist/blocklist.

use crate::error::{AGwError, AGwResult};
use crate::plugins::{cfg_str_array, PluginContext};
use crate::proxy::GwResponse;
use serde_json::Value;
use std::net::IpAddr;

pub fn access(cfg: &Value, ctx: &mut PluginContext) -> AGwResult<Option<GwResponse>> {
    let allow = cfg_str_array(cfg, "allow");
    let deny = cfg_str_array(cfg, "deny");
    let ip_str = ctx.request.source_ip.clone()
        .or_else(|| ctx.request.headers.get("x-real-ip").cloned())
        .unwrap_or_default();
    if ip_str.is_empty() { return Err(AGwError::Forbidden("no source ip".into())); }
    let ip: IpAddr = ip_str.parse().map_err(|_| AGwError::Forbidden("bad ip".into()))?;
    if !allow.is_empty() && !allow.iter().any(|r| matches_cidr(r, &ip)) {
        return Err(AGwError::Forbidden(format!("ip {ip_str} not in allowlist")));
    }
    if !deny.is_empty() && deny.iter().any(|r| matches_cidr(r, &ip)) {
        return Err(AGwError::Forbidden(format!("ip {ip_str} blocked")));
    }
    Ok(None)
}

pub fn matches_cidr(rule: &str, ip: &IpAddr) -> bool {
    if !rule.contains('/') {
        return rule.parse::<IpAddr>().map(|r| &r == ip).unwrap_or(false);
    }
    let (net, prefix) = match rule.split_once('/') { Some(x) => x, None => return false };
    let prefix: u8 = match prefix.parse() { Ok(p) => p, Err(_) => return false };
    let net_ip: IpAddr = match net.parse() { Ok(i) => i, Err(_) => return false };
    match (net_ip, ip) {
        (IpAddr::V4(a), IpAddr::V4(b)) => {
            let mask = if prefix == 0 { 0u32 } else { u32::MAX << (32 - prefix) };
            (u32::from(a) & mask) == (u32::from(*b) & mask)
        }
        (IpAddr::V6(a), IpAddr::V6(b)) => {
            let (a_b, b_b) = (a.octets(), b.octets());
            let bytes = prefix / 8; let rem = prefix % 8;
            if a_b[..bytes as usize] != b_b[..bytes as usize] { return false; }
            if rem == 0 { return true; }
            let mask = 0xFFu8 << (8 - rem);
            (a_b[bytes as usize] & mask) == (b_b[bytes as usize] & mask)
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Route;
    use crate::proxy::GwRequest;
    fn pc(ip: &str) -> PluginContext {
        let mut r = GwRequest::new("GET", "/", "h"); r.source_ip = Some(ip.into());
        PluginContext::new(r, None, Route::new("r"))
    }
    #[test] fn cidr_v4() {
        assert!(matches_cidr("10.0.0.0/8", &"10.1.2.3".parse().unwrap()));
        assert!(!matches_cidr("10.0.0.0/8", &"11.0.0.1".parse().unwrap()));
    }
    #[test] fn exact() {
        assert!(matches_cidr("192.168.1.1", &"192.168.1.1".parse().unwrap()));
        assert!(!matches_cidr("192.168.1.1", &"192.168.1.2".parse().unwrap()));
    }
    #[test] fn allow_listed() {
        let mut c = pc("10.0.0.5");
        assert!(access(&serde_json::json!({ "allow": ["10.0.0.0/8"] }), &mut c).unwrap().is_none());
    }
    #[test] fn allow_unlisted() {
        let mut c = pc("8.8.8.8");
        assert!(access(&serde_json::json!({ "allow": ["10.0.0.0/8"] }), &mut c).is_err());
    }
    #[test] fn deny_listed() {
        let mut c = pc("203.0.113.5");
        assert!(access(&serde_json::json!({ "deny": ["203.0.113.0/24"] }), &mut c).is_err());
    }
    #[test] fn missing_ip() {
        let mut c = pc("");
        c.request.source_ip = None;
        assert!(access(&serde_json::json!({ "allow": ["10.0.0.0/8"] }), &mut c).is_err());
    }
    #[test] fn cidr_v6() {
        assert!(matches_cidr("2001:db8::/32", &"2001:db8:1::1".parse().unwrap()));
        assert!(!matches_cidr("2001:db8::/32", &"2001:dead::1".parse().unwrap()));
    }
}
