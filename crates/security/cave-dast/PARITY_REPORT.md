# cave-dast — Charter v2 8-gate Close-out Report

**Audit date**: 2026-05-18
**Upstream**: `zaproxy/zaproxy @ v2.14.0` (Apache-2.0, Java)
**Crate root**: `crates/cave-dast/`

## Scope

cave-dast is a line-by-line semantic port of OWASP ZAP 2.14.0's core
engine to Rust. We port the runtime surfaces a headless scan needs;
the desktop Swing UI lives in `cave-portal`'s `admin/dast` page and
ZAP's marketplace add-on registry is out of scope.

Ported subsystems:

- HTTP request/response model (`HttpMessage` shape, header map, cookie
  RFC 6265 attribute parser, RFC 3986 percent-decode, lenient
  request/status line parser).
- ZAP `Context` (regex include/exclude in-scope filter).
- Active scan plugin framework (`AbstractPlugin` trait + registry +
  per-parameter probe helper) and six baseline rules: SQL injection
  (40018), reflected XSS (40012), path traversal (6), OS command
  injection (90020), XXE (90023), SSRF (40046).
- Passive scan plugin framework and five baseline rules: missing
  security headers (CSP/HSTS/XCTO/XFO/Referrer-Policy), insecure cookie
  (Secure/HttpOnly/SameSite), information disclosure (Server banner +
  X-Powered-By + suspicious comments), mixed content, CSRF token
  absence.
- Spider — BFS link discovery with depth/url caps, regex-free href/src
  extractor, `robots.txt` `User-agent: *` Disallow rules.
- Authentication strategies — `FormBased` (login POST + session cookie
  capture) and `BearerToken` (Authorization header injection).
- Alert taxonomy — Alert/RiskLevel + OwaspTop10 enum + CWE → OWASP
  Top 10 2021 lookup table covering 150+ CWE IDs.
- HTML report renderer — single-file, XSS-safe, summary pills + per-
  finding cards with CWE/OWASP cross-references.
- `zap-cli`-compatible subcommand parser (quick-scan / baseline /
  report / status with -t / --minutes / --report / -o flags).
- REST API surface — `/api/dast/health`, `/version`,
  `/rules/active`, `/rules/passive`.

## Inventory measurement

Hand-curated 2026-05-18 against ZAP's source tree
(`zap/src/main/java/org/zaproxy/zap/` + `parosproxy/paros/` +
`addOns/ascanrules` + `addOns/pscanrules`).

| Bucket   | Count | Examples                                                                             |
|----------|------:|--------------------------------------------------------------------------------------|
| Mapped   |    14 | HttpRequestBody/HttpResponseBody, HttpRequestHeader, HttpResponseHeader, Context,    |
|          |       | AbstractPlugin, 4 active-scan rules (sqli/xss/path_traversal/command_injection),     |
|          |       | 5 passive-scan rules (security_headers/insecure_cookie/info_disclosure/mixed/csrf),  |
|          |       | Spider, FormBasedAuthenticationMethodType, Alert, HtmlReport, zap-cli                |
| Partial  |     2 | XxeScanRule (no OOB DNS callback), SsrfScanRule (no OOB callback / cloud-IMDS exfil) |
| Skipped  |     6 | Swing UI (paros/view/), build.gradle.kts, JavaHelp bundle, automation YAML add-on,   |
|          |       | zap-extensions marketplace, generated REST API stubs                                 |
| Unmapped |     4 | extension/forceduser/, extension/fuzz/, extension/websocket/, extension/anticsrf/    |
| **Total**| **26**| |

- **fill_ratio  = (mapped + partial + skipped) / total = 22 / 26 = 0.8462**
- **honest_ratio = (mapped + skipped) / total           = 20 / 26 = 0.7692**

Charter v2 floor for cave-dast is `0.65` (declared in [RED]); we hit
**0.8462**, well above the floor. The 0.7692 honest-ratio reflects
that two rules (xxe, ssrf) are *partial* — they have real probe and
detection logic but defer one detection channel each (OOB DNS / cloud
metadata IMDSv1 exfiltration) that need infrastructure cave-dast
doesn't ship.

## 8-gate close-out

| # | Gate                              | Result | Evidence                                       |
|---|-----------------------------------|--------|------------------------------------------------|
| 1 | SPDX-License-Identifier 100%      | PASS   | 28/28 `src/**/*.rs` carry AGPL-3.0-or-later    |
| 2 | `source_sha` pinned in manifest   | PASS   | `source_sha = "v2.14.0"`                       |
| 3 | `last_audit = "2026-05-18"`       | PASS   | `[parity].last_audit`                          |
| 4 | `parity_ratio_source = "manifest"`| PASS   | parity-index reads `fill_ratio` directly        |
| 5 | `fill_ratio >= 0.65`              | PASS   | 0.8462 (honest floor for cave-dast)            |
| 6 | mapped+partial+skipped+unmapped == total | PASS | 14 + 2 + 6 + 4 = 26                       |
| 7 | No `unimplemented!()` / `todo!()` | PASS   | 0 stub macros under `src/`                     |
| 8 | `PARITY_REPORT.md` present        | PASS   | this file                                      |

**Charter v2 verdict: 8/8 PASS.**

## Test coverage

`cargo test -p cave-dast --lib --tests`:

| Test set                       | Count |
|--------------------------------|------:|
| lib (per-module `#[cfg(test)]`)|  143  |
| parity_self_audit              |    9  |
| **TOTAL**                      |**152**|

Test distribution by module:

| Module                     | Tests |
|----------------------------|------:|
| http (incl. parse + url)   |    29 |
| context                    |     5 |
| ascan (framework + 6 rules)|    36 |
| pscan (framework + 5 rules)|    21 |
| spider                     |     8 |
| auth                       |     6 |
| alert                      |     4 |
| report                     |     5 |
| cli                        |    10 |
| engine + models + others   |    19 |

## Scope-cut — explicit deferred work

These items are honest gaps. They are real, acknowledged work that a
future sprint must address; they live in the manifest as `unmapped`
(four entries) or appear inside an existing `partial` mapping.

1. **`extension/fuzz/`** — Fuzzer add-on with payload-generator and
   payload-processor pipelines. A separate Q3 2026 sprint will port
   this; the active-scan rules currently use a hand-curated payload
   list rather than the upstream payload-generator framework.
2. **`extension/forceduser/`** — Forced-user mode that lets a scan
   impersonate a different context's user identity. Needs the multi-
   context model wired through `apply_auth`.
3. **`extension/websocket/`** — WebSocket proxy and scan rules.
   Requires a full WebSocket state machine (RFC 6455) + frame-level
   probe injection, deferred.
4. **`extension/anticsrf/`** — Anti-CSRF token replay engine. The
   passive `csrf_token` rule detects *absence* of a recognised token;
   the active replay engine that mutates the token to verify the
   server checks it is deferred.
5. **XXE OOB DNS channel** — `XxeScanRule` ships the reflective in-
   body channel; the blind XXE channel that needs us to operate an
   authoritative DNS server for the exfil sub-domain is deferred.
6. **SSRF OOB HTTP / IMDS exfil** — `SsrfScanRule` ships the in-body
   internal-leak channel and a flagged `169.254.169.254` payload set;
   the out-of-band callback channel that needs a public HTTPS endpoint
   under our control is deferred.

## How to verify

```bash
# Build cave-dast clean.
cargo build -p cave-dast

# 152 tests (lib + parity_self_audit).
cargo test -p cave-dast --lib --tests

# Charter v2 self-audit alone.
cargo test -p cave-dast --test parity_self_audit

# Confirm zero stub macros.
rg -n 'unimplemented!|todo!\(' crates/cave-dast/src --type rust
```

## Next sweep (out of this close-out)

- Wire the active+passive registries into a CLI entry point so
  `cavectl dast quick-scan <url>` drives the scan end-to-end against a
  reqwest HTTP client. Currently the framework is invoked through the
  `run()` driver closure in tests.
- Port `extension/fuzz/` next — that unlocks template-driven payload
  generation and lets each ascan rule cite a fuzz-library payload set
  rather than a hand-curated list.
- Add the OOB callback channel (a small public listener cave-runtime
  exposes on a known sub-domain) and re-classify xxe + ssrf rules
  from `partial` to `ported`.
