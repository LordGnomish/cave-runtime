//! `cavectl kubectl create -f file.yaml`

use anyhow::{Context, Result};
use clap::Args;
use serde_json::Value;

use crate::compat::kubectl::resource::ns_path;
use crate::native::{HttpVerb, PreparedRequest};

#[derive(Args, Debug, Clone)]
pub struct KubectlCreateArgs {
    #[arg(short = 'f', long = "filename")]
    pub filename: String,
    #[arg(short = 'n', long)]
    pub namespace: Option<String>,
    #[arg(long = "dry-run")]
    pub dry_run: Option<String>,
    #[arg(long = "save-config")]
    pub save_config: bool,
}

pub fn parse_manifest(body: &str) -> Result<Value> {
    if let Ok(v) = serde_json::from_str::<Value>(body) {
        return Ok(v);
    }
    serde_yaml::from_str(body).context("manifest is neither valid JSON nor YAML")
}

pub fn prepare(args: &KubectlCreateArgs, manifest: &Value) -> Result<PreparedRequest> {
    let kind = manifest
        .get("kind")
        .and_then(|v| v.as_str())
        .context("manifest missing `kind`")?;
    let resource = pluralise_kind(kind);
    let manifest_ns = manifest
        .get("metadata")
        .and_then(|m| m.get("namespace"))
        .and_then(|n| n.as_str());
    let ns = args.namespace.as_deref().or(manifest_ns);
    let mut path = ns_path(&resource, ns, false)?;
    let mut params: Vec<String> = Vec::new();
    if let Some(d) = &args.dry_run {
        match d.as_str() {
            "client" | "server" | "none" => {
                params.push(format!("dryRun={}", d));
            }
            _ => anyhow::bail!("invalid --dry-run value `{}`", d),
        }
    }
    if args.save_config {
        params.push("saveConfig=true".to_string());
    }
    if !params.is_empty() {
        path.push('?');
        path.push_str(&params.join("&"));
    }
    Ok(PreparedRequest::new(HttpVerb::Post, path).with_body(manifest.clone()))
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
        "replicaset" => "replicasets".into(),
        "job" => "jobs".into(),
        "cronjob" => "cronjobs".into(),
        other => format!("{}s", other),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn args() -> KubectlCreateArgs {
        KubectlCreateArgs {
            filename: "x.yaml".into(),
            namespace: None,
            dry_run: None,
            save_config: false,
        }
    }

    #[test]
    fn parse_json() {
        let v = parse_manifest(r#"{"kind":"Pod"}"#).unwrap();
        assert_eq!(v["kind"], "Pod");
    }

    #[test]
    fn parse_yaml() {
        let v = parse_manifest("kind: Pod\nmetadata:\n  name: x\n").unwrap();
        assert_eq!(v["kind"], "Pod");
    }

    #[test]
    fn parse_invalid() {
        assert!(parse_manifest("[unterminated").is_err());
    }

    #[test]
    fn create_pod_default_ns() {
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&args(), &m).unwrap();
        assert_eq!(r.verb, HttpVerb::Post);
        assert_eq!(
            r.path,
            "/api/compat/kubectl/v1/namespaces/default/pods"
        );
    }

    #[test]
    fn create_uses_manifest_namespace() {
        let m = json!({"kind": "Pod", "metadata": {"name": "x", "namespace": "in-manifest"}});
        let r = prepare(&args(), &m).unwrap();
        assert!(r.path.contains("/namespaces/in-manifest/"));
    }

    #[test]
    fn cli_namespace_overrides() {
        let mut a = args();
        a.namespace = Some("override".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x", "namespace": "in-manifest"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("/namespaces/override/"));
    }

    #[test]
    fn dry_run_client() {
        let mut a = args();
        a.dry_run = Some("client".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("dryRun=client"));
    }

    #[test]
    fn dry_run_server() {
        let mut a = args();
        a.dry_run = Some("server".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("dryRun=server"));
    }

    #[test]
    fn dry_run_invalid() {
        let mut a = args();
        a.dry_run = Some("yolo".into());
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        assert!(prepare(&a, &m).is_err());
    }

    #[test]
    fn save_config_flag() {
        let mut a = args();
        a.save_config = true;
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}});
        let r = prepare(&a, &m).unwrap();
        assert!(r.path.contains("saveConfig=true"));
    }

    #[test]
    fn missing_kind_errors() {
        let m = json!({"metadata": {"name": "x"}});
        assert!(prepare(&args(), &m).is_err());
    }

    #[test]
    fn body_is_full_manifest() {
        let m = json!({"kind": "Pod", "metadata": {"name": "x"}, "spec": {"containers": []}});
        let r = prepare(&args(), &m).unwrap();
        assert_eq!(r.body.unwrap(), m);
    }

    #[test]
    fn pluralise_known_kinds() {
        assert_eq!(pluralise_kind("Pod"), "pods");
        assert_eq!(pluralise_kind("Service"), "services");
        assert_eq!(pluralise_kind("Deployment"), "deployments");
        assert_eq!(pluralise_kind("StatefulSet"), "statefulsets");
        assert_eq!(pluralise_kind("DaemonSet"), "daemonsets");
        assert_eq!(pluralise_kind("Ingress"), "ingresses");
    }

    #[test]
    fn pluralise_default() {
        assert_eq!(pluralise_kind("Widget"), "widgets");
    }
}
