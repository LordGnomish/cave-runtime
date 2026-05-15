# cave-mesh

Full Istio-parity service mesh control plane for sovereign Cloud OS.

## Status

This crate is in pre-OSS-launch development, with parity tracked against `istio/istio`. The implementation covers the full xDS surface, including sidecar and ambient modes, as well as multi-cluster federation capabilities.

## Upstream

- [istio/istio](https://github.com/istio/istio) — Reference implementation for service mesh control plane logic and xDS protocol definitions.

## Surface ported

- xDS discovery service (ADS, EDS, RDS, CDS, LDS, SDS)
- mTLS + SPIFFE identity (cert rotation, trust domain)
- Authorization policy (RBAC, JWT validation)
- Traffic mgmt: VirtualService, DestinationRule, weighted routing, retries, fault injection
- Circuit breakers + rate limiting
- Sidecar mode + ambient mode (sidecarless)
- Service registry + config registry (Istio CRDs)
- Observability: telemetry pipeline, metrics, distributed tracing hooks
- Multi-cluster federation (primary-remote topology)
- Envoy admin proxy passthrough

## Public API

- `cave_mesh::routes::router()` — Istio-style HTTP control-plane API
- `cave_mesh::xds` — gRPC xDS server primitives
- `cave_mesh::spiffe` — SPIFFE/SPIRE identity primitives
- `cave_mesh::models` — CRD-shaped type catalog
- See parity.manifest.toml for upstream→local file mapping.

## Tests

Integration tests cover traffic policies, mTLS handshakes, xDS response correctness, and multi-cluster mode. Mode B-prime expansions are pending.

## License

Apache-2.0 (matches Istio).

## See also

- ../cave-net (Cilium L3/L4 datapath)
- ../cave-gateway (Kong, north-south ingress)
- ../cave-vault (mesh secrets storage backend for SPIFFE root CA)
