# TDD coverage audit — cave-container-scan vs Trivy v0.70.0

- Cave crate: `crates/security/cave-container-scan` (theme: security)
- Upstream: https://github.com/aquasecurity/trivy @ `v0.70.0`
- Upstream test inventory: 626 test files, 1251 `func Test*` / `it(` symbols (`/tmp/tdd-audit/cave-container-scan-upstream-tests.txt`)
- Cave test functions: 55 (`#[test]` / `#[tokio::test]` across `src/**` + `tests/**`)

## Scope reality

Trivy v0.70.0 is a large Go application: a full image/repo/fs/SBOM/K8s/VM scanner with
a vuln DB, OCI registry client, lockfile parsers for ~12 ecosystems, Rego/misconf engine
(`trivy-checks`), Redis/FS caches, RPC client-server, CycloneDX/SPDX/SARIF serializers, a
plugin marketplace and a GitHub-Actions triage bot. `cave-container-scan` ports only a
**small heuristic backend subset**: 6 regex/heuristic scanners (image-ref, IaC, fs,
secret, YARA-stub, namespace/typosquat), a verdict aggregator, a pure OpenVEX matcher, and
an axum route layer (~2,966 LOC). The overwhelming majority of upstream tests therefore
exercise machinery cave does not implement and are **scope-cut**, not gaps.

## Classification summary

| Class | Count (approx) | Notes |
|---|---|---|
| scope-cut | ~1240 | vuln-DB/OCI registry/cache(fs+redis+memory)/RPC client-server/SBOM(CycloneDX,SPDX)/SARIF/lockfile parsers/Rego misconf engine/VM/K8s-operator CRD/plugin/e2e/integration/GH-Action triage bot/Helm magefiles/version-comparers per ecosystem |
| portable-coverage (cave implements, source-verified, no test) | 3 behavioral units | listed below — the real gaps |
| missing-impl | 0 high-value | the un-ported behaviors are all scope-cut, not partial stubs that need a test |

The VEX matcher (`OpenVex::matches` / `not_affected` / `filter` / `finding_status`), which
ports Trivy `pkg/vex/openvex.go` and is the natural overlap with upstream `TestFilter`
(pkg/vex), is **already fully covered** by `tests/vex_eval_tdd.rs` (latest-statement-wins,
not_affected/fixed suppression, under_investigation kept, filter keep/remove). Not a gap.

## Portable-coverage gaps (real, PRIORITY)

These are public behaviors cave actually implements, verified in source, with **no test**
asserting the behavior. Each corresponds to an upstream behavioral unit.

| # | Cave public fn (entry point) | Implemented behavior, source-verified | Upstream analogue |
|---|---|---|---|
| 1 | `engine::ScanOrchestrator::run` (`src/engine.rs:124`) | Dispatch by `ScanKind` to the matching `Scanner`, run it, **dedupe** the result set, and map outcome→`ScanStatus` (`Completed` on Ok, `Failed` when the scanner errors **and** when no scanner matches the kind). The whole orchestration path — including the no-matching-scanner→`Failed` branch and dedup-on-result — has zero test coverage. | `scanner.Scanner.ScanArtifact` dispatch + result assembly (pkg/scanner) |
| 2 | `scanners::secret::SecretScanner::scan` → SEC-004 high-entropy branch (`src/scanners/secret.rs:67`) | The 3 pattern rules (AWS/GitHub/private-key, SEC-001/002/003) are tested, but the Shannon-entropy path (`shannon_entropy(line) >= 4.5` over base64-like ≥40-char lines emitting `SEC-004`) — the only non-trivial detector — is untested. `test_shannon_entropy_calculation` tests the helper in isolation but never drives it through `scan`. | `secret.Scanner.Scan` entropy/regex rule firing (pkg/fanal/secret — `TestSecretScanner`) |
| 3 | `scanners::iac::IacScanner::scan` → K8S-003, K8S-004, TF-002 rules (`src/scanners/iac.rs:113`, `:129`, `:169`) | Only DOCK-001, DOCK-002, K8S-001, TF-001 are tested. The `hostNetwork: true` (K8S-003), `imagePullPolicy: Always`+`:latest` (K8S-004), and `0.0.0.0/0` ingress (TF-002) misconfig rules fire in source but have no assertion. | misconf rule evaluation (pkg/iac / trivy-checks — `TestScanner`) |

## Recommended TDD fills (portable-coverage first)

1. **`ScanOrchestrator::run`** — RED test: build a `ScanOrchestrator::new(vec![Box::new(SecretScanner)])`, run a `ScanKind::Secret` request and assert `status == Completed` with deduped findings; then run a `ScanKind::Image` request (no matching scanner registered) and assert `status == Failed` with empty findings. Exercises the dispatch + no-scanner→`Failed` + dedup branches in one file.
2. **`SecretScanner::scan` (SEC-004)** — RED test: feed a `ScanTarget::Content` whose line is a ≥40-char high-entropy base64 blob (entropy ≥ 4.5) and assert a `SEC-004` finding with `Severity::High` / `Confidence::Medium`; pair with a low-entropy 40-char line asserting no SEC-004. Drives the entropy detector through the public `scan` path, not just `shannon_entropy`.
3. **`IacScanner::scan` (K8S-003 / K8S-004 / TF-002)** — RED tests: a Kubernetes bundle containing `hostNetwork: true` → `K8S-003`; a Kubernetes bundle with `imagePullPolicy: Always` and `:latest` → `K8S-004`; a Terraform bundle with `0.0.0.0/0` + `ingress` → `TF-002`. Closes the untested misconfig rules in the IaC scanner.

No further portable gaps: the verdict aggregator (`aggregate_verdict`/`evaluate_policy`),
dedupe, namespace typosquat (`is_typosquat`/`levenshtein_distance`), YARA-stub, image and
fs heuristics, and the full VEX matcher are already covered. The remaining ~1,240 upstream
test symbols target subsystems cave deliberately does not port (scope-cut).
