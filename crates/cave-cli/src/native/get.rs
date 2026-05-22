// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl get <resource> [name] [-t <tenant>] [--all-tenants]`
//!
//! Native get verb. Resources are *Cave* concepts:
//!   tenants / modules / deployments / pods / secrets / flags /
//!   incidents / playbooks / events.
//!
//! Compat shims (`cavectl kubectl get pods`) delegate here.

use anyhow::{Result, bail};
use clap::Args;

use super::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct GetArgs {
    pub resource: String,
    pub name: Option<String>,

    /// Tenant scope. Defaults to current shell tenant.
    #[arg(short = 't', long)]
    pub tenant: Option<String>,

    /// All tenants — only callable with platform-admin role.
    #[arg(long)]
    pub all_tenants: bool,

    /// Label selector.
    #[arg(short = 'l', long)]
    pub selector: Option<String>,

    /// Watch mode.
    #[arg(short, long)]
    pub watch: bool,
}

pub fn prepare(args: &GetArgs) -> Result<PreparedRequest> {
    let resource = canonical_resource(&args.resource)?;
    let mut path = match (&args.name, args.all_tenants) {
        (Some(n), false) => match args.tenant.as_deref() {
            Some(t) => format!("/api/native/tenants/{}/{}/{}", t, resource, n),
            None => format!("/api/native/{}/{}", resource, n),
        },
        (None, true) => format!("/api/native/all/{}", resource),
        (None, false) => match args.tenant.as_deref() {
            Some(t) => format!("/api/native/tenants/{}/{}", t, resource),
            None => format!("/api/native/{}", resource),
        },
        (Some(_), true) => bail!("name and --all-tenants are mutually exclusive"),
    };

    let mut params: Vec<String> = Vec::new();
    if let Some(sel) = &args.selector {
        params.push(format!("selector={}", urlencode(sel)));
    }
    if args.watch {
        params.push("watch=true".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok(PreparedRequest::new(HttpVerb::Get, path))
}

/// Map short names and aliases to canonical resource paths.
///
/// Cave-domain resources — `pod` here means *cave pod* (a runtime
/// container managed by cave-cri), not Kubernetes pod. The compat
/// shim adapts kubectl's view onto this.
pub fn canonical_resource(r: &str) -> Result<String> {
    let lower = r.to_lowercase();
    let s = match lower.as_str() {
        "tenant" | "tenants" | "tn" => "tenants",
        "module" | "modules" | "mod" => "modules",
        "deployment" | "deployments" | "deploy" | "deps" => "deployments",
        "pod" | "pods" | "po" => "pods",
        "secret" | "secrets" | "sec" => "secrets",
        "flag" | "flags" | "fl" => "flags",
        "incident" | "incidents" | "inc" => "incidents",
        "playbook" | "playbooks" | "pb" => "playbooks",
        "event" | "events" | "ev" => "events",
        other if other.is_empty() => bail!("resource cannot be empty"),
        other => other,
    };
    Ok(s.to_string())
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write;
                let _ = write!(out, "%{:02X}", b);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(resource: &str) -> GetArgs {
        GetArgs {
            resource: resource.into(),
            name: None,
            tenant: None,
            all_tenants: false,
            selector: None,
            watch: false,
        }
    }

    #[test]
    fn get_pods_collection() {
        let r = prepare(&args("pods")).unwrap();
        assert_eq!(r.path, "/api/native/pods");
    }

    #[test]
    fn get_pod_short_alias() {
        let r = prepare(&args("po")).unwrap();
        assert_eq!(r.path, "/api/native/pods");
    }

    #[test]
    fn get_with_tenant_scoped_path() {
        let mut a = args("pods");
        a.tenant = Some("acme".into());
        assert_eq!(prepare(&a).unwrap().path, "/api/native/tenants/acme/pods");
    }

    #[test]
    fn get_all_tenants_path() {
        let mut a = args("pods");
        a.all_tenants = true;
        assert_eq!(prepare(&a).unwrap().path, "/api/native/all/pods");
    }

    #[test]
    fn get_single_collection_default() {
        let mut a = args("pods");
        a.name = Some("nginx".into());
        assert_eq!(prepare(&a).unwrap().path, "/api/native/pods/nginx");
    }

    #[test]
    fn get_single_with_tenant() {
        let mut a = args("pods");
        a.name = Some("nginx".into());
        a.tenant = Some("acme".into());
        assert_eq!(
            prepare(&a).unwrap().path,
            "/api/native/tenants/acme/pods/nginx"
        );
    }

    #[test]
    fn name_and_all_tenants_rejected() {
        let mut a = args("pods");
        a.name = Some("x".into());
        a.all_tenants = true;
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn get_tenants_resource() {
        let r = prepare(&args("tenants")).unwrap();
        assert_eq!(r.path, "/api/native/tenants");
    }

    #[test]
    fn get_modules_resource() {
        let r = prepare(&args("modules")).unwrap();
        assert_eq!(r.path, "/api/native/modules");
    }

    #[test]
    fn get_module_short_alias() {
        let r = prepare(&args("mod")).unwrap();
        assert_eq!(r.path, "/api/native/modules");
    }

    #[test]
    fn get_deployments_resource() {
        let r = prepare(&args("deployments")).unwrap();
        assert_eq!(r.path, "/api/native/deployments");
    }

    #[test]
    fn get_secrets_resource() {
        let r = prepare(&args("secrets")).unwrap();
        assert_eq!(r.path, "/api/native/secrets");
    }

    #[test]
    fn get_flags_resource() {
        let r = prepare(&args("flags")).unwrap();
        assert_eq!(r.path, "/api/native/flags");
    }

    #[test]
    fn get_incidents_resource() {
        let r = prepare(&args("incidents")).unwrap();
        assert_eq!(r.path, "/api/native/incidents");
    }

    #[test]
    fn get_playbooks_resource() {
        let r = prepare(&args("playbooks")).unwrap();
        assert_eq!(r.path, "/api/native/playbooks");
    }

    #[test]
    fn get_events_resource() {
        let r = prepare(&args("events")).unwrap();
        assert_eq!(r.path, "/api/native/events");
    }

    #[test]
    fn get_with_selector() {
        let mut a = args("pods");
        a.selector = Some("app=foo".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("selector=app%3Dfoo"));
    }

    #[test]
    fn get_with_watch() {
        let mut a = args("pods");
        a.watch = true;
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("watch=true"));
    }

    #[test]
    fn get_with_selector_and_watch() {
        let mut a = args("pods");
        a.selector = Some("app=foo".into());
        a.watch = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("selector="));
        assert!(p.contains("watch=true"));
    }

    #[test]
    fn get_passthrough_unknown_resource() {
        // Custom resource? Pass through. Server decides.
        let r = prepare(&args("widgets")).unwrap();
        assert_eq!(r.path, "/api/native/widgets");
    }

    #[test]
    fn get_empty_resource_rejected() {
        let r = prepare(&args(""));
        assert!(r.is_err());
    }

    #[test]
    fn get_uses_get_verb() {
        let r = prepare(&args("pods")).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn get_no_body() {
        let r = prepare(&args("pods")).unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn canonical_uppercase_normalised() {
        assert_eq!(canonical_resource("PODS").unwrap(), "pods");
    }

    #[test]
    fn get_all_tenants_with_selector() {
        let mut a = args("pods");
        a.all_tenants = true;
        a.selector = Some("env=prod".into());
        let p = prepare(&a).unwrap().path;
        assert!(p.starts_with("/api/native/all/pods?"));
        assert!(p.contains("selector=env%3Dprod"));
    }

    #[test]
    fn get_tenant_alias() {
        let r = prepare(&args("tn")).unwrap();
        assert_eq!(r.path, "/api/native/tenants");
    }

    #[test]
    fn get_flag_singular_alias() {
        let r = prepare(&args("flag")).unwrap();
        assert_eq!(r.path, "/api/native/flags");
    }

    #[test]
    fn get_event_singular_alias() {
        let r = prepare(&args("event")).unwrap();
        assert_eq!(r.path, "/api/native/events");
    }

    #[test]
    fn get_deps_short_alias() {
        let r = prepare(&args("deps")).unwrap();
        assert_eq!(r.path, "/api/native/deployments");
    }
}
