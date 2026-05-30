<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# ADR-RUNTIME-KARPENTER-CLOUD-AGNOSTIC-001 — Port Karpenter Core, Not the AWS Provider

| Status | Accepted |
| ------ | -------- |
| Date   | 2026-05-30 |
| Track  | Backend (port in progress) |
| Supersedes scope note in | ADR-146 |

## Context

Upstream Karpenter ships as two repositories:

- **`kubernetes-sigs/karpenter`** — the cloud-agnostic *core*: the
  `NodePool` / `NodeClaim` CRD model, the scheduling engine (requirement
  set-algebra, topology, host-port and volume accounting, taint
  tolerations), the disruption controllers (consolidation, drift,
  expiration, emptiness, interruption), cluster state tracking, and the
  `CloudProvider` interface that the core calls out to. **~34.7K LOC Go**
  (v1.12.1, sha `ed490e8`), excluding tests.
- **`aws/karpenter-provider-aws`** — the most mature *provider*: a
  concrete implementation of the `CloudProvider` interface against EC2
  (instance-type discovery, fleet/spot launch, AMI resolution, subnet /
  security-group selection, interruption-queue polling). AWS-specific.

`cave-karpenter` reimplements Karpenter in Rust as part of the single
`cave-runtime` binary (per [the single-binary
charter](cave-runtime-single-binary)). The question this ADR settles:
**which of the two repos do we port?**

## Decision

**Port the cloud-agnostic core. Do not port the AWS provider.**

1. `cave-karpenter` ports `kubernetes-sigs/karpenter` line-by-line: the
   CRD model, the scheduling set-algebra, the disruption decision
   engine, cluster state, and the `CloudProvider` *trait* (Rust idiom of
   the Go interface): `create` / `delete` / `get` / `list` /
   `is_drifted` / `get_instance_types`.

2. The concrete provider behind that trait is **Cave-native**, fulfilled
   via `cave-cloud-controller-manager` (cave-CCM), which already speaks
   to the sovereign-cloud and Azure substrates the runtime targets. AWS EC2
   fleet logic from `karpenter-provider-aws` is therefore **out of
   scope** — it would couple the runtime to an AWS surface we do not
   ship. The provider boundary is the `CloudProvider` trait; everything
   above it is portable, everything below it is cave-CCM's job.

3. `honest_ratio` for this crate is measured **against the core repo
   only** (cave non-test LOC / core non-test LOC). The AWS provider LOC
   is explicitly excluded from the denominator because it is not in
   scope — counting it would understate parity against work we have
   decided not to do.

## Consequences

- The `CloudProvider` trait surface in `cave-karpenter` is the stable
  contract; cave-CCM implements it. Mock/static providers live in-crate
  for testing.
- Instance-type catalogues, pricing, and capacity signals are sourced
  from cave-CCM, not hard-coded per cloud as the AWS provider does.
- Interruption handling (spot reclaim, scheduled maintenance) is modelled
  as a generic `CloudProvider`-surfaced event, not an SQS queue poll.
- This ADR records **only** the scope boundary. It is not an
  `adr_justified` parity-credit marker — parity is earned by ported,
  tested code, measured by LOC.

## References

- Upstream core: <https://github.com/kubernetes-sigs/karpenter> (v1.12.1)
- Upstream AWS provider (out of scope): <https://github.com/aws/karpenter-provider-aws>
- ADR-146 — Karpenter adoption (scaffold)
- cave-cloud-controller-manager — provider substrate
