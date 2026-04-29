//! Endpoints controller — fans out a Service's selector across the cluster's
//! Pods, producing the IP list of ready, matching backends.
//!
//! Upstream: pkg/controller/endpoint/endpoints_controller.go::syncService

use crate::models::{EndpointsObj, PodEndpoint, ServiceObs};

/// Reconcile a Service into its Endpoints object. Selects pods whose labels
/// match the service's selector, are Ready, and live in the same namespace.
pub fn reconcile_endpoints(svc: &ServiceObs, pods: &[PodEndpoint]) -> EndpointsObj {
    let addresses: Vec<String> = pods
        .iter()
        .filter(|p| p.namespace == svc.namespace && p.ready && labels_match(&svc.selector, &p.labels))
        .map(|p| p.ip.clone())
        .collect();

    EndpointsObj {
        service_name: svc.name.clone(),
        namespace: svc.namespace.clone(),
        addresses,
    }
}

fn labels_match(
    selector: &std::collections::HashMap<String, String>,
    labels: &std::collections::HashMap<String, String>,
) -> bool {
    selector.iter().all(|(k, v)| labels.get(k) == Some(v))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn svc(name: &str, ns: &str, selector: &[(&str, &str)]) -> ServiceObs {
        ServiceObs {
            name: name.into(),
            namespace: ns.into(),
            selector: selector.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect(),
        }
    }

    fn pod(name: &str, ns: &str, ip: &str, labels: &[(&str, &str)], ready: bool) -> PodEndpoint {
        let mut m = HashMap::new();
        for (k, v) in labels {
            m.insert((*k).into(), (*v).into());
        }
        PodEndpoint {
            pod_name: name.into(),
            namespace: ns.into(),
            ip: ip.into(),
            labels: m,
            ready,
        }
    }

    /// Port of `TestEndpointsControllerSync` — matched + ready pods become
    /// addresses; unmatched / not-ready / wrong-namespace pods are excluded.
    #[test]
    fn test_endpoints_controller_sync() {
        let s = svc("api", "default", &[("app", "api")]);
        let pods = [
            pod("a", "default", "10.0.0.1", &[("app", "api")], true),
            pod("b", "default", "10.0.0.2", &[("app", "api")], true),
            pod("c", "default", "10.0.0.3", &[("app", "api")], false), // not ready
            pod("d", "default", "10.0.0.4", &[("app", "web")], true),  // wrong selector
            pod("e", "other", "10.0.0.5", &[("app", "api")], true),    // wrong ns
        ];
        let ep = reconcile_endpoints(&s, &pods);
        assert_eq!(ep.service_name, "api");
        assert_eq!(ep.addresses.len(), 2);
        assert!(ep.addresses.contains(&"10.0.0.1".into()));
        assert!(ep.addresses.contains(&"10.0.0.2".into()));
    }

    #[test]
    fn test_endpoints_no_pods_match() {
        let s = svc("api", "default", &[("app", "api")]);
        let ep = reconcile_endpoints(&s, &[]);
        assert!(ep.addresses.is_empty());
    }
}
