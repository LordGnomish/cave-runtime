# cave-cli — Charter v2 8-gate Close-out Report (first-party)

**Audit date**: 2026-05-19
**Upstream pin**: `cave-runtime @ v0.1.0` (first-party — workspace self-pin)
**Crate root**: `crates/cave-cli/`

cave-cli is the `cavectl` top-level CLI. It is a **first-party** crate
with no external upstream: there is nothing to measure parity against.
UX patterns are *inspired* by `kubectl` and the `step` CLI, but no code
is ported. The Charter v2 8-gate audit applies in modified form — see
below.

---

## TL;DR

| metric                                | value |
|---------------------------------------|---|
| upstream parity entries                | n/a — first-party |
| `infra_only`                           | **true** |
| `parity_ratio_source`                  | `"infra_only"` |
| `source_sha`                           | `"v0.1.0"` (cave-runtime workspace version) |
| `last_audit`                           | `2026-05-19` |
| SPDX `AGPL-3.0-or-later` coverage      | 100% (53/53 `.rs` files in `src/`) |
| `unimplemented!` / `todo!` in `src/`   | 0 |

---

## Scope

`cavectl` is the operator-facing CLI shipping with cave-runtime. It
exposes top-level command groups that each delegate to the per-domain
crate runtime APIs:

* `cavectl cluster` — init / join / status / destroy (real plumbing landed)
* `cavectl auth`, `cavectl keda`, `cavectl rdbms`, `cavectl streams`,
  `cavectl portal` — domain command groups (≥ 30 top-level groups today)
* `cavectl portal audit` — Charter v2 per-crate matrix dashboard

Inspired-by, not ported-from:

* `kubectl` (kubernetes/kubernetes) — apply / get / describe verb taxonomy
* `step` (smallstep/cli) — leaf-cert / WebAuthn / kubeconfig flows
* `argocd` CLI — declarative apply UX

No upstream files are mirrored 1:1; the cave-cli source tree is sovereign.

---

## Charter v2 8-gate status — **8/8 PASS** (first-party variant)

| # | Gate (first-party variant)            | Status | Evidence                                  |
|---|---------------------------------------|--------|-------------------------------------------|
| 1 | SPDX `AGPL-3.0-or-later` on every `.rs` | PASS | 53/53 (`gate_1_spdx_full_coverage`)       |
| 2 | `source_sha` pins workspace version    | PASS  | `[upstream].source_sha = "v0.1.0"`        |
| 3 | `last_audit = "2026-05-19"`            | PASS  | `[parity].last_audit`                     |
| 4 | `parity_ratio_source = "infra_only"`   | PASS  | `[parity].parity_ratio_source`            |
| 5 | First-party fill_ratio exemption       | PASS  | `infra_only = true` + `first_party = true` |
| 6 | First-party marker present             | PASS  | `[module].first_party = true`             |
| 7 | no `unimplemented!()` / `todo!()` in `src/` | PASS | `gate_7_no_stub_macros_in_src`        |
| 8 | `PARITY_REPORT.md` exists with stamp   | PASS  | this file (`gate_8_parity_report_exists`) |

All nine `tests/parity_self_audit.rs` assertions pass.

---

## Notes

* The `parity_ratio_source = "infra_only"` value (instead of `"manifest"`)
  is the Charter v2 signal to `/admin/compliance` that this crate is
  exempt from the fill_ratio floor. The compliance dashboard already
  honours `infra_only = true` via the existing `infra_only_exempt` axis.
* `ratio = 0.0` is **honest**, not a placeholder: there is no upstream
  to compute a ratio against. Compare to crates that incorrectly used
  `ratio = 0.0` as a "not yet measured" placeholder against a real
  upstream — those got fixed in the data-persistence / cave-net /
  cave-mesh close-outs landing in this same branch.
