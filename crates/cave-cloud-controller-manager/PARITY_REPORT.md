# cave-cloud-controller-manager — Charter v2 PARITY_REPORT

**Upstream:** `kubernetes/kubernetes` @ **v1.36.0** (pinned via `[upstream] source_sha`)
**Out-of-tree providers tracked:** `hcloud-cloud-controller-manager` (Hetzner) + `cloud-provider-azure` (Azure)
**Last audit:** 2026-05-19
**Methodology:** subsystem-level inventory of `staging/src/k8s.io/cloud-provider/` plus the two supported out-of-tree provider implementations, each classified `mapped / partial / skipped / unmapped` against the local Rust source.

## Headline numbers

| Metric                | Value  |
| --------------------- | ------ |
| `mapped_count`        | 13     |
| `partial_count`       | 1      |
| `skipped_count`       | 9      |
| `unmapped_count`      | 0      |
| `total`               | 23     |
| **`fill_ratio`**      | **1.0000** |
| **`honest_ratio`**    | **0.9565** |
| `parity_ratio_source` | manifest |
| `infra_only`          | false  |

`fill_ratio   = (mapped + partial + skipped) / total = 23 / 23`
`honest_ratio = (mapped + skipped)            / total = 22 / 23`

> **2026-05-19 uplift:** the last `[[unmapped]]` (`volume/`) was promoted to
> `[[mapped]]` by a real port — `src/cloud_volume_controller.rs` (`CloudVolume`
> trait, `HetznerCloudVolume` + `AzureDiskCloudVolume` impls, and a
> `CloudVolumeController` watch-loop with exponential backoff). Two additive
> deepening helpers were also added — `src/providers/hetzner_instance_lookup.rs`
> and `src/providers/azure_instance_lookup.rs` — porting the
> `Instances.{InstanceID,NodeAddresses,InstanceType,Zone}` quartet from each
> upstream provider behind mock-friendly client traits.

## Charter v2 8-gate ledger

| # | Gate                                       | Status |
| - | ------------------------------------------ | ------ |
| 1 | TDD-strict RED → GREEN → REFACTOR          | PASS   |
| 2 | SPDX `AGPL-3.0-or-later` on every `.rs`    | PASS (30 / 30) |
| 3 | `source_sha` pinned in `[upstream]`        | PASS (`v1.36.0`) |
| 4 | `last_audit` ≥ today                       | PASS (`2026-05-19`) |
| 5 | `parity_ratio_source = "manifest"`         | PASS   |
| 6 | No stubs (`unimplemented!`/`todo!`/TODO)   | PASS (0 hits in src) |
| 7 | 4-track (backend / portal / cavectl / obs) | PASS-backend (portal_ui + obs follow-up tracked) |
| 8 | `honest_ratio ≤ fill_ratio`                | PASS (0.9565 ≤ 1.0000) |

## Scope-cut table — what is `skipped`, with reason

| Subsystem                                     | Reason                                                             |
| --------------------------------------------- | ------------------------------------------------------------------ |
| `fake/` (test scaffolding)                    | Upstream-only test double; replaced by Rust mocks in `tests_crosscut.rs`. |
| `credentialconfig/`                           | Replaced by per-provider config (`HCLOUD_TOKEN` env, Azure MSI).   |
| `names/` (controller-name constants)          | One-line const list; not load-bearing.                             |
| `api/` (cloud-provider API types)             | Types sourced from `k8s.io/api` via `cave-apiserver`.              |
| `providers/` (in-tree legacy)                 | Removed upstream by KEP-2395 in v1.36; out-of-tree replaces in-tree. |
| `cloud-provider-aws`                          | Out of OSS-launch scope (Hetzner + Azure only).                    |
| `cloud-provider-gcp`                          | Out of OSS-launch scope.                                           |
| `cloud-provider-openstack`                    | Out of OSS-launch scope.                                           |
| `cloud-provider-vsphere`                      | Out of OSS-launch scope.                                           |

## Gap (unmapped) — what is genuinely missing

_None. The 2026-05-19 uplift closed the last gap (`volume/`) — see
`src/cloud_volume_controller.rs`._

## Self-audit coverage

`tests/parity_self_audit.rs` — 9 assertions:

1. `upstream_version_is_pinned` — `[upstream] version == "v1.36.0"`.
2. `upstream_source_sha_is_present_and_matches_version` — `source_sha` is set and equals the pinned version.
3. `parity_fill_ratio_is_measured_and_at_least_floor` — `fill_ratio ≥ 0.95`.
4. `parity_honest_ratio_does_not_exceed_fill` — `honest_ratio ≤ fill_ratio`.
5. `parity_last_audit_is_2026_05_19` — `[parity] last_audit == "2026-05-19"`.
6. `parity_infra_only_is_false` — this crate is a parity surface, not infra-only.
7. `at_least_floor_mapped_blocks` — at least 13 `[[mapped]]` blocks present.
8. `counts_sum_to_total` — `mapped + partial + skipped + unmapped == total`.
9. `every_rs_file_carries_agpl_spdx` — every `.rs` file in the crate starts with the AGPL SPDX header (and ≥ 30 `.rs` files exist).

All 9 assertions PASS as of `2026-05-19`.
