# cave-dns — Charter v2 Parity Report

**Audit date:** 2026-05-23
**Branch:** `claude/cave-dns-close-2026-05-23`
**Upstream:** [coredns/coredns](https://github.com/coredns/coredns) `v1.14.3`
**source_sha:** `17fceec6d93fd1dde5ba6888c363f131ff6d647f`
**License:** Apache-2.0 (upstream) -> AGPL-3.0-or-later (cave-runtime)
**Companion upstream:** [miekg/dns](https://github.com/miekg/dns) `v1.1.66` (vendored by CoreDNS)

## Scorecard

| metric              | value                              |
|---------------------|------------------------------------|
| `fill_ratio`        | **0.9583** (was 0.0)               |
| `honest_ratio`      | **0.7500**                         |
| `mapped_count`      | 33                                 |
| `partial_count`     | 3                                  |
| `skipped_count`     | 10                                 |
| `unmapped_count`    | 2                                  |
| `total`             | 48                                 |
| `parity_ratio_source` | `manifest`                       |
| `last_audit`        | 2026-05-23                         |

`fill_ratio = (mapped + partial + skipped) / total = (33 + 3 + 10) / 48 = 0.9583`
`honest_ratio = (mapped + partial) / total = (33 + 3) / 48 = 0.7500`

## Charter v2 8-gate audit (G1-G8 + G9 surface)

| Gate | Description                                | Status |
|------|--------------------------------------------|--------|
| G1   | `[upstream]` block + pinned `source_sha`   | PASS   |
| G2   | `[[mapped]]` `local_files` exist on disk   | PASS   |
| G3   | `[[partial]]` carries `gap_reason`         | PASS   |
| G4   | `[[skipped]]` carries `scope_cut_target`   | PASS   |
| G5   | `[[unmapped]]` honest (1..=5, documented)  | PASS   |
| G6   | `fill_ratio` >= 0.95 + counts sum to total | PASS   |
| G7   | 100% AGPL SPDX header coverage             | PASS   |
| G8   | no `todo!()`/`unimplemented!()`/stub macros| PASS   |
| G9   | runtime surface intact (CLI + observability + dnssec + plugins) | PASS |

`cargo test -p cave-dns --lib --tests`:
- **88** lib tests PASS
- **9** parity-self-audit assertions PASS
- **0** failed / **0** ignored

## Mapped subsystems (33)

| # | name | upstream | local_files |
|---|------|----------|-------------|
| 1 | `plugin-chain-framework` | `plugin/plugin.go + chain.go` | `src/plugins/mod.rs` |
| 2 | `plugin-cache` | `plugin/cache/{cache,setup}.go` | `src/plugins/cache.rs`, `src/cache.rs` |
| 3 | `plugin-errors` | `plugin/errors/errors.go` | `src/plugins/errors.rs` |
| 4 | `plugin-file` | `plugin/file/{file,setup}.go` | `src/plugins/file.rs`, `src/zone/file.rs` |
| 5 | `plugin-forward` | `plugin/forward/{forward,setup}.go` | `src/plugins/forward.rs`, `src/forward.rs`, `src/resolver.rs` |
| 6 | `plugin-health` | `plugin/health/health.go` | `src/plugins/health.rs` |
| 7 | `plugin-hosts` | `plugin/hosts/hosts.go` | `src/plugins/hosts.rs` |
| 8 | `plugin-kubernetes` | `plugin/kubernetes/{kubernetes,handler}.go` | `src/plugins/kubernetes.rs`, `src/discovery.rs`, `src/manager.rs` |
| 9 | `plugin-log` | `plugin/log/log.go` | `src/plugins/log.rs` |
| 10 | `plugin-loop-detect` | `plugin/loop/loop.go` | `src/plugins/loop_detect.rs` |
| 11 | `plugin-metrics` | `plugin/metrics/metrics.go` | `src/plugins/metrics.rs` |
| 12 | `plugin-prometheus` | `plugin/metrics/setup.go (alias)` | `src/plugins/prometheus.rs` |
| 13 | `plugin-ready` | `plugin/ready/ready.go` | `src/plugins/ready.rs` |
| 14 | `plugin-reload` | `plugin/reload/reload.go` | `src/plugins/reload.rs` |
| 15 | `plugin-rewrite` | `plugin/rewrite/rewrite.go` | `src/plugins/rewrite.rs` |
| 16 | `plugin-root` | `plugin/root/root.go` | `src/plugins/root.rs` |
| 17 | `plugin-secondary` | `plugin/secondary/secondary.go` | `src/plugins/secondary.rs`, `src/zone/transfer.rs` |
| 18 | `plugin-template` | `plugin/template/template.go` | `src/plugins/template.rs` |
| 19 | `plugin-tls` | `plugin/tls/tls.go` | `src/plugins/tls.rs` |
| 20 | `plugin-trace` | `plugin/trace/trace.go` | `src/plugins/trace.rs` |
| 21 | `plugin-whoami` | `plugin/whoami/whoami.go` | `src/plugins/whoami.rs` |
| 22 | `plugin-acl` | `plugin/acl/acl.go` | `src/plugins/acl.rs` |
| 23 | `plugin-loadbalance` | `plugin/loadbalance/loadbalance.go` | `src/plugins/loadbalance.rs` |
| 24 | `plugin-chaos` | `plugin/chaos/chaos.go` | `src/plugins/chaos.rs` |
| 25 | `plugin-any` | `plugin/any/any.go` | `src/plugins/any.rs` |
| 26 | `plugin-auto` | `plugin/auto/auto.go` | `src/plugins/auto.rs` |
| 27 | `plugin-etcd` | `plugin/etcd/etcd.go` | `src/plugins/etcd.rs` |
| 28 | `plugin-route53` | `plugin/route53/route53.go` | `src/plugins/route53.rs` |
| 29 | `zone-file-parser` | `plugin/file/{parse,zone}.go` | `src/zone/file.rs`, `src/zone/zone.rs` |
| 30 | `zone-transfer-axfr-ixfr` | `plugin/transfer/transfer.go` | `src/zone/transfer.rs` |
| 31 | `zone-dynamic-update` | RFC 2136 | `src/zone/update.rs` |
| 32 | `server-listeners` | `core/dnsserver/server_{udp,tcp,https,tls}.go` | `src/server/{mod,udp,tcp,doh,dot}.rs` |
| 33 | `dnssec-validator` | `plugin/dnssec/* + vendor/miekg/dns/dnssec.go` | `src/protocol/dnssec.rs`, `src/dnssec/{nsec,nsec3,dnskey,rrsig,validator}.rs` |

## Partial subsystems (3) — gap_reason called out

| name | gap_reason |
|------|-----------|
| `doq-quic-listener` | QUIC 0-RTT pre-handshake binding deferred to dedicated `cave-net` QUIC slot; current code path reuses DoH HTTP/3 |
| `caddyfile-corefile-parser` | Caddyfile tokeniser deferred to dedicated `cave-dns-corefile` parser; serde JSON shim is functional for cave-runtime use |
| `tsig-hmac-zone-transfer` | HMAC signing deferred until cave-vault provides the zone-transfer keyring API |

## Skipped subsystems (10) — scope_cut targets

| name | scope_cut_target |
|------|------------------|
| `caddyfile-lexer` | `cave-dns-corefile` |
| `external-plugin-loader` | `cave-dns-plugins-marketplace` |
| `aws-route53-live-sdk` | `cave-cloud` |
| `azure-dns-plugin` | `cave-cloud` |
| `gcp-clouddns-plugin` | `cave-cloud` |
| `etcd-live-watch` | `cave-etcd` |
| `federation-deprecated` | `n/a` (removed upstream) |
| `k8s-external-deprecated` | folded into cave-dns/plugins/kubernetes |
| `file-watch-notify` | `cave-fsnotify` |
| `corefile-import-directive` | `cave-dns-corefile` |

## Unmapped (2) — honest gaps

| name | reason |
|------|--------|
| `fuzz-msg-roundtrip` | Wire-format fuzz harness — deferred to workspace cargo-fuzz job rather than per-crate Go-style harness. |
| `live-dnssec-key-rollover` | Online KSK/ZSK rollover state machine — current cave-dns ships offline pre-signed zones; the rollover scheduler is a Phase 2 work item. |

## Scope-cut groups -> Phase 2 owners

| group | crates | items |
|-------|--------|-------|
| `corefile-parser` | `cave-dns-corefile` | caddyfile-lexer, corefile-import-directive |
| `external-plugins` | `cave-dns-plugins-marketplace` | external-plugin-loader |
| `cloud-dns-sdks` | `cave-cloud` | aws-route53-live-sdk, azure-dns-plugin, gcp-clouddns-plugin |
| `etcd-watch` | `cave-etcd` | etcd-live-watch |
| `fs-watch` | `cave-fsnotify` | file-watch-notify |
| `deprecated-upstream` | `n/a` | federation-deprecated, k8s-external-deprecated |

## Observability

8 dashboard panels via `cave_dns::observability::panels()`:
1. DNS requests per second
2. Request latency p95
3. Response code distribution
4. Cache hit ratio
5. Upstream forward failures
6. Active TCP / DoT / DoH connections
7. Zone transfer (AXFR / IXFR) per minute
8. DNSSEC validation verdicts

5 Prometheus alert rules via `cave_dns::observability::alerts()`:
- `DnsHighErrorRate` (critical)
- `DnsLatencyP95High` (warning)
- `DnsCacheHitRateLow` (warning)
- `DnsForwardUpstreamDown` (critical)
- `DnssecBogusResponsesElevated` (critical)

## `cavectl dns` CLI dispatcher

Parser library at `src/cli.rs` (no clap, no binary touch):
- `cavectl dns query <name> [type]`
- `cavectl dns zone {list|show|reload} [zone]`
- `cavectl dns plugin {list|describe} [name]`
- `cavectl dns cache {stats|flush}`
- `cavectl dns reload`

## LOC delta

| component | before | after |
|-----------|-------:|------:|
| `src/` total | 7,892 | 9,823 |
| files in `src/` | 47 | 67 |
| lib tests | 19 | 88 |
| self-audit assertions | 0 | 9 |

## Commits

```
bf9f19f9 test(cave-dns): Charter v2 8-gate self-audit + runtime surface gate
f996cfc1 feat(cave-dns): observability + cavectl dns CLI dispatcher
03cd4184 feat(cave-dns): split DNSSEC primitives into focused sub-modules
19a247f4 feat(cave-dns): add prometheus/root/tls/trace built-in plugins
013ee671 feat(cave-dns): rewrite parity.manifest.toml to Charter v2 schema
```
