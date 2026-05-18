// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl deploy <module> [--revision <r>] [--strategy <s>]`
//!
//! Native deploy verb — upstream-agnostic. Targets a *Cave module*
//! (a crate name like `cave-apiserver`) rather than a Kubernetes
//! `Deployment`; the runtime dispatches to the right strategy
//! (rolling, blue/green, canary) based on the module's manifest.

use anyhow::{bail, Result};
use clap::Args;
use serde_json::json;

use super::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct DeployArgs {
    /// Cave module/crate name (e.g. `cave-apiserver`).
    pub module: String,

    /// Target revision (commit SHA, tag, or `head`).
    #[arg(long, default_value = "head")]
    pub revision: String,

    /// Rollout strategy: `rolling`, `blue-green`, `canary`.
    #[arg(long, default_value = "rolling")]
    pub strategy: String,

    /// Tenant scope. Defaults to current shell tenant.
    #[arg(long)]
    pub tenant: Option<String>,

    /// Dry-run; render the plan without applying.
    #[arg(long)]
    pub dry_run: bool,
}

const STRATEGIES: &[&str] = &["rolling", "blue-green", "canary"];

pub fn prepare(args: &DeployArgs) -> Result<PreparedRequest> {
    validate_module(&args.module)?;
    if !STRATEGIES.contains(&args.strategy.as_str()) {
        bail!(
            "unknown strategy `{}`; want one of {:?}",
            args.strategy,
            STRATEGIES
        );
    }
    let mut body = json!({
        "module": args.module,
        "revision": args.revision,
        "strategy": args.strategy,
    });
    if let Some(t) = &args.tenant {
        body["tenant"] = json!(t);
    }
    if args.dry_run {
        body["dry_run"] = json!(true);
    }
    Ok(PreparedRequest::new(HttpVerb::Post, "/api/native/deploy").with_body(body))
}

/// Cave modules are crate names: lowercase, alnum + `-`, must start
/// with `cave-`.
pub fn validate_module(m: &str) -> Result<()> {
    if !m.starts_with("cave-") {
        bail!("module must be a cave-* crate (got `{}`)", m);
    }
    if m.len() < 6 || m.len() > 64 {
        bail!("module name length must be 6..=64");
    }
    for ch in m.chars() {
        if !(ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '-') {
            bail!("module allows only [a-z0-9-]");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(module: &str) -> DeployArgs {
        DeployArgs {
            module: module.into(),
            revision: "head".into(),
            strategy: "rolling".into(),
            tenant: None,
            dry_run: false,
        }
    }

    #[test]
    fn deploy_default_post() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(r.path, "/api/native/deploy");
    }

    #[test]
    fn deploy_module_in_body() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert_eq!(r.body.unwrap()["module"], "cave-apiserver");
    }

    #[test]
    fn deploy_default_revision_head() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert_eq!(r.body.unwrap()["revision"], "head");
    }

    #[test]
    fn deploy_default_strategy_rolling() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert_eq!(r.body.unwrap()["strategy"], "rolling");
    }

    #[test]
    fn deploy_strategies_round_trip() {
        for s in STRATEGIES {
            let mut a = args("cave-apiserver");
            a.strategy = (*s).into();
            assert!(prepare(&a).is_ok(), "strategy {} should parse", s);
        }
    }

    #[test]
    fn deploy_rejects_unknown_strategy() {
        let mut a = args("cave-apiserver");
        a.strategy = "yolo".into();
        assert!(prepare(&a).is_err());
    }

    #[test]
    fn deploy_with_tenant() {
        let mut a = args("cave-apiserver");
        a.tenant = Some("acme".into());
        assert_eq!(prepare(&a).unwrap().body.unwrap()["tenant"], "acme");
    }

    #[test]
    fn deploy_omits_tenant_when_unset() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert!(r.body.unwrap().get("tenant").is_none());
    }

    #[test]
    fn deploy_dry_run_in_body() {
        let mut a = args("cave-apiserver");
        a.dry_run = true;
        assert_eq!(prepare(&a).unwrap().body.unwrap()["dry_run"], true);
    }

    #[test]
    fn deploy_omits_dry_run_when_false() {
        let r = prepare(&args("cave-apiserver")).unwrap();
        assert!(r.body.unwrap().get("dry_run").is_none());
    }

    #[test]
    fn deploy_revision_sha() {
        let mut a = args("cave-apiserver");
        a.revision = "abcdef0".into();
        assert_eq!(prepare(&a).unwrap().body.unwrap()["revision"], "abcdef0");
    }

    #[test]
    fn deploy_revision_tag() {
        let mut a = args("cave-apiserver");
        a.revision = "v1.2.3".into();
        assert_eq!(prepare(&a).unwrap().body.unwrap()["revision"], "v1.2.3");
    }

    #[test]
    fn validate_accepts_cave_module() {
        assert!(validate_module("cave-apiserver").is_ok());
        assert!(validate_module("cave-cri").is_ok());
        assert!(validate_module("cave-net").is_ok());
    }

    #[test]
    fn validate_rejects_non_cave_prefix() {
        assert!(validate_module("apiserver").is_err());
        assert!(validate_module("nginx").is_err());
    }

    #[test]
    fn validate_rejects_uppercase() {
        assert!(validate_module("Cave-X").is_err());
    }

    #[test]
    fn validate_rejects_underscore() {
        assert!(validate_module("cave_x").is_err());
    }

    #[test]
    fn validate_rejects_too_short() {
        assert!(validate_module("cave-").is_err());
    }

    #[test]
    fn validate_rejects_too_long() {
        let long = format!("cave-{}", "a".repeat(60));
        assert!(validate_module(&long).is_err());
    }

    #[test]
    fn validate_accepts_min_length_module() {
        assert!(validate_module("cave-x").is_ok());
    }

    #[test]
    fn deploy_blue_green() {
        let mut a = args("cave-x");
        a.strategy = "blue-green".into();
        assert_eq!(prepare(&a).unwrap().body.unwrap()["strategy"], "blue-green");
    }

    #[test]
    fn deploy_canary() {
        let mut a = args("cave-x");
        a.strategy = "canary".into();
        assert_eq!(prepare(&a).unwrap().body.unwrap()["strategy"], "canary");
    }

    #[test]
    fn deploy_rejects_bad_module() {
        let r = prepare(&args("not-cave"));
        assert!(r.is_err());
    }

    #[test]
    fn deploy_endpoint_is_native_path() {
        let r = prepare(&args("cave-x")).unwrap();
        assert!(r.path.starts_with("/api/native/"));
    }

    #[test]
    fn deploy_body_has_three_required_fields() {
        let r = prepare(&args("cave-x")).unwrap();
        let body = r.body.unwrap();
        assert!(body.get("module").is_some());
        assert!(body.get("revision").is_some());
        assert!(body.get("strategy").is_some());
    }

    #[test]
    fn deploy_dry_run_with_tenant() {
        let mut a = args("cave-x");
        a.dry_run = true;
        a.tenant = Some("acme".into());
        let body = prepare(&a).unwrap().body.unwrap();
        assert_eq!(body["dry_run"], true);
        assert_eq!(body["tenant"], "acme");
    }
}
