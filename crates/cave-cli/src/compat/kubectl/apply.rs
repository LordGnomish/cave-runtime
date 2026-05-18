// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl kubectl apply -f file.yaml` — server-side apply.

use anyhow::{bail, Context, Result};
use clap::Args;
use serde_json::Value;

use crate::compat::kubectl::resource::ns_path;
use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlApplyArgs {
    #[arg(short = 'f', long = "filename")]
    pub filename: String,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    /// `--dry-run=client|server|none`
    #[arg(long = "dry-run")]
    pub dry_run: Option<String>,
    /// Force conflicts on a server-side apply.
    #[arg(long)]
    pub force: bool,
    /// Field manager identity for the apply.
    #[arg(long = "field-manager")]
    pub field_manager: Option<String>,
}

pub fn prepare(args: &KubectlApplyArgs, manifest: &Value) -> Result<PreparedRequest> {
    let kind = manifest
        .get("kind")
        .and_then(|v| v.as_str())
        .context("manifest missing `kind`")?;
    let name = manifest
        .get("metadata")
        .and_then(|m| m.get("name"))
        .and_then(|n| n.as_str())
        .context("manifest missing `metadata.name`")?;
    let manifest_ns = manifest
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|n| n.as_str());
    let ns = args.namespace.as_deref().or(manifest_ns);
    let resource = pluralise_kind(kind);
    let mut path = format!("{}/{}", ns_path(&resource, ns, false)?, name);
    let mut params = vec!["apply=true".to_string()];
    if let Some(d) = &args.dry_run {
        match d.as_str() {
            "client" | "server" | "none" => params.push(format!("dryRun={}", d)),
            _ => bail!("invalid --dry-run value `{}`", d),
        }
    }
    if args.force {
        params.push("force=true".to_string());
    }
    if let Some(fm) = &args.field_manager {
        params.push(format!("fieldManager={}", fm));
    }
    path.push('?');
    path.push_str(&params.join("&"));
    Ok(PreparedRequest::new(HttpVerb::Patch, path).with_body(manifest.clone()))
}

fn pluralise_kind(kind: &str) -> String {
    match kind.to_lowercase().as_str() {
        "pod" => "pods".into(),
        "service" => "services".into(),
        "deployment" => "deployments".into(),
        "configmap" => "configmaps".into(),
        "secret" => "secrets".into(),
        "namespace" => "namespaces".into(),
        "ingress" => "ingresses".into(),
        "statefulset" => "statefulsets".into(),
        "daemonset" => "daemonsets".into(),
        other => format!("{}s", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args() -> KubectlApplyArgs {
        KubectlApplyArgs {
            filename: "x".into(),
            namespace: None,
            dry_run: None,
            force: false,
            field_manager: None,
        }
    }

    #[test]
    fn apply_uses_patch() {
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&args(), &m).unwrap();
        assert_eq!(r.verb, HttpVerb::Patch);
    }

    #[test]
    fn apply_default_path() {
        let m = json!({"kind": "Deployment", "metadata": {"name": "api"}});
        let r = prepare(&args(), &m).unwrap();
        assert!(r.path.starts_with(
            "/api/compat/kubectl/v1/namespaces/default/deployments/api?apply=true"
        ));
    }

    #[test]
    fn apply_with_namespace_override() {
        let mut a = args();
        a.namespace = Some("prod".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.starts_with(
            "/api/compat/kubectl/v1/namespaces/prod/pods/x?apply=true"
        ));
    }

    #[test]
    fn apply_force_flag() {
        let mut a = args();
        a.force = true;
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("force=true"));
    }

    #[test]
    fn apply_field_manager() {
        let mut a = args();
        a.field_manager = Some("gitops".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("fieldManager=gitops"));
    }

    #[test]
    fn apply_dry_run_server() {
        let mut a = args();
        a.dry_run = Some("server".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("dryRun=server"));
    }

    #[test]
    fn apply_invalid_dry_run() {
        let mut a = args();
        a.dry_run = Some("nope".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        assert!(prepare(&a, &m).is_err());
    }

    #[test]
    fn apply_missing_name() {
        let m = json!({"kind": "Pod"});
        assert!(prepare(&args(), &m).is_err());
    }

    #[test]
    fn apply_missing_kind() {
        let m = json!({"metadata": {"name": "x"}});
        assert!(prepare(&args(), &m).is_err());
    }

    #[test]
    fn apply_uses_manifest_namespace() {
        let m = json!({"kind": "Pod", "metadata": {"name": "x", "namespace": "in"}});
        let r = prepare(&args(), &m).unwrap();
        assert!(r.path.contains("/namespaces/in/"));
    }

    #[test]
    fn apply_body_is_manifest() {
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}, "spec": {}});
        let r = prepare(&args(), &m).unwrap();
        assert_eq!(r.body.unwrap(), m);
    }

    #[test]
    fn apply_combined_flags() {
        let mut a = args();
        a.force = true;
        a.field_manager = Some("ops".into());
        a.dry_run = Some("server".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        let p = r.path;
        assert!(p.contains("apply=true"));
        assert!(p.contains("force=true"));
        assert!(p.contains("fieldManager=ops"));
        assert!(p.contains("dryRun=server"));
    }
}
