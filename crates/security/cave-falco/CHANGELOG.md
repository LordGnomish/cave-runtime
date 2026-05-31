# Changelog — cave-falco

All notable changes to this crate are recorded here.
Format follows [Keep a Changelog](https://keepachangelog.com/) loosely;
versions track the workspace cadence rather than per-crate semver until
the runtime hits `v1.0`.

## [Unreleased]

- Tracked under `parity.manifest.toml` — see `docs/PARITY_INDEX.md` for the
  current fill / honest ratios.

### 2026-05-31 — honest uplift (`honest_ratio` 0.7308 → 0.7667)

Six strict-TDD cycles add four userspace Falco features and close the engine
grammar partial:

- **output-rate-limiting** — libsinsp `token_bucket` + `falco_outputs`
  notification throttle (`src/token_bucket.rs`, `src/output.rs::OutputThrottle`).
- **engine-tag-selection** — `-T`/`-t` tag selection (`enable_rule_by_tag`).
- **rule-append-override** — `rule_loader_collector` append/replace
  (`src/overrides.rs`).
- **falcoctl-artifact-index** — falcoctl `index.Index` + `ResolveReference`
  (`src/falcoctl.rs`; new upstream `falcosecurity/falcoctl` v0.13.0).
- **engine-condition-grammar-full** — full libsinsp operator set (numeric,
  glob/iglob, icontains, regex, exists, pmatch, intersects, CIDR v4/v6); the
  partial is now mapped.
- Wired `cavectl falco {operators, artifact-resolve}`, `/api/falco/operators`,
  `/api/falco/artifact/resolve`, and mounted `cave_falco::router()` in
  cave-runtime. Tests 84 → 134.

## [0.1.0] — 2026-05-22

Initial OSS launch (see ADR-148 — OSS launch history strategy).
The pre-launch history was squashed; see the workspace `CHANGELOG`
or `docs/oss-launch-final-audit-2026-05-19.md` for the launch audit.
