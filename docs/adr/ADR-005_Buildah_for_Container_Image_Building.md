# ADR-005: Buildah for Container Image Building

**Status:** Accepted

**Scope:** Universal

**Category:** Infrastructure — CI/CD

**Related ADRs:** 023 (ARC), 028 (Harbor), 032 (cosign), 077 (Sigstore), 101 (SLSA L3), 108 (Supply Chain)

## Context

CAVE's 27-stage CI/CD pipeline (stages 12-13) needs a container image build tool. Requirements:

- Rootless execution (CI runners must not require privileged containers)
- Daemonless (no Docker daemon dependency in CI environment)
- Hermetic builds (no network access during build — SLSA Level 3 requirement, ADR-101)
- OCI-compliant image output (push to Harbor, ADR-028)
- Dockerfile compatibility (standard Dockerfile syntax for developer familiarity)
- Multi-arch build capability (amd64 + arm64 for future edge scenarios)
- Compatible with ARC runners on K8s (ADR-023)

---

## Candidates

## ### 3.1 Container Build Tool Comparison

| Criteria | Buildah | Kaniko | Docker (BuildKit) | Podman Build | Jib (Java) | ko (Go) |
|---|---|---|---|---|---|---|
| **Rootless** | ✅ Native rootless mode. No root privileges needed. | ✅ Runs in user namespace (no Docker daemon). | ⚠️ BuildKit rootless available but less mature. Docker daemon traditionally requires root. | ✅ Native rootless (uses Buildah under the hood). | ✅ No container runtime needed. | ✅ No container runtime needed. |
| **Daemonless** | ✅ No daemon. CLI-only. | ✅ No daemon. Runs as container. | ❌ Requires Docker daemon (dockerd) or BuildKit daemon (buildkitd). | ✅ No daemon (uses Buildah). | ✅ Maven/Gradle plugin. | ✅ CLI tool. |
| **Hermetic build** | ✅ `--no-network` flag disables network during build. Enforces SLSA L3 hermetic requirement. | ⚠️ Network isolation requires K8s NetworkPolicy around Kaniko pod. Not native. | ⚠️ `--network=none` in Dockerfile, but daemon still has network access. | ✅ Same as Buildah. | ⚠️ JVM has network access during build. | ⚠️ Go module download requires network (pre-vendor required). |
| **Dockerfile support** | ✅ Full Dockerfile syntax. | ✅ Full Dockerfile syntax. | ✅ Full Dockerfile syntax (native). | ✅ Full Dockerfile (via Buildah). | ❌ No Dockerfile. Java-specific. | ❌ No Dockerfile. Go-specific. |
| **OCI output** | ✅ Native OCI image format. | ✅ OCI output. | ✅ OCI output. | ✅ OCI (via Buildah). | ✅ OCI output. | ✅ OCI output. |
| **Multi-arch** | ✅ `buildah manifest` for multi-arch. | ⚠️ Limited. Requires separate Kaniko runs + manifest list. | ✅ `docker buildx` (native multi-arch). | ✅ Same as Buildah. | ✅ Multi-platform support. | ✅ Multi-platform. |
| **K8s CI runner compatible** | ✅ Runs as unprivileged container in ARC runner pod. | ✅ Designed for K8s (runs as pod). | ❌ Requires Docker-in-Docker (DinD) or Docker socket mount — security risk in multi-tenant CI. | ✅ Same as Buildah. | ✅ JVM in runner pod. | ✅ Go binary in runner pod. |
| **Build cache** | ✅ Layer caching via `--layers`. Registry-based cache via `--cache-from`. | ⚠️ Limited caching. Pulls layers from registry. Slower than local cache. | ✅ Excellent caching (BuildKit). | ✅ Same as Buildah. | ✅ Layer caching. | ✅ Layer caching. |
| **Language agnostic** | ✅ Any Dockerfile | ✅ Any Dockerfile | ✅ Any Dockerfile | ✅ Any Dockerfile | ❌ Java only | ❌ Go only |
| **License** | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 | Apache 2.0 |
| **Signing integration** | ✅ Output → cosign sign (ADR-032). Standard OCI workflow. | ✅ Same workflow. | ✅ Same workflow. | ✅ Same workflow. | ✅ Same workflow. | ✅ Same workflow. |

### 3.2 Security Posture in CI

| Attack Vector | Buildah | Kaniko | Docker (DinD) |
|---|---|---|---|
| Container escape via privileged mode | ❌ Rootless. No privilege escalation path. | ❌ No daemon, no privilege. | ✅ DinD requires `--privileged` flag — container escape possible. |
| Docker socket mount attack | ❌ No socket. | ❌ No socket. | ✅ Socket mount exposes host Docker daemon. Any container in CI can build/run arbitrary images on host. |
| Build-time network exfiltration | ❌ `--no-network` enforced. Build cannot phone home. | ⚠️ Requires NetworkPolicy enforcement by K8s. | ⚠️ Daemon has network access by default. |
| Supply chain (base image tampering) | Harbor pull-through cache + digest pinning (ADR-108). Same for all tools. | Same. | Same. |
| Multi-tenant CI isolation | ✅ Each ARC runner pod runs Buildah in unprivileged namespace. Pod destroyed after use. | ✅ Each Kaniko run is a separate pod. | ❌ Shared Docker daemon across runners = cross-build contamination risk. |

### 3.3 Build Performance (Spring Boot app, ~200MB image)

| Tool | Cold Build | Warm Build (cached layers) | Registry Push |
|---|---|---|---|
| Buildah (rootless) | ~90s | ~25s | ~15s |
| Kaniko | ~120s | ~60s (registry-based cache slower) | ~15s |
| Docker BuildKit | ~70s | ~15s (local cache fastest) | ~15s |
| Jib (Java only) | ~45s | ~10s (layer optimization) | ~10s |

Buildah is ~30% slower than Docker BuildKit on cold builds due to rootless overhead, but the security posture trade-off is justified. Warm builds are comparable. Kaniko is slowest due to registry-based caching.

---

## Decision

**Buildah** (rootless, daemonless) for container image building in all CI pipelines.

---

## Rejected

## ### 4.1 Kaniko — Rejected

**Primary:** No native hermetic build mode. Kaniko runs as a container and can access the network during build unless constrained by external K8s NetworkPolicy. SLSA Level 3 (ADR-101) requires hermetic builds — Buildah's `--no-network` flag provides this natively at the tool level. Relying on NetworkPolicy for hermeticity is an infrastructure-level control, not a build-level control — easier to misconfigure or bypass.

**Secondary:** Cache performance. Kaniko's registry-based caching is significantly slower than Buildah's local layer caching. For CAVE's 27-stage pipeline with <15min SLO target, every second matters. Cold builds take ~30% longer on Kaniko vs Buildah.

### 4.2 Docker (BuildKit / DinD) — Rejected

**Primary:** Docker daemon requirement in CI. Running Docker-in-Docker (DinD) requires `--privileged` flag on the CI runner container — this grants the build process full host kernel access, enabling container escape. In a multi-tenant CI environment (ARC runners building for multiple tenants), this is a critical security risk. Docker socket mounting is equally dangerous — any process in the runner can execute arbitrary containers on the host.

**Secondary:** BuildKit daemon (buildkitd) can run rootless but is not daemonless. Requires a long-running process in the CI environment, which contradicts CAVE's ephemeral runner model (ARC creates runner pod → build runs → pod destroyed). Buildah's CLI-only model fits ephemeral runners perfectly.

### 4.3 Podman Build — Viable but redundant

Podman's build command uses Buildah under the hood. Using Podman adds an unnecessary abstraction layer. CAVE uses Buildah directly for clarity and minimal dependency. Podman would be equivalent in capability but adds Podman as an additional package to manage.

### 4.4 Jib / ko — Rejected as primary

**Primary:** Language-specific. Jib supports only Java (Spring Boot, Quarkus). ko supports only Go. CAVE supports 5 languages (ADR-032): Java, Python, Go, TypeScript, Rust. A single build tool for all languages is operationally simpler. Buildah handles all Dockerfiles regardless of language.

**Secondary:** Jib and ko produce optimized images for their respective languages but bypass Hadolint (stage 11) and container-level security scanning patterns. Buildah's Dockerfile-based workflow integrates naturally with the full 27-stage pipeline.

---

## CI Pipeline Integration

```
Stage 11: Hadolint lints Dockerfile
Stage 12: Buildah builds image (rootless, --no-network, hermetic)
          → Pulp proxy for all build dependencies (ADR-044)
          → Base images from Harbor pull-through cache (ADR-028)
          → Digest-pinned base images (ADR-108)
Stage 13: Buildah pushes to Harbor
Stage 14: cosign signs image (keyless, OIDC from ARC runner SA)
          → SLSA provenance attestation generated
          → Provenance uploaded to Harbor OCI + Sovereign Ledger
```

---

## Consequences

## ### Positive

- Rootless + daemonless = zero privilege escalation in CI
- Native `--no-network` enforces SLSA L3 hermetic builds at tool level
- Language-agnostic (single tool for all 5 supported languages)
- CLI-only model fits ephemeral ARC runner pods perfectly
- Standard Dockerfile syntax — zero developer learning curve
- Apache 2.0 license, Red Hat maintained, large community

### Negative

- ~30% slower cold builds vs Docker BuildKit (rootless overhead)
- Multi-arch builds less streamlined than `docker buildx`
- Smaller ecosystem than Docker (fewer blog posts, tutorials)
- Rootless mode requires user namespace support in kernel (Talos provides this)

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Buildah regression breaks CI | Low | High | Buildah version pinned + digest (ADR-108). Staging validates before prod. |
| Rootless performance degrades | Low | Medium | Monitor CI pipeline duration (DORA metrics, ADR-042). Fallback: BuildKit rootless if needed. |
| Red Hat deprioritizes Buildah | Very Low | Medium | Apache 2.0 license. Podman ecosystem depends on Buildah — discontinuation unlikely. |

## Compliance Mapping

SOC2 CC8.1 (build integrity — hermetic, reproducible builds). SLSA Level 3 (hermetic build — no network access, all dependencies pre-cached). ISO A.8.25 (secure development — controlled build environment). NIS2 Art.21 (supply chain security — build isolation prevents dependency injection attacks).
