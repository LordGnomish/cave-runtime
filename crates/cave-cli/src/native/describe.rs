// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl describe <resource> <name>`
//!
//! Native describe — returns the rich, human-readable view of a Cave
//! resource. Server returns the same shape as `get` plus events,
//! conditions, and recent reconcile diffs.

use anyhow::{bail, Result};
use clap::Args;

use super::{HttpVerb, PreparedRequest};
use crate::native::get::canonical_resource;

#[derive(Args, Debug, Clone)]
pub struct DescribeArgs {
    pub resource: String,
    pub name: String,
    #[arg(short = 't', long)]
    pub tenant: Option<String>,
    /// Include reconcile diffs in the output.
    #[arg(long)]
    pub show_diffs: bool,
}

pub fn prepare(args: &DescribeArgs) -> Result<PreparedRequest> {
    if args.name.is_empty() {
        bail!("resource name required");
    }
    let resource = canonical_resource(&args.resource)?;
    let mut path = match args.tenant.as_deref() {
        Some(t) => format!("/api/native/tenants/{}/{}/{}", t, resource, args.name),
        None => format!("/api/native/{}/{}", resource, args.name),
    };
    let mut params = vec!["describe=true".to_string()];
    if args.show_diffs {
        params.push("diffs=true".to_string());
    }
    path.push('?');
    path.push_str(&params.join("&"));
    Ok(PreparedRequest::new(HttpVerb::Get, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(resource: &str, name: &str) -> DescribeArgs {
        DescribeArgs {
            resource: resource.into(),
            name: name.into(),
            tenant: None,
            show_diffs: false,
        }
    }

    #[test]
    fn describe_pod() {
        let r = prepare(&args("pods", "nginx")).unwrap();
        assert_eq!(r.path, "/api/native/pods/nginx?describe=true");
    }

    #[test]
    fn describe_with_tenant() {
        let mut a = args("pods", "nginx");
        a.tenant = Some("acme".into());
        let r = prepare(&a).unwrap();
        assert_eq!(
            r.path,
            "/api/native/tenants/acme/pods/nginx?describe=true"
        );
    }

    #[test]
    fn describe_with_diffs() {
        let mut a = args("modules", "cave-apiserver");
        a.show_diffs = true;
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("diffs=true"));
    }

    #[test]
    fn describe_short_alias() {
        let r = prepare(&args("po", "nginx")).unwrap();
        assert!(r.path.starts_with("/api/native/pods/nginx"));
    }

    #[test]
    fn describe_module() {
        let r = prepare(&args("modules", "cave-apiserver")).unwrap();
        assert_eq!(
            r.path,
            "/api/native/modules/cave-apiserver?describe=true"
        );
    }

    #[test]
    fn describe_rejects_empty_name() {
        let r = prepare(&args("pods", ""));
        assert!(r.is_err());
    }

    #[test]
    fn describe_uses_get() {
        let r = prepare(&args("pods", "nginx")).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn describe_no_body() {
        let r = prepare(&args("pods", "nginx")).unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn describe_diff_and_tenant_combine() {
        let mut a = args("pods", "x");
        a.tenant = Some("acme".into());
        a.show_diffs = true;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("describe=true"));
        assert!(p.contains("diffs=true"));
        assert!(p.starts_with("/api/native/tenants/acme/"));
    }

    #[test]
    fn describe_tenant_in_resource_path_segment() {
        let mut a = args("flags", "billing.v2");
        a.tenant = Some("acme".into());
        let r = prepare(&a).unwrap();
        assert!(r.path.contains("/tenants/acme/flags/billing.v2"));
    }

    #[test]
    fn describe_default_includes_describe_param() {
        let r = prepare(&args("pods", "x")).unwrap();
        assert!(r.path.contains("describe=true"));
    }
}
