// SPDX-License-Identifier: AGPL-3.0-or-later
//! gRPC routing + REST→gRPC transcoding. Envoy `grpc_json_transcoder` reference.

use crate::error::{AGwError, AGwResult};
use regex::Regex;

#[derive(Debug, Clone)]
pub struct GrpcMethod { pub package: String, pub service: String, pub method: String }
impl GrpcMethod {
    pub fn parse(path: &str) -> AGwResult<Self> {
        let trimmed = path.trim_start_matches('/');
        let mut parts = trimmed.split('/');
        let svc_full = parts.next().ok_or_else(|| AGwError::BadRequest("no svc".into()))?;
        let method = parts.next().ok_or_else(|| AGwError::BadRequest("no method".into()))?;
        let (package, service) = svc_full.rsplit_once('.').ok_or_else(|| AGwError::BadRequest("not FQN".into()))?;
        Ok(Self { package: package.into(), service: service.into(), method: method.into() })
    }
    pub fn path(&self) -> String { format!("/{}.{}/{}", self.package, self.service, self.method) }
}

#[derive(Debug, Clone)]
pub struct TranscodingRule {
    pub http_method: String, pub http_path: String, pub grpc: GrpcMethod, pub body_field: Option<String>,
}

pub struct Transcoder { rules: Vec<(Regex, TranscodingRule)> }
impl Default for Transcoder { fn default() -> Self { Self { rules: vec![] } } }
impl Transcoder {
    pub fn new() -> Self { Self::default() }
    pub fn add(&mut self, rule: TranscodingRule) -> AGwResult<()> {
        let pat = path_template_to_regex(&rule.http_path);
        let re = Regex::new(&pat).map_err(|e| AGwError::BadRequest(format!("bad template: {e}")))?;
        self.rules.push((re, rule)); Ok(())
    }
    pub fn lookup(&self, method: &str, path: &str) -> Option<(GrpcMethod, Vec<(String, String)>)> {
        for (re, rule) in &self.rules {
            if !rule.http_method.eq_ignore_ascii_case(method) { continue; }
            if let Some(caps) = re.captures(path) {
                let mut vars = vec![];
                for n in re.capture_names().flatten() {
                    if let Some(m) = caps.name(n) { vars.push((n.into(), m.as_str().into())); }
                }
                return Some((rule.grpc.clone(), vars));
            }
        }
        None
    }
    pub fn rules_count(&self) -> usize { self.rules.len() }
}

fn path_template_to_regex(t: &str) -> String {
    let mut out = String::from("^"); let mut depth = 0usize; let mut name = String::new();
    for ch in t.chars() {
        match ch {
            '{' => { depth += 1; name.clear(); }
            '}' => { depth -= 1; out.push_str(&format!("(?P<{name}>[^/]+)")); }
            _ if depth > 0 => name.push(ch),
            '/' => out.push('/'),
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '\\' | '^' | '$' => { out.push('\\'); out.push(ch); }
            other => out.push(other),
        }
    }
    out.push('$'); out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test] fn parse_method() {
        let m = GrpcMethod::parse("/foo.bar.Svc/Method").unwrap();
        assert_eq!(m.package, "foo.bar"); assert_eq!(m.service, "Svc"); assert_eq!(m.method, "Method");
    }
    #[test] fn parse_reject_no_dot() { assert!(GrpcMethod::parse("/Svc/Method").is_err()); }
    #[test] fn transcode_lookup() {
        let mut t = Transcoder::new();
        t.add(TranscodingRule { http_method: "GET".into(), http_path: "/v1/users/{id}".into(),
            grpc: GrpcMethod { package: "users.v1".into(), service: "Svc".into(), method: "GetUser".into() },
            body_field: None }).unwrap();
        let (m, vars) = t.lookup("GET", "/v1/users/42").unwrap();
        assert_eq!(m.method, "GetUser"); assert_eq!(vars[0], ("id".into(), "42".into()));
    }
    #[test] fn transcode_none() { assert!(Transcoder::new().lookup("GET", "/x").is_none()); }
    #[test] fn transcode_multi_vars() {
        let mut t = Transcoder::new();
        t.add(TranscodingRule { http_method: "GET".into(), http_path: "/v1/{ns}/users/{id}".into(),
            grpc: GrpcMethod { package: "u.v1".into(), service: "Svc".into(), method: "Get".into() },
            body_field: None }).unwrap();
        let (_, v) = t.lookup("GET", "/v1/blue/users/9").unwrap();
        assert!(v.iter().any(|(k, x)| k == "ns" && x == "blue"));
        assert!(v.iter().any(|(k, x)| k == "id" && x == "9"));
    }
    #[test] fn transcode_method_filter() {
        let mut t = Transcoder::new();
        t.add(TranscodingRule { http_method: "POST".into(), http_path: "/v1/users".into(),
            grpc: GrpcMethod { package: "u.v1".into(), service: "Svc".into(), method: "Create".into() },
            body_field: Some("user".into()) }).unwrap();
        assert!(t.lookup("GET", "/v1/users").is_none());
        assert!(t.lookup("POST", "/v1/users").is_some());
    }
}
