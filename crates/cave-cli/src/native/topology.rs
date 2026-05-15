// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl topology` — module/dependency graph.
//!
//! Returns a snapshot of how Cave modules depend on each other,
//! optionally filtered to a tenant or focused on one module.
//! Output rendering (DOT, JSON, ASCII) is the caller's choice.

use anyhow::{bail, Result};
use clap::Args;

use super::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct TopologyArgs {
    #[arg(short = 't', long)]
    pub tenant: Option<String>,

    /// Focus on a single module — show its first-degree neighbours.
    #[arg(long)]
    pub focus: Option<String>,

    /// Maximum depth from focus.
    #[arg(long, default_value_t = 1)]
    pub depth: u8,

    /// Output shape: `dot` (Graphviz), `json`, `ascii`, `mermaid`.
    #[arg(long, default_value = "ascii")]
    pub shape: String,

    /// Include sidecar/observability modules in the graph.
    #[arg(long)]
    pub include_sidecars: bool,
}

const SHAPES: &[&str] = &["dot", "json", "ascii", "mermaid"];

pub fn prepare(args: &TopologyArgs) -> Result<PreparedRequest> {
    if !SHAPES.contains(&args.shape.as_str()) {
        bail!(
            "unknown topology shape `{}`; want one of {:?}",
            args.shape,
            SHAPES
        );
    }
    if args.depth == 0 {
        bail!("depth must be at least 1");
    }
    if args.depth > 10 {
        bail!("depth too large (max 10)");
    }
    let mut path = match args.tenant.as_deref() {
        Some(t) => format!("/api/native/tenants/{}/topology", t),
        None => "/api/native/topology".to_string(),
    };
    let mut params: Vec<String> = Vec::new();
    if let Some(f) = &args.focus {
        params.push(format!("focus={}", f));
        params.push(format!("depth={}", args.depth));
    }
    params.push(format!("shape={}", args.shape));
    if args.include_sidecars {
        params.push("sidecars=true".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok(PreparedRequest::new(HttpVerb::Get, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args() -> TopologyArgs {
        TopologyArgs {
            tenant: None,
            focus: None,
            depth: 1,
            shape: "ascii".into(),
            include_sidecars: false,
        }
    }

    #[test]
    fn default_path() {
        let r = prepare(&args()).unwrap();
        assert!(r.path.starts_with("/api/native/topology?"));
        assert!(r.path.contains("shape=ascii"));
    }

    #[test]
    fn with_tenant() {
        let mut a = args();
        a.tenant = Some("acme".into());
        let p = prepare(&a).unwrap().path;
        assert!(p.starts_with("/api/native/tenants/acme/topology"));
    }

    #[test]
    fn focus_adds_depth() {
        let mut a = args();
        a.focus = Some("cave-apiserver".into());
        a.depth = 2;
        let p = prepare(&a).unwrap().path;
        assert!(p.contains("focus=cave-apiserver"));
        assert!(p.contains("depth=2"));
    }

    #[test]
    fn focus_default_depth_one() {
        let mut a = args();
        a.focus = Some("cave-apiserver".into());
        assert!(prepare(&a).unwrap().path.contains("depth=1"));
    }

    #[test]
    fn shapes_round_trip() {
        for s in SHAPES {
            let mut a = args();
            a.shape = (*s).into();
            assert!(prepare(&a).is_ok(), "shape {} should be accepted", s);
        }
    }

    #[test]
    fn rejects_unknown_shape() {
        let mut a = args();
        a.shape = "ascii-art".into();
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn rejects_zero_depth() {
        let mut a = args();
        a.depth = 0;
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn rejects_too_deep() {
        let mut a = args();
        a.depth = 11;
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn max_depth_ok() {
        let mut a = args();
        a.depth = 10;
        assert!(prepare(&a).is_ok());
    }

    #[test]
    fn min_depth_ok() {
        let mut a = args();
        a.depth = 1;
        assert!(prepare(&a).is_ok());
    }

    #[test]
    fn include_sidecars() {
        let mut a = args();
        a.include_sidecars = true;
        assert!(prepare(&a).unwrap().path.contains("sidecars=true"));
    }

    #[test]
    fn no_sidecars_default() {
        let p = prepare(&args()).unwrap().path;
        assert!(!p.contains("sidecars="));
    }

    #[test]
    fn uses_get() {
        let r = prepare(&args()).unwrap();
        assert_eq!(r.verb, HttpVerb::Get);
    }

    #[test]
    fn no_body() {
        let r = prepare(&args()).unwrap();
        assert!(r.body.is_none());
    }

    #[test]
    fn dot_shape() {
        let mut a = args();
        a.shape = "dot".into();
        assert!(prepare(&a).unwrap().path.contains("shape=dot"));
    }

    #[test]
    fn json_shape() {
        let mut a = args();
        a.shape = "json".into();
        assert!(prepare(&a).unwrap().path.contains("shape=json"));
    }

    #[test]
    fn mermaid_shape() {
        let mut a = args();
        a.shape = "mermaid".into();
        assert!(prepare(&a).unwrap().path.contains("shape=mermaid"));
    }

    #[test]
    fn focus_omits_when_unset() {
        let p = prepare(&args()).unwrap().path;
        assert!(!p.contains("focus="));
    }

    #[test]
    fn depth_omits_when_no_focus() {
        // Without focus, depth makes no sense in the path.
        let p = prepare(&args()).unwrap().path;
        assert!(!p.contains("depth="));
    }

    #[test]
    fn focus_with_tenant_and_sidecars() {
        let mut a = args();
        a.tenant = Some("acme".into());
        a.focus = Some("cave-net".into());
        a.depth = 3;
        a.include_sidecars = true;
        a.shape = "dot".into();
        let p = prepare(&a).unwrap().path;
        assert!(p.starts_with("/api/native/tenants/acme/topology?"));
        assert!(p.contains("focus=cave-net"));
        assert!(p.contains("depth=3"));
        assert!(p.contains("shape=dot"));
        assert!(p.contains("sidecars=true"));
    }

    #[test]
    fn shape_always_in_path() {
        // Even default invocation carries a shape.
        let p = prepare(&args()).unwrap().path;
        assert!(p.contains("shape="));
    }
}
