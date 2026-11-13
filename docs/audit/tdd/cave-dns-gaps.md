# TDD coverage audit — cave-dns

- **Crate:** `crates/networking/cave-dns` (theme: networking)
- **Upstream:** https://github.com/coredns/coredns @ `v1.14.3`
- **Upstream test symbols:** 801 `func Test*` across 336 `_test.go` files
- **Cave test functions:** 110 `#[test]` / `#[tokio::test]`
- **Date:** 2026-05-30

## Summary

cave-dns already has substantial, honest coverage of its strongest subsystems:
DNSSEC (DNSKEY/RRSIG/NSEC/NSEC3/validator), zone file parse + AXFR + dynamic
update, cache put/get/expiry, TLS readiness, trace sampling, CLI argv parsing,
observability aggregation, GitOps drift/sync, and Charter-v2 self-audit gates.
Those areas are **not** re-listed below.

The genuine uncovered behavior is concentrated in the **DNS protocol helper
layer** (`protocol/message.rs`, `protocol/records.rs`, `protocol/edns.rs`) — a
set of small, deterministic, fully-implemented public functions that map
directly to upstream CoreDNS unit tests but currently have **zero** Rust tests.
These are the priority portable-coverage fills.

The large remainder of upstream's 801 tests are **scope-cut**: they exercise Go
plugin `ServeDNS` HTTP/gRPC/QUIC server plumbing, Kubernetes/etcd/clouddns
controllers, Caddyfile-directive setup parsing, and vendor transport (DoH/DoQ/
HTTP3) — none of which is a portable behavioral unit for cave's hickory-based
core.

## Classification table

| Upstream behavior (representative test) | Cave public fn | Status |
|---|---|---|
| `TestErraticTruncate`, `TestAWithExternalCNAMELookupTruncated` (UDP truncation + TC bit) | `protocol::message::truncate_to_udp` | **portable-coverage** (impl present, no test) |
| EDNS0 advertised payload extraction (`plugin/pkg/edns`, `request`) | `protocol::message::edns_payload_size` | **portable-coverage** |
| DO-bit / DNSSEC-OK detection | `protocol::message::dnssec_ok` | **portable-coverage** |
| Error-response RCODE skeleton (SERVFAIL/NXDOMAIN/REFUSED) | `protocol::message::make_error_response` | **portable-coverage** |
| Probe/health query construction (`make_query`) | `protocol::message::make_query` | **portable-coverage** |
| AAAA record build | `protocol::message::aaaa_record` | **portable-coverage** |
| TXT record build (`TestTXT*`) | `protocol::message::txt_record` | **portable-coverage** |
| `TestFilterRRSlice` (type filter, CNAME passthrough) | `protocol::records::filter_by_type` | **portable-coverage** |
| `TestNormalize` (name → FQDN normalization) | `protocol::records::parse_fqdn` | **portable-coverage** |
| Generic RData record build | `protocol::records::build_record` | **portable-coverage** |
| EDNS options struct extraction (`request.Req.Size`, DO bit) | `protocol::edns::EdnsOptions::from_message` | **portable-coverage** |
| EDNS min-512 buffer clamp | `protocol::edns::EdnsOptions::effective_udp_size` | **portable-coverage** |
| `TestRoundRobinEmpty` + round-robin rotation (`plugin/loadbalance`) | `LoadbalancePlugin::handle` (via `new`; `rotate` private) | **portable-coverage** (behavior reachable only through `handle`) |
| `TestHostsParse`, `TestHostsInlineParse` (`/etc/hosts` parse + inline) | `HostsPlugin` (`parse_content`/`build_records` private) | **portable-coverage** (private; needs `handle`/ready harness or visibility) |
| `encode`/`decode` wire round-trip (`message.rs`) | `message::encode`, `message::decode` | **portable-coverage** (no test in `message.rs`; only `protocol::message` tested) |
| Recursive resolver dispatch | `resolver::Resolver::resolve` | portable-coverage (heavier harness; lower priority) |
| Plugin `ServeDNS` over HTTP/gRPC/QUIC, DoHWriter, server_https3 | — | scope-cut (vendor transport / server plumbing) |
| kubernetes / etcd / clouddns / route53 controller `ServeDNS` | — | scope-cut (controller/CRD/cloud vendor) |
| Caddyfile directive `setup`/`parse` per plugin | — | scope-cut (config-syntax, replaced by cave `config.rs`) |
| metrics/dnstap/trace/log/pprof/health exporter wiring | — | scope-cut (covered by cave observability tests or infra) |

## Recommended TDD fills (portable-coverage first)

Priority 1 — pure, deterministic, public, zero-harness (`protocol/message.rs`,
`protocol/records.rs`, `protocol/edns.rs`):

1. `protocol::message::truncate_to_udp` — build a `Message` with answers whose
   encoded length exceeds a small `max_bytes`; assert TC bit set and answers
   cleared; and that an under-budget message is left untouched with TC clear.
2. `protocol::message::edns_payload_size` — assert 512 default when no OPT, and
   the advertised value when an EDNS extension is attached.
3. `protocol::message::dnssec_ok` — assert `false` without OPT, `true` when the
   DO bit is set.
4. `protocol::message::make_error_response` — assert it mirrors id/queries from
   `make_response` and carries the requested `ResponseCode` (e.g. ServFail,
   NXDomain).
5. `protocol::message::make_query` — assert MessageType::Query, OpCode::Query,
   recursion-desired, single query with the given name/type, IN class.
6. `protocol::message::aaaa_record` / `txt_record` — assert record type, ttl,
   class, and decoded RData round-trip; error on malformed name.
7. `protocol::records::filter_by_type` — assert qtype matches are kept, CNAME is
   always passed through, and unrelated types dropped.
8. `protocol::records::parse_fqdn` — assert `"example.com"` and
   `"example.com."` both yield the same trailing-dot `Name`; error on invalid.
9. `protocol::records::build_record` — assert record_type is derived from the
   RData (not set explicitly) and ttl/class round-trip.
10. `protocol::edns::EdnsOptions::from_message` — assert defaults with no
    extension; populated `udp_payload_size` + `dnssec_ok` with one.
11. `protocol::edns::EdnsOptions::effective_udp_size` — assert it clamps any
    sub-512 advertised size up to 512 and passes larger sizes through.
12. `message::encode` / `message::decode` — round-trip a `DnsMessage` and assert
    id/queries/answers survive (the existing round-trip test lives in
    `protocol::message`, not in `message.rs`, so the wire codec is untested).

Priority 2 — behavior reachable only via the `Plugin` trait (needs a small
`QueryContext` + terminal `Next` harness, or relaxing `rotate`/`parse_content`/
`build_records` to `pub(crate)`):

13. `LoadbalancePlugin::handle` — round-robin: a 3-answer A set is rotated by one
    position per successive query for the same name; a 0/1-answer set and the
    non-A "fixed" partition are left in place (covers `TestRoundRobinEmpty`).
14. `HostsPlugin` parse — a hosts blob plus inline entries resolves A/AAAA
    forward records and skips comment/blank lines (covers `TestHostsParse` /
    `TestHostsInlineParse`).

All Priority-1 functions are confirmed real implementations (not stub macros)
and require no I/O, making them ideal RED→GREEN targets.
