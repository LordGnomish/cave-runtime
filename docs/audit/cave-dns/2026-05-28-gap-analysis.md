# cave-dns ↔ CoreDNS v1.14.3 line-by-line gap analysis

- **Upstream**: coredns/coredns `v1.14.3` (`17fceec6d93fd1dde5ba6888c363f131ff6d647f`), Apache-2.0
- **Companion**: miekg/dns `v1.1.66` (BSD-3-Clause), vendored by CoreDNS
- **Local crate**: `crates/networking/cave-dns` (AGPL-3.0-or-later)
- **Upstream non-test Go LOC**: 35,521 across 62 `plugin/*` directories + `core/`
- **Local src LOC (pre-uplift)**: 9,823 across 69 `.rs` files, 27 plugin ports
- **Pre-uplift manifest parity**: `fill_ratio = 1.0000`, `honest_ratio = 0.7917`
  (count-based: mapped 33 / partial 3 / skipped 12 / total 48)

> **Metric note.** `honest_ratio` in this repo is a hand-authored, *count-based*
> manifest field consumed verbatim by `scripts/build-parity-index.py`
> (`parse_manifest` → `_re_float(parity_block, "honest_ratio")`). It is **not**
> LOC-derived. A naive `local_loc / upstream_loc = 9823/35521 = 0.2766` would
> *lower* the published metric and corrupt a passing crate — so this uplift
> raises honest coverage by converting partial/skipped subsystems into real,
> tested **mapped** ports, not by rewriting the metric.

## Upstream plugin × cave coverage matrix

| CoreDNS plugin | cave-dns | status |
|----------------|----------|--------|
| acl | plugins/acl.rs | ported |
| any | plugins/any.rs | ported |
| auto | plugins/auto.rs | ported |
| cache | plugins/cache.rs | ported |
| chaos | plugins/chaos.rs | ported |
| dnssec | dnssec/* + protocol/dnssec.rs | ported |
| errors | plugins/errors.rs | ported |
| etcd | plugins/etcd.rs | ported (read-path; live watch → cave-etcd) |
| file | plugins/file.rs + zone/* | ported |
| forward | plugins/forward.rs + forward.rs + resolver.rs | ported |
| health | plugins/health.rs | ported |
| hosts | plugins/hosts.rs | ported |
| kubernetes | plugins/kubernetes.rs + discovery.rs | ported |
| loadbalance | plugins/loadbalance.rs | ported |
| log | plugins/log.rs | ported |
| loop | plugins/loop_detect.rs | ported |
| metrics | plugins/metrics.rs | ported |
| ready | plugins/ready.rs | ported |
| reload | plugins/reload.rs | ported |
| rewrite | plugins/rewrite.rs | ported |
| root | plugins/root.rs | ported |
| route53 | plugins/route53.rs | ported (offline; live SDK → cave-cloud) |
| secondary | plugins/secondary.rs + zone/transfer.rs | ported |
| template | plugins/template.rs | ported |
| tls | plugins/tls.rs | ported |
| trace | plugins/trace.rs | ported |
| whoami | plugins/whoami.rs | ported |
| **bufsize** | plugins/bufsize.rs | **gap → this uplift** |
| **minimal** | plugins/minimal.rs | **gap → this uplift** |
| **nsid** | plugins/nsid.rs | **gap → this uplift** |
| **header** | plugins/header.rs | **gap → this uplift** |
| **local** | plugins/local.rs | **gap → this uplift** |
| **dns64** | plugins/dns64.rs | **gap → this uplift** |
| autopath | — | gap (server-side search-path; pure, future cycle) |
| metadata | — | gap (request metadata propagation; future cycle) |
| view | — | gap (CEL/expression server select; future cycle) |
| sign | dnssec/* (offline signer present) | partial (online signer) |
| bind | config (listen addresses) | folded into config |
| cancel | plugin chain (ctx cancel) | folded into chain |
| nomad | — | scope_cut (live Nomad API → cave-cloud) |
| geoip | — | scope_cut (MaxMind DB → cave-geoip) |
| grpc | — | scope_cut (gRPC upstream → cave-net) |
| dnstap | — | scope_cut (frame-stream tap → cave-observability) |
| proxyproto | — | gap (PROXY protocol v1/v2 parse; pure, future cycle) |
| multisocket | — | scope_cut (SO_REUSEPORT — OS listener detail) |
| azure / clouddns | — | scope_cut (live cloud SDK → cave-cloud) |
| k8s_external | plugins/kubernetes.rs (externalName) | folded (deprecated upstream) |
| federation | — | **n/a — removed from CoreDNS before v1.14.3** |

## Partial subsystems (pre-uplift)

| subsystem | gap | this uplift |
|-----------|-----|-------------|
| tsig-hmac-zone-transfer | response not HMAC-signed | **TSIG MAC (HMAC-SHA256, RFC 8945/4231) ported + wired** |
| caddyfile-corefile-parser | Caddyfile lexer absent (serde JSON shim only) | left partial — lexer lives in the separate `coredns/caddy` repo, not in this tree |
| doq-quic-listener | no dedicated 0-RTT QUIC bind | left partial — needs a QUIC stack (quinn) not yet a workspace dep |

## Structurally un-mappable (block a *count-based* honest 1.00)

These cannot become `mapped` without either fabricating live integrations or
adding the forbidden `adr_justified` scope-cut marks, so the crate honestly
lands **below 1.00**:

- `external-plugin-loader` — CoreDNS' cgo dynamic `.so` plugin marketplace is
  architecturally incompatible with cave-runtime's single static binary.
- `aws-route53-live-sdk`, `azure-dns-plugin`, `gcp-clouddns-plugin` — require
  live cloud SDKs; out of scope for the offline runtime.
- `etcd-live-watch` — needs a live etcd cluster.
- `federation-deprecated`, `k8s-external-deprecated` — removed/deprecated
  upstream; no source to port.
- `fuzz-msg-roundtrip` — owned by the workspace-wide cargo-fuzz job.
- `live-dnssec-key-rollover` — online KSK/ZSK rollover is an operator concern.

## This uplift (strict-TDD, line-by-line from CoreDNS v1.14.3)

7 RED→GREEN cycles, each a `test(...)` commit (failing) followed by a
`feat(...)` commit (passing): **bufsize, minimal, nsid, header, local, dns64**
plugins + **TSIG HMAC-SHA256** zone-transfer MAC. Net effect: 6 new mapped
plugin subsystems + 1 partial→mapped conversion; honest coverage rises while
the residual un-mappable surface above is preserved transparently (no
`adr_justified` inflation).
