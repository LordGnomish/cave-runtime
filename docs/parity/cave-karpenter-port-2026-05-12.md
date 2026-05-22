# cave-karpenter parity — 2026-05-12 audit

**Upstream:** `kubernetes-sigs/karpenter v1.12.0` (Apache-2.0).

## Methodology

Standard cave-etcd pattern. Inventory enumerates the top-level
Karpenter packages and classifies each. cave-karpenter is small
(4 files, 345 LOC), so most entries are `[[unmapped]]` — the
audit reflects honest scaffold status rather than inflated
coverage.

## Counts

| Bucket   | Count |
|----------|------:|
| Mapped   | 3 |
| Skipped  | 8 |
| Unmapped | 6 |
| **Total** | **17** |
| **fill_ratio** | **0.6471** |

The `0.65` ratio is honest: the **mapped** bucket only covers the
CRD type shells + a scheduler stub + an in-memory store, while
**unmapped** carries every reconcile loop Karpenter actually does
in production (provisioning, disruption, nodeclaim lifecycle,
termination, topology solver, cloud-provider binding). The
`skipped` bucket reasonably accounts for kubebuilder boilerplate,
Helm charts, and operator-binary main glue.

## What this PR does NOT claim

* The fill_ratio does NOT claim cave-karpenter is 65% done. It
  claims 65% of the upstream's top-level packages are either
  covered (3) or explicitly skipped (8). The unmapped six are
  the real port targets.
* No new code lands in cave-karpenter from this audit pass.
