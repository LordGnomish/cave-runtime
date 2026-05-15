# Sweep-008 — `cave_kernel::identity::SpiffeId` adoption (cave-net)

**Author:** Sweep-008 close-out (2026-05-12)
**Branch:** `claude/gracious-banach-9be8eb`
**Owner:** runtime
**Honest budget consumed:** ~40 min recon + 20 min implementation.
**Status:** Landed for cave-net only. cave-mesh adoption deferred (see §4).

## 1. Premise

The kernel ships a SPIFFE 1.0 parser (`cave_kernel::identity::SpiffeId`)
with proper grammar enforcement: scheme check, trust-domain charset,
path segment validation, percent-encoding rejection. Two crates have
their own SPIFFE ID handling:

- `cave-mesh` — its own `SpiffeId` struct in `src/models.rs`, wired
  into `CertBundle`, `Svid`, and the ambient/ztunnel stack.
- `cave-net` — uses bare `String` for SPIFFE IDs in `Svid` (in
  `src/cilium/auth.rs`), with a hand-rolled `validate_trust_domain`
  that does `starts_with("spiffe://td/")`.

## 2. Recon

| Site | Shape | Verdict |
|------|-------|---------|
| `cave-net::cilium::auth::Svid` | `spiffe_id: String` + `starts_with`-based validation | **Adopt** — keep field as String (wire-stable serde), swap validator internals to kernel parser |
| `cave-mesh::models::SpiffeId` | `pub struct { trust_domain, path }` + lossy `parse() -> Option<Self>`; wired into `CertBundle` / `Svid` / ambient `*.rs` (12 files) | **Defer** — touching the public type would cascade through the mesh public API and ambient module; not a "narrow sweep" |

cave-net was already a `cave_kernel::ns::TenantId` adopter (sweep-002),
so it carries an existing `cave-kernel = { path = ... }` dependency —
no Cargo.toml change needed.

## 3. Change

`crates/cave-net/src/cilium/auth.rs`:

- `use cave_kernel::identity::SpiffeId as KernelSpiffeId;`
  + `use std::str::FromStr;`
- `Svid::validate_trust_domain` now parses the stored string with the
  kernel SPIFFE parser. The trust-domain match goes through the
  kernel's `is_member_of()` (charset-aware) rather than a literal
  string `starts_with`. A bare `spiffe://td` (no workload path) is
  rejected explicitly to preserve the legacy "workload path required"
  invariant that was implicit in the old `format!("spiffe://{}/", td)`
  prefix.

New test
`svid_validate_trust_domain_rejects_malformed_id_via_kernel_parser`
demonstrates two cases the legacy validator accepted but the kernel
parser rejects:

- `not-spiffe://cluster.local/workload` — missing scheme
- `spiffe://cluster.local/work%2fload` — percent-encoded path

Both now fail validation. The on-disk `cilium-parity-e2e` test suite
passes unchanged (42/42 in `cilium::auth::tests`).

## 4. What we did not touch and why

cave-mesh's `SpiffeId` is publicly exported and embedded in serialized
types (`CertBundle`, `Svid`) that live in xDS / ambient mesh wire paths.
Replacing it with the kernel struct would:

- Force every `pub spiffe_id: String` path-format users (12 files) to
  migrate to the new opaque struct, and
- Cascade through `Display` / `FromStr` semantics where mesh's lossy
  `parse() -> Option<Self>` is different from the kernel's
  `FromStr -> Result<Self, SpiffeError>`.

That's a sweep-sized refactor in its own right, not a narrow adoption.
Recorded as follow-up: **Sweep-008b — cave-mesh SpiffeId migration**,
to be scoped separately with explicit serde-back-compat plan.

## 5. Adoption delta

| Primitive | Crates importing before | Crates importing after |
|-----------|------------------------:|-----------------------:|
| `cave_kernel::identity::SpiffeId` | 0 | 1 (`cave-net`) |

## 6. Test surface

`cargo test -p cave-net --lib cilium::auth::` — 43 passed (42 pre-
existing + 1 new), 0 failed.
