// SPDX-License-Identifier: AGPL-3.0-or-later
//! kubectl resource short-name normalisation.
//!
//! kubectl accepts `po`, `svc`, `deploy`, `ns`, `cm`, etc. The shim
//! maps these to canonical pluralised names so the routed path looks
//! like `/api/compat/kubectl/v1/namespaces/<ns>/<resource>`.

use anyhow::{bail, Result};

pub fn canonical(r: &str) -> Result<String> {
    let lower = r.to_lowercase();
    let s = match lower.as_str() {
        "po" | "pod" | "pods" => "pods",
        "svc" | "service" | "services" => "services",
        "deploy" | "deployment" | "deployments" => "deployments",
        "rs" | "replicaset" | "replicasets" => "replicasets",
        "ds" | "daemonset" | "daemonsets" => "daemonsets",
        "sts" | "statefulset" | "statefulsets" => "statefulsets",
        "ns" | "namespace" | "namespaces" => "namespaces",
        "no" | "node" | "nodes" => "nodes",
        "cm" | "configmap" | "configmaps" => "configmaps",
        "secret" | "secrets" => "secrets",
        "ing" | "ingress" | "ingresses" => "ingresses",
        "ep" | "endpoint" | "endpoints" => "endpoints",
        "pvc" => "persistentvolumeclaims",
        "pv" => "persistentvolumes",
        "sa" | "serviceaccount" | "serviceaccounts" => "serviceaccounts",
        "job" | "jobs" => "jobs",
        "cj" | "cronjob" | "cronjobs" => "cronjobs",
        "hpa" => "horizontalpodautoscalers",
        "pdb" => "poddisruptionbudgets",
        "" => bail!("resource cannot be empty"),
        other => other,
    };
    Ok(s.to_string())
}

/// Build the namespace-aware base path for a kubectl-style request.
pub fn ns_path(resource: &str, namespace: Option<&str>, all_namespaces: bool) -> Result<String> {
    let r = canonical(resource)?;
    if all_namespaces {
        return Ok(format!("/api/compat/kubectl/v1/{}", r));
    }
    let ns = namespace.unwrap_or("default");
    Ok(format!("/api/compat/kubectl/v1/namespaces/{}/{}", ns, r))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pod_aliases() {
        assert_eq!(canonical("po").unwrap(), "pods");
        assert_eq!(canonical("pod").unwrap(), "pods");
        assert_eq!(canonical("PODS").unwrap(), "pods");
    }

    #[test]
    fn svc_alias() {
        assert_eq!(canonical("svc").unwrap(), "services");
    }

    #[test]
    fn deploy_alias() {
        assert_eq!(canonical("deploy").unwrap(), "deployments");
    }

    #[test]
    fn pvc_alias() {
        assert_eq!(canonical("pvc").unwrap(), "persistentvolumeclaims");
    }

    #[test]
    fn ns_alias() {
        assert_eq!(canonical("ns").unwrap(), "namespaces");
    }

    #[test]
    fn passthrough_unknown() {
        assert_eq!(canonical("widgets").unwrap(), "widgets");
    }

    #[test]
    fn empty_rejected() {
        assert!(canonical("").is_err());
    }

    #[test]
    fn ns_path_default() {
        assert_eq!(
            ns_path("pods", None, false).unwrap(),
            "/api/compat/kubectl/v1/namespaces/default/pods"
        );
    }

    #[test]
    fn ns_path_named() {
        assert_eq!(
            ns_path("pods", Some("kube-system"), false).unwrap(),
            "/api/compat/kubectl/v1/namespaces/kube-system/pods"
        );
    }

    #[test]
    fn ns_path_all() {
        assert_eq!(
            ns_path("pods", None, true).unwrap(),
            "/api/compat/kubectl/v1/pods"
        );
    }

    #[test]
    fn ns_path_short_alias() {
        assert_eq!(
            ns_path("po", None, false).unwrap(),
            "/api/compat/kubectl/v1/namespaces/default/pods"
        );
    }

    #[test]
    fn ns_path_empty_resource_errors() {
        assert!(ns_path("", None, false).is_err());
    }
}
