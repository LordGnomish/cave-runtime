<!--
SPDX-License-Identifier: AGPL-3.0-or-later
-->

# ADR-004: Zero-Trust as a Runtime Architectural Principle

## Status
Accepted

## Context
Cave Runtime is multi-tenant by construction (ADR-003).
Workloads from different tenants share the underlying control plane, the
same network fabric, and the same persistence layer. A perimeter-based
trust model — "trust everything inside the cluster" — is incompatible
with that invariant: a single compromised pod or stolen credential would
have full lateral movement.

The Runtime therefore commits to zero-trust as a non-negotiable design
property of every Runtime crate that participates in network or auth.

## Decision
Every network connection inside Cave Runtime is authenticated, authorised
and encrypted. There is no implicit trust based on source IP, source
node, source namespace, or "the cluster boundary". The principle is
enforced by Runtime crates, not by a single product.

Concretely:

1. **Mutual TLS by default for east-west traffic.**
   `cave-mesh` enforces mTLS between every service-to-service connection.
   Workload identity uses SPIFFE-compatible SVIDs; no service can call
   another without presenting a verifiable identity certificate.

2. **Default-deny L3/L4 between tenants.**
   `cave-net` ships with a default-deny posture per tenant namespace.
   Tenant-A pods cannot reach tenant-B pods unless an explicit
   `NetworkPolicy` allows it. Tenant identity is a label on every
   policy and every flow log.

3. **Identity-aware admission.**
   `cave-apiserver` admission requires a valid token tied to a Runtime
   identity (cave-auth). No anonymous control-plane writes; no static
   shared secrets in the request path.

4. **Independent control-plane mTLS.**
   `cave-runtime` listeners (apiserver, etcd, kubelet) use a separate CA
   hierarchy from data-plane mTLS. A data-plane compromise must not
   yield control-plane reach (see ADR-010).

5. **No backwards-compatible "insecure" mode.**
   The Charter forbids backcompat. There is no flag to disable mTLS, no
   flag to disable default-deny, no plaintext etcd listener. Operators
   running development clusters use the same trust model as production.

## Rationale
Zero-trust is described here as a *Runtime invariant* rather than a
deployment choice. Operators can swap the underlying eBPF / mesh /
identity implementations (Cilium, Istio, SPIRE, etc.) via Platform
profiles — Cave Runtime's reimplementations (`cave-net`, `cave-mesh`,
`cave-auth`) honour the same wire protocols. The invariant is that the
cluster never operates without the four properties above.

Specific upstream tool selections (which eBPF agent, which mesh
implementation) live on the Platform side and are inherited via
ADR-005.

## Consequences

Positive
- Tenant isolation property of ADR-003 has a concrete
  network-layer enforcement contract.
- A compromised workload cannot reach other tenants without crossing
  two independently enforced layers (L3/L4 policy and L7 mTLS).
- The Runtime has a single answer to "how do you stop lateral movement?"
  rather than a product matrix.

Negative
- Bringing up a development cluster always pays the mTLS + default-deny
  cost. There is no "fast insecure path".
- Debugging connectivity requires understanding both L3/L4 policy
  (cave-net) and L7 mTLS (cave-mesh). Tooling (Hubble-equivalent flow
  logs in `cavectl net flows`) compensates.

## Compliance Anchors
This ADR is the Runtime-side hook used by external compliance mappings
(SOC2 CC6.1/CC6.6, ISO 27001 A.8.22/A.8.24, NIS2 Art.21, GDPR Art.32).
Mapping detail lives in `docs/compliance/`, not in this ADR — those
mappings are deployment-profile specific.

## Related ADRs
- ADR-003 (why zero-trust is mandatory)
- ADR-010 (control-plane vs data-plane CA split)
- ADR-005 (how Platform tool choices map onto Runtime crates)
