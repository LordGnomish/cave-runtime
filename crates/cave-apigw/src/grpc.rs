// SPDX-License-Identifier: AGPL-3.0-or-later
//! gRPC routing + REST→gRPC transcoding.
//!
//! Maps HTTP path patterns onto fully-qualified gRPC method names per the
//! Google API HTTP annotation convention (`google.api.http`).
//! Reference: Envoy `grpc_json_transcoder`.

use crate::error::{AGwError, AGwResult};
use regex::Regex;

#[derive(Debug, Clone)]
pub struct GrpcMethod {
    pub package: String,
    pub service: String,
    pub method: String,
}

impl GrpcMethod {
    pub fn parse(path: &str) -> AGwResult<Self> {
        let trimmed = path.trim_start_matches('/');
        let mut parts = trimmed.split('/');
        let svc_full = parts.next().ok_or_else(|| AGwError::BadRequest("no svc".into()))?;
        let method = parts.next().ok_or_else(|| AGwError::BadRequest("no method".into()))?;
        let (package, service) = svc_full
            .rsplit_once('.')
            .ok_or_else(|| AGwError::BadRequest("svc not fully-qualified".into()))?;
        Ok(Self {
            package: package.to_string(),
            service: service.to_string(),
            method: method.to_string(),
        })
    }
    pub fn path(&self) -> String {
        format!("/{}.{}/{}", self.package, self.service, self.method)
    }
}

#[derive(Debug, Clone)]
pub struct TranscodingRule {
    pub http_method: String,
    pub http_path: String, // template e.g. /v1/users/{id}
    pub grpc: GrpcMethod,
    pub body_field: Option<String>,
}

pub struct Transcoder {
    rules: Vec<(Regex, TranscodingRule)>,
}

impl Default for Transcoder {
    fn default() -> Self {
        Self { rules: vec![] }
    }
}

impl Transcoder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn add(&mut self, rule: TranscodingRule) -> AGwResult<()> {
        let pat = path_template_to_regex(&rule.http_path);
        let re = Regex::new(&pat).map_err(|e| AGwError::BadRequest(format!("bad template: {e}")))?;
        self.rules.push((re, rule));
        Ok(())
    }

    /// Look up a transcoding rule for a given (method, path); return the gRPC target.
    pub fn lookup(&self, method: &str, path: &str) -> Option<(GrpcMethod, Vec<(String, String)>)> {
        for (re, rule) in &self.rules {
            if !rule.http_method.eq_ignore_ascii_case(method) {
                continue;
            }
            if let Some(caps) = re.captures(path) {
                let mut vars: Vec<(String, String)> = vec![];
                let names: Vec<&str> = re.capture_names().flatten().collect();
                for n in names {
                    if let Some(m) = caps.name(n) {
                        vars.push((n.to_string(), m.as_str().to_string()));
                    }
                }
                return Some((rule.grpc.clone(), vars));
            }
        }
        None
    }

    pub fn rules_count(&self) -> usize {
        self.rules.len()
    }
}

fn path_template_to_regex(template: &str) -> String {
    let mut out = String::with_capacity(template.len() + 16);
    out.push('^');
    let mut depth = 0usize;
    let mut name = String::new();
    for ch in template.chars() {
        match ch {
            '{' => { depth += 1; name.clear(); }
            '}' => {
                depth -= 1;
                out.push_str(&format!("(?P<{name}>[^/]+)"));
            }
            _ if depth > 0 => name.push(ch),
            '/' => out.push('/'),
            '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '\\' | '^' | '$' => {
                out.push('\\');
                out.push(ch);
            }
            other => out.push(other),
        }
    }
    out.push('$');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_method_basic() {
        let m = GrpcMethod::parse("/foo.bar.Svc/Method").unwrap();
        assert_eq!(m.package, "foo.bar");
        assert_eq!(m.service, "Svc");
        assert_eq!(m.method, "Method");
        assert_eq!(m.path(), "/foo.bar.Svc/Method");
    }

    #[test]
    fn parse_rejects_no_dot() {
        assert!(GrpcMethod::parse("/Svc/Method").is_err());
    }

    #[test]
    fn transcode_lookup_static() {
        let mut t = Transcoder::new();
        t.add(TranscodingRule {
            http_method: "GET".into(),
            http_path: "/v1/users/{id}".into(),
            grpc: GrpcMethod {
                package: "users.v1".into(),
                service: "UsersService".into(),
                method: "GetUser".into(),
            },
            body_field: None,
        })
        .unwrap();
        let (m, vars) = t.lookup("GET", "/v1/users/42").unwrap();
        assert_eq!(m.method, "GetUser");
        assert_eq!(vars[0], ("id".into(), "42".into()));
    }

    #[test]
    fn transcode_no_match_returns_none() {
        let t = Transcoder::new();
        assert!(t.lookup("GET", "/x").is_none());
    }

    #[test]
    fn transcode_multiple_path_vars() {
        let mut t = Transcoder::new();
        t.add(TranscodingRule {
            http_method: "GET".into(),
            http_path: "/v1/{ns}/users/{id}".into(),
            grpc: GrpcMethod {
                package: "u.v1".into(),
                service: "Svc".into(),
                method: "Get".into(),
            },
            body_field: None,
        })
        .unwrap();
        let (_, vars) = t.lookup("GET", "/v1/blue/users/9").unwrap();
        assert!(vars.iter().any(|(k, v)| k == "ns" && v == "blue"));
        assert!(vars.iter().any(|(k, v)| k == "id" && v == "9"));
    }

    #[test]
    fn transcode_method_filter() {
        let mut t = Transcoder::new();
        t.add(TranscodingRule {
            http_method: "POST".into(),
            http_path: "/v1/users".into(),
            grpc: GrpcMethod {
                package: "u.v1".into(),
                service: "Svc".into(),
                method: "Create".into(),
            },
            body_field: Some("user".into()),
        })
        .unwrap();
        assert!(t.lookup("GET", "/v1/users").is_none());
        assert!(t.lookup("POST", "/v1/users").is_some());
    }
}
