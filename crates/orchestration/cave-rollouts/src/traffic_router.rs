// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Traffic router parity — splits canary / stable weights across the supported
//! providers from `argoproj/argo-rollouts v1.9.0`:
//!
//! * Istio        (`rollout/trafficrouting/istio`)
//! * SMI          (`rollout/trafficrouting/smi`)
//! * NGINX        (`rollout/trafficrouting/nginx`)
//! * AWS ALB      (`rollout/trafficrouting/alb`)
//! * Apisix       (`rollout/trafficrouting/apisix`)
//! * Plugin       (`rollout/trafficrouting/plugin`)
//!
//! Pure shape — emits the manifest patch a downstream controller would apply.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct WeightSplit {
    pub stable: u8,
    pub canary: u8,
}

impl WeightSplit {
    pub fn new(canary: u8) -> Self {
        let canary = canary.min(100);
        Self {
            stable: 100 - canary,
            canary,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum TrafficProvider {
    Istio {
        virtual_service: String,
        namespace: String,
    },
    Smi {
        trafficsplit_name: String,
        namespace: String,
    },
    Nginx {
        stable_ingress: String,
        namespace: String,
    },
    Alb {
        ingress: String,
        annotation_prefix: String,
    },
    Apisix {
        route: String,
        namespace: String,
    },
    Plugin {
        plugin_name: String,
        config: serde_json::Value,
    },
    Traefik {
        traefik_service: String,
        namespace: String,
    },
}

pub fn render_patch(
    provider: &TrafficProvider,
    split: &WeightSplit,
    stable_service: &str,
    canary_service: &str,
) -> serde_json::Value {
    match provider {
        TrafficProvider::Istio {
            virtual_service,
            namespace,
        } => serde_json::json!({
            "apiVersion": "networking.istio.io/v1beta1",
            "kind": "VirtualService",
            "metadata": { "name": virtual_service, "namespace": namespace },
            "spec": {
                "http": [{
                    "route": [
                        {"destination": {"host": stable_service}, "weight": split.stable},
                        {"destination": {"host": canary_service}, "weight": split.canary},
                    ]
                }]
            }
        }),
        TrafficProvider::Smi {
            trafficsplit_name,
            namespace,
        } => serde_json::json!({
            "apiVersion": "split.smi-spec.io/v1alpha1",
            "kind": "TrafficSplit",
            "metadata": { "name": trafficsplit_name, "namespace": namespace },
            "spec": {
                "service": stable_service,
                "backends": [
                    {"service": stable_service, "weight": split.stable},
                    {"service": canary_service, "weight": split.canary},
                ]
            }
        }),
        TrafficProvider::Nginx {
            stable_ingress,
            namespace,
        } => serde_json::json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "Ingress",
            "metadata": {
                "name": format!("{stable_ingress}-canary"),
                "namespace": namespace,
                "annotations": {
                    "nginx.ingress.kubernetes.io/canary": "true",
                    "nginx.ingress.kubernetes.io/canary-weight": split.canary.to_string(),
                }
            }
        }),
        TrafficProvider::Alb {
            ingress,
            annotation_prefix,
        } => serde_json::json!({
            "apiVersion": "networking.k8s.io/v1",
            "kind": "Ingress",
            "metadata": {
                "name": ingress,
                "annotations": {
                    format!("{annotation_prefix}/actions.weighted-routing"): serde_json::json!({
                        "Type": "forward",
                        "ForwardConfig": {
                            "TargetGroups": [
                                {"ServiceName": stable_service, "Weight": split.stable},
                                {"ServiceName": canary_service, "Weight": split.canary},
                            ]
                        }
                    }).to_string()
                }
            }
        }),
        TrafficProvider::Apisix { route, namespace } => serde_json::json!({
            "apiVersion": "apisix.apache.org/v2",
            "kind": "ApisixRoute",
            "metadata": { "name": route, "namespace": namespace },
            "spec": {
                "http": [{
                    "name": "canary",
                    "backends": [
                        {"serviceName": stable_service, "weight": split.stable},
                        {"serviceName": canary_service, "weight": split.canary},
                    ]
                }]
            }
        }),
        TrafficProvider::Plugin {
            plugin_name,
            config,
        } => serde_json::json!({
            "plugin": plugin_name,
            "config": config,
            "split": {"stable": split.stable, "canary": split.canary},
            "services": {"stable": stable_service, "canary": canary_service},
        }),
        TrafficProvider::Traefik {
            traefik_service,
            namespace,
        } => serde_json::json!({
            "apiVersion": "traefik.io/v1alpha1",
            "kind": "TraefikService",
            "metadata": { "name": traefik_service, "namespace": namespace },
            "spec": {
                "weighted": {
                    "services": [
                        {"name": stable_service, "weight": split.stable},
                        {"name": canary_service, "weight": split.canary},
                    ]
                }
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weight_split_clamps_to_100() {
        let s = WeightSplit::new(150);
        assert_eq!(s.canary, 100);
        assert_eq!(s.stable, 0);
    }

    #[test]
    fn weight_split_sums_to_100() {
        for c in [0u8, 5, 25, 50, 75, 99, 100] {
            let s = WeightSplit::new(c);
            assert_eq!(s.stable as u16 + s.canary as u16, 100);
        }
    }

    #[test]
    fn istio_patch_carries_two_destinations() {
        let p = TrafficProvider::Istio {
            virtual_service: "rollouts-demo".into(),
            namespace: "argo".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(20), "stable", "canary");
        let routes = &patch["spec"]["http"][0]["route"];
        assert_eq!(routes[0]["weight"], 80);
        assert_eq!(routes[1]["weight"], 20);
        assert_eq!(routes[0]["destination"]["host"], "stable");
        assert_eq!(routes[1]["destination"]["host"], "canary");
    }

    #[test]
    fn smi_patch_uses_backends() {
        let p = TrafficProvider::Smi {
            trafficsplit_name: "ts".into(),
            namespace: "argo".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(10), "stable", "canary");
        assert_eq!(patch["spec"]["backends"][0]["weight"], 90);
        assert_eq!(patch["spec"]["backends"][1]["weight"], 10);
    }

    #[test]
    fn nginx_patch_emits_canary_annotation() {
        let p = TrafficProvider::Nginx {
            stable_ingress: "demo".into(),
            namespace: "argo".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(33), "s", "c");
        assert_eq!(
            patch["metadata"]["annotations"]["nginx.ingress.kubernetes.io/canary-weight"],
            "33"
        );
    }

    #[test]
    fn alb_patch_renders_forward_config_string() {
        let p = TrafficProvider::Alb {
            ingress: "demo".into(),
            annotation_prefix: "alb.ingress.kubernetes.io".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(40), "s", "c");
        let fwd = patch["metadata"]["annotations"]
            ["alb.ingress.kubernetes.io/actions.weighted-routing"]
            .as_str()
            .unwrap();
        assert!(fwd.contains("\"Weight\":60"));
        assert!(fwd.contains("\"Weight\":40"));
    }

    #[test]
    fn plugin_patch_round_trips_config() {
        let p = TrafficProvider::Plugin {
            plugin_name: "my-router".into(),
            config: serde_json::json!({"region": "eu"}),
        };
        let patch = render_patch(&p, &WeightSplit::new(5), "s", "c");
        assert_eq!(patch["config"]["region"], "eu");
        assert_eq!(patch["split"]["canary"], 5);
    }

    #[test]
    fn apisix_patch_uses_named_route() {
        let p = TrafficProvider::Apisix {
            route: "checkout".into(),
            namespace: "argo".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(50), "s", "c");
        assert_eq!(patch["spec"]["http"][0]["name"], "canary");
        assert_eq!(patch["spec"]["http"][0]["backends"][0]["weight"], 50);
    }

    #[test]
    fn traefik_patch_weights_weighted_services() {
        // argo-rollouts v1.9.0 rollout/trafficrouting/traefik: TraefikService CRD,
        // weights live at spec.weighted.services[{name, weight}]; canary = desired,
        // stable = 100 - desired.
        let p = TrafficProvider::Traefik {
            traefik_service: "rollouts-demo".into(),
            namespace: "argo".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(30), "stable-svc", "canary-svc");
        assert_eq!(patch["apiVersion"], "traefik.io/v1alpha1");
        assert_eq!(patch["kind"], "TraefikService");
        assert_eq!(patch["metadata"]["name"], "rollouts-demo");
        assert_eq!(patch["metadata"]["namespace"], "argo");
        let svcs = &patch["spec"]["weighted"]["services"];
        assert_eq!(svcs[0]["name"], "stable-svc");
        assert_eq!(svcs[0]["weight"], 70);
        assert_eq!(svcs[1]["name"], "canary-svc");
        assert_eq!(svcs[1]["weight"], 30);
    }

    #[test]
    fn ambassador_patch_builds_canary_mapping() {
        // argo-rollouts v1.9.0 rollout/trafficrouting/ambassador: clones the base
        // Mapping into "<name>-canary", repoints spec.service to the canary service
        // and sets spec.weight = desired (int).
        let p = TrafficProvider::Ambassador {
            mapping: "demo-mapping".into(),
            namespace: "argo".into(),
        };
        let patch = render_patch(&p, &WeightSplit::new(25), "stable-svc", "canary-svc");
        assert_eq!(patch["apiVersion"], "getambassador.io/v2");
        assert_eq!(patch["kind"], "Mapping");
        assert_eq!(patch["metadata"]["name"], "demo-mapping-canary");
        assert_eq!(patch["metadata"]["namespace"], "argo");
        assert_eq!(patch["spec"]["service"], "canary-svc");
        assert_eq!(patch["spec"]["weight"], 25);
    }

    #[test]
    fn weight_split_serde_roundtrip() {
        let s = WeightSplit::new(37);
        let j = serde_json::to_string(&s).unwrap();
        let back: WeightSplit = serde_json::from_str(&j).unwrap();
        assert_eq!(s, back);
    }
}
