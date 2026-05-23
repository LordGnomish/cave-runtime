# cave-k8s

`cave-k8s` is the cave-runtime Kubernetes **control-plane umbrella**: it
unifies the eight subsystem crates (`cave-apiserver`, `cave-scheduler`,
`cave-kubelet`, `cave-kube-proxy`, `cave-controller-manager`,
`cave-cloud-controller-manager`, `cave-cri`, `cave-etcd`) behind a
single `ControlPlane` facade and adds the cluster-wide concerns that
no single subsystem owns:

* PQC-ready ServiceAccount token signing (Ed25519 + ML-DSA-65 envelope)
* Built-in admission chain (NamespaceLifecycle + ServiceAccount +
  LimitRanger + PodSecurity + ValidatingAdmissionPolicy)
* CRD lifecycle registry + structural-schema gate
* APIService aggregator registry
* OpenAPI v3 schema composition over builtin + CRD schemas
* Generic resource manager + GarbageCollector cascade planner
* cgroupv2-only QoS classifier + cgroup path layout
* PV / PVC / StorageClass CSI-only binder
* Eviction + probe state-machine + image-GC planners
* kube-proxy facade tracking nftables / iptables / eBPF mode
* Prometheus-shaped `/metrics` scrape surface

## Upstream parity

Pinned to **`kubernetes/kubernetes v1.32.0`**
(`70d3cc986aa8221cd1dfb1121852688902d3bf53`, Apache-2.0).

`fill_ratio = 0.9516` · `honest_ratio = 0.6452` · `last_audit = 2026-05-23`.

See [`parity.manifest.toml`](parity.manifest.toml) for the per-subsystem
mapping and [`PARITY_REPORT.md`](PARITY_REPORT.md) for the Charter v2
8-gate close-out evidence.

## Quick start

```rust
use cave_k8s::{ControlPlane, ClusterConfig};

let cp = ControlPlane::new(ClusterConfig::default());
cp.start();
assert!(cp.status().is_healthy());
```

## cavectl

```sh
cavectl k8s cluster                  # cluster overview (phase + component health)
cavectl k8s version                  # kubernetes + cave-k8s version pin
cavectl k8s healthz                  # /healthz
cavectl k8s readyz                   # /readyz
cavectl k8s discovery                # /apis discovery doc
cavectl k8s openapi                  # /openapi/v3 composed schema
cavectl k8s metrics                  # Prometheus text-format metrics
cavectl k8s apply -f manifest.json   # apply a manifest
cavectl k8s scale Deployment web --replicas 5 -n prod
cavectl k8s rollout Deployment web -n prod
cavectl k8s top-nodes
cavectl k8s top-pods -n prod
cavectl k8s logs my-pod -f -n prod
cavectl k8s exec my-pod -- sh -n prod
cavectl k8s port-forward my-pod 8080:80 -n prod
```

## License

AGPL-3.0-or-later. cave-runtime contributors © 2026.
