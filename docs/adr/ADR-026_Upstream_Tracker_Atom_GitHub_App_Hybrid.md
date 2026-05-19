# ADR-026 — Upstream tracker: Atom feed primary + optional GitHub App enrichment

**Status:** Accepted
**Date:** 2026-05-19
**Author:** Burak / Cave Runtime ops

## Context

`cave-upstream-watchd` polls every tracked OSS upstream's
`/repos/<owner>/<repo>/releases/latest` endpoint to detect new
releases, diff them against `parity.manifest.toml::[upstream] version`
pins, and emit `GAP_OPENED` events into the dispatcher pipeline.

Until 2026-05-19 the implementation used the GitHub REST JSON API,
which forced two undesirable couplings:

1. **PAT-required at scale.** The anonymous JSON API limit is
   60 req/h. With ~80 tracked upstreams polled every 5 min that's
   ~960 req/h — a PAT is mandatory in any non-trivial install.
2. **Secret-on-disk.** The simplest deploy path stashed the PAT in
   `~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist`
   `EnvironmentVariables` or in `upstream-watch.toml`. Both files
   are part of routine backups + sync flows — leaks happen.

Two GitHub-issued tokens were found leaked on disk on 2026-05-19
(`gho_5J8DH…` in the poller plist, `ghp_21KnO…` in the toml). Both
have been moved into the macOS keychain under
`cave-upstream-legacy-poller` / `cave-upstream-legacy-watchd` and
are no longer read by code; the new architecture stops needing
them.

## Decision

**Atom feed becomes the primary release-detection path.**

* URL: `https://github.com/<owner>/<repo>/releases.atom`
* No authentication required — feed is public for public repos.
* Per-feed rate limit is generous (~ thousands of unauthenticated
  requests per hour, governed by GitHub's web rate-limiter, not the
  API quota).
* Supports `If-None-Match` + `If-Modified-Since` conditional
  caching, so steady-state polling returns 304 most of the time.

**Optional second path: a GitHub App for richer data.**

* When the operator stands up a GitHub App and stashes its
  private key + ID in the macOS keychain under
  `cave-upstream-github-app` and `cave-upstream-github-app-id`,
  the daemon mints an installation token via the App JWT flow and
  upgrades to the REST JSON API for that tick.
* App enrichment is **optional** — the Atom path covers every
  GAP_OPENED-relevant field (tag, published_at, body). The App is
  only useful if a downstream consumer needs asset URLs, the
  prerelease flag, or the full markdown release notes (vs. the
  Atom feed's HTML).

**Legacy PAT path stays available, marked deprecated.**

* `CAVE_WATCHD_PRIMARY=json` flips the daemon back to the old
  REST + PAT path. Used for the cave-upstream legacy binary
  shipped from `crates/cave-upstream` and for headless CI that
  can't use the keychain.

## Detection logic

Strategy resolution at tick start:

| `CAVE_WATCHD_PRIMARY` | App keychain present | Result            |
| --------------------- | -------------------- | ----------------- |
| `atom` (default)      | any                  | Atom              |
| `auto`                | yes (configured)     | Atom (Phase 2: JSON via App once token-exchange wiring lands) |
| `auto`                | no / partial         | Atom              |
| `json`                | any                  | REST + PAT        |

## Trade-offs

**What Atom gives up vs. REST JSON:**

| Field           | REST JSON  | Atom                |
| --------------- | ---------- | ------------------- |
| tag_name        | yes        | yes                 |
| published_at    | yes        | yes                 |
| body            | markdown   | HTML (→ converted)  |
| html_url        | yes        | yes                 |
| prerelease flag | yes        | **NO**              |
| asset URLs      | yes        | **NO**              |

The GAP_OPENED pipeline uses tag + published_at + body — all
covered by Atom. `prerelease` is currently ignored downstream; if
a future consumer needs it, opt that crate into the App path.

## Migration

1. The two leaked PATs were moved on 2026-05-19 into keychain
   under `cave-upstream-legacy-poller` / `cave-upstream-legacy-watchd`.
2. Plaintext PAT in `~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist`
   was removed.
3. `~/Library/Application Support/cave-runtime/upstream-watch.toml`
   `github_token` field was removed; a comment points at the new
   architecture.
4. Code refactor lives in `cave-upstream-watchd/src/atom.rs` +
   `cave-upstream-watchd/src/github_app.rs`; `bin/main.rs` resolves
   the strategy once at tick start and dispatches accordingly.

## Phase 2 (future)

* Wire `mint_app_jwt` → `GET /app/installations` →
  `POST /access_tokens` so the `auto` strategy actually upgrades
  to JSON when the App is configured.
* Audit-doc dashboard: surface which crates were last polled via
  Atom vs. App for traceability.
* Linux deploy: drop the macOS-keychain dependency in favor of
  systemd `LoadCredential=` for the App key + ID.

## Runbook

Setup instructions for the optional GitHub App are at
`docs/runbooks/github-app-setup.md`.
