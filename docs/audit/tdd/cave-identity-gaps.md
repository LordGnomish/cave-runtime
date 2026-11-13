# TDD coverage audit — cave-identity vs spiffe/spire @ v1.15.0

| field | value |
|---|---|
| crate | cave-identity (theme: security) |
| upstream | https://github.com/spiffe/spire @ v1.15.0 |
| upstream test files | 406 |
| upstream test symbols (functions) | 1180 |
| cave `#[test]`/`#[tokio::test]` fns | 119 (105 unit + 4 proptest + 9 self-audit + 1 routes-pair) |
| audit date | 2026-05-30 |
| audit scope | behaviors NOT yet covered; report only genuine uncovered portable units |

## Summary

cave-identity already has **substantial, honest coverage** of the portable SPIFFE/SPIRE
surface: SPIFFE-ID parsing/validation (13 tests), X.509-SVID issue/verify/rotate (7),
JWT-SVID issue/verify (7), bundle JWKS marshal/unmarshal (3), federation (7),
admission policy (8), registration CRUD + selector set-algebra (9), server-CA bootstrap
+ rotation (7), events (15), k8s PSAT attestor (6), attestor engine (5), OIDC (4),
store CRUD (4). The vast majority of upstream test symbols (1180) are **scope-cut**:
CLI command help/synopsis/run harnesses (`spire-agent`/`spire-server` cobra commands),
gRPC service plumbing, datastore SQL drivers, plugin SDK wiring, agent-side LRU SVID
cache subscriber notifications, and config/flag parsing — none of which cave-identity
implements (it is a library facade, not the agent/server daemon).

After reading every `src/*.rs` module and matching each public fn against existing
`#[cfg(test)] mod tests`, **7 genuine uncovered portable behaviors** remain — all are
error/edge branches of functions cave already implements, each with a direct upstream
behavioral analog.

## Classification

| behavior | upstream analog | cave fn | status |
|---|---|---|---|
| SPIFFE-ID parse/validate (charset, len, dot-segments, trailing slash, td-only, descendant) | `idutil`, `spiffeid` pkg tests | `parse_spiffe_id`, `validate_trust_domain`, `is_descendant`, `agent_id` | **covered** |
| X.509-SVID issue + chain verify + rotate-at-half-life + fingerprint | `x509svid`, `manager` rotation tests | `x509_svid::issue/verify/should_rotate/rotate_if_needed/fingerprint` | **covered** |
| JWT-SVID issue + audience match + tainted-kid + tamper reject | `jwtsvid`, `TestValidateJWTSVID` | `jwt_svid::issue/verify/unsafe_decode_sub` | **covered** |
| Bundle JWKS marshal/unmarshal + taint marker + unknown-use reject | `bundleutil/marshal` | `bundle::marshal/unmarshal/to_json/from_json` | **covered** |
| Federation create/refresh/td-mismatch/non-https reject | `bundle/client`, federation tests | `FederationManager::create/refresh/verify_bundle` | **covered** |
| Admission TTL clamp + admin/downstream gating + foreign-td reject | `entry/v1/service` CreateEntries | `policy::admit_entry` | **covered** |
| Selector MATCH_SUBSET/EXACT/SUPERSET set algebra | `selector` set tests | `registration::selectors_match/_equal/_superset` | **covered** |
| Server-CA bootstrap + intermediate/jwt rotation + taint-root flag | `pkg/server/ca` rotation tests | `ServerCa::bootstrap/rotate_*/taint_root` | **covered** |
| **JWT-SVID rejects EXPIRED token** (`claims.exp < now`) | `jwtsvid` expiry / `TestIsSVIDExpired` | `jwt_svid::verify` | **portable-coverage (uncovered)** |
| **JWT-SVID rejects UNKNOWN kid** (kid not in bundle authorities) | `TestValidateJWTSVID` unknown-key | `jwt_svid::verify` | **portable-coverage (uncovered)** |
| **X.509-SVID verify rejects SVID with no intermediates** | `x509svid.Verify` empty-chain | `x509_svid::verify` | **portable-coverage (uncovered)** |
| **X.509-SVID rotate_if_needed returns None when not yet due** | `manager` no-rotation path | `x509_svid::rotate_if_needed` | **portable-coverage (uncovered)** |
| **Policy rejects PARENT_ID foreign trust-domain** (distinct from spiffe_id) | `entry/v1` parent validation | `policy::admit_entry` | **portable-coverage (uncovered)** |
| **Server-CA taint propagates into trust_bundle() x509 authority** | `TestTaintX509SVIDs` | `ServerCa::taint_root` → `ServerCa::trust_bundle` | **portable-coverage (uncovered)** |
| **Bundle unmarshal rejects x509 entry missing x5c** | `bundleutil` malformed-jwk | `bundle::unmarshal` | **portable-coverage (uncovered)** |
| CLI command help/synopsis/run (spire-agent, spire-server cobra) | `cmd/spire-*/cli/**` (hundreds) | — | scope-cut (no CLI in crate) |
| gRPC services, datastore SQL drivers, plugin SDK | `pkg/server/**`, `proto/**` | — | scope-cut (daemon/infra plumbing) |
| Agent LRU SVID cache subscriber notification matrix | `pkg/agent/manager/cache` (TestLRUCache*) | — | scope-cut (agent runtime not ported) |

## Recommended TDD fills (portable-coverage first)

Each test below is a RED→GREEN unit test against an **existing public cave fn** whose
error/edge branch is currently unexercised. No source changes required — these assert
already-implemented behavior.

1. **`jwt_svid::verify` — expired token** (src/jwt_svid.rs:102)
   Issue a JWT-SVID, mutate the encoded `exp` (or issue with a near-zero TTL and advance),
   then `verify(&token, aud, &bundle)` must return `Err(JwtInvalid("expired"))`.
   Exercises the `claims.exp < now` branch that no current test reaches.

2. **`jwt_svid::verify` — unknown kid** (src/jwt_svid.rs:111-117)
   Issue a token, then verify against a bundle whose `jwt_authorities` do NOT contain the
   token's `kid` (e.g. an empty-jwt bundle or one rotated to a different key id) — must
   return `Err(JwtInvalid("unknown kid: ..."))`. Distinct from the tainted-kid test.

3. **`x509_svid::verify` — no intermediates** (src/x509_svid.rs:61-65)
   Build an `X509Svid` (or clone an issued one) with `intermediates_der` emptied, verify
   against a non-empty bundle — must return `Err(SvidVerificationFailed("no intermediates"))`.
   The empty-chain branch is never hit (existing tests only empty the bundle).

4. **`x509_svid::rotate_if_needed` — returns None when fresh** (src/x509_svid.rs:101-111)
   Issue a full-TTL SVID (remaining > half-life) and call `rotate_if_needed` — must return
   `Ok(None)`. Every existing rotate test forces the `Some(..)` path; the no-op branch is
   uncovered.

5. **`policy::admit_entry` — parent_id foreign trust-domain** (src/policy.rs:61-68)
   Submit an entry whose `spiffe_id` is in-domain but whose `parent_id` is
   `spiffe://other.org/agent` — must return `Err(PolicyViolation("parent_id trust-domain
   mismatch ..."))`. All current td-mismatch tests mutate spiffe_id, never parent_id.

6. **`server_ca::taint_root` → `server_ca::trust_bundle` propagation** (src/server_ca.rs:232 / 262-273)
   Bootstrap a CA, call `taint_root()`, then assert `trust_bundle().x509_authorities[0].tainted == true`.
   Existing `taint_root_flips_flag` checks only the internal flag, never the published-bundle
   surface that relying parties consume (the SPIRE `TaintX509` contract).

7. **`bundle::unmarshal` — x509 entry missing x5c** (src/bundle.rs:97-100)
   Feed a `BundleDoc` with a `key_use == "x509-svid"` entry whose `x5c` is `None` — must
   return `Err(Internal("x509-svid missing x5c"))`. Current `unmarshal_rejects_unknown_use`
   only covers the unknown-`use` branch, not the malformed-x509 branch (nor the symmetric
   jwt-svid missing-`x` branch, src/bundle.rs:113-116 — an optional 8th fill).

### Notes / honest exclusions

- `oidc::jwks_for_bundle` tainted-filter, `events` rebind cross-td validation, k8s PSAT
  audience checks, federation non-https/bad-spiffe rejection, and all selector set-algebra
  modes are **already covered** — not listed.
- The overwhelming majority of the 1180 upstream symbols are CLI/daemon/gRPC/datastore/
  agent-cache tests with no library analog in cave-identity and are correctly scope-cut.
- All 7 fills assert existing behavior; none reveal a missing implementation. Crate is
  honestly near-complete on the portable surface.
