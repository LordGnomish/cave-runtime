# cave-cri parity — 2026-05-12 measured audit

**Upstream pin:** `containerd/containerd` v2.2.x (CRI bearing packages) + `opencontainers/runc` v1.4.x (OCI bundle + cgroup writer secondary).

## Why

2026-05-01 audit: **tier 100, parity_ratio = 1.0** via wave3 mechanical metric (31 files / 83 fns / 87 tests / 35 surfaces). That hid the fact that cave-cri does NOT implement containerd's content-addressable store, diff service, or lease tracker — three load-bearing pieces of the upstream's image-storage subsystem. This pass replaces the self-report with a measured ratio over enumerated `pkg/cri/*` + `core/*` packages.

## Counts

| Bucket | Count |
|---|---:|
| `[[mapped]]` | 17 |
| `[[skipped]]` | 11 |
| `[[unmapped]]` | 6 |
| **Total** | **34** |
| **fill_ratio** | **0.8235** |

`parity_ratio = 1.0` → `fill_ratio = 0.8235`.

## Mapped (17)

CRI server: server core, podsandbox, container, images, streaming (exec/attach/portforward), stats, runtime handler. Storage layer: snapshotter (overlayfs assembly), store. Image registry: Docker transport + Bearer auth. Runtime: shim v2 collapsed into direct OCI bundle exec, OCI spec generation, cgroup v1 + v2 writers. Logs (rotation + multi-stream), health, user namespaces.

## Unmapped (6)

1. **`core/content/`** — content-addressable store with GC. cave-cri reads images directly from registry into rootfs.rs without a persistent CAS layer. Image GC reduces to image_gc.rs in cave-kubelet, which only knows about images-by-tag.
2. **`core/diff/`** — layer diff service. cave-cri shells out to tar at rootfs assembly; the diff-production path (commit container changes back to a new layer) is not implemented.
3. **`core/leases/`** — resource lease tracker to prevent GC of in-use content. Tightly coupled to `core/content/` — same root cause.
4. **`pkg/cri/server/podsandbox/sandbox_run_other.go`** — Windows / FreeBSD sandbox runners. cave-cri is Linux-only by design but the Charter does not explicitly limit platform support, so this is recorded as an honest gap rather than a skip.
5. **`pkg/oom/`** — OOM event watcher (feeds kubelet's eviction.rs). cave-cri exits containers with OOMKilled status but does not surface the kernel `oom_score_adj` events for cluster-level scoring.
6. **`core/introspection/`** — containerd's `/introspection` API listing installed plugins + versions. cave-cri serves `/healthz` but operator tooling that depends on `/introspection` would need a cave-runtime-wide equivalent.

## Skipped (11)

`cmd/containerd/` go-bootstrap. `cmd/ctr/` CLI (cave-cli). `cmd/containerd-shim-runc-v2/` wire-format-detail (cave runs runc directly without a per-container shim binary). `api/` wire-format-detail (gRPC protobuf vs cave JSON). `core/transfer/` parallel-track (folded into registry.rs). `core/sandbox/` wire-format-detail (folded into sandbox.rs). `pkg/events/` stdlib-analog (tracing). `pkg/process/` stdlib-analog. `core/metrics/` parallel-track. `pkg/plugin/` stdlib-analog (cave wires plugins statically in main.rs). `vendor/` stdlib-analog.

## Implications of the 3 storage gaps

`core/content/` + `core/diff/` + `core/leases/` together are containerd's image-storage spine. Without them:
- Image GC is best-effort (cave-kubelet's image_gc.rs operates on top-level image refs, not the underlying layer DAG).
- Layer dedup is implicit (overlayfs union FS handles it at filesystem level) but cave can't enumerate which images share which layers.
- Producing new images from running containers (`commit`) is not supported.

Porting the trio is ~3-5 K LOC of work. Documented here as the largest single behavioural gap; not landed in this pass.

## Out of scope

The 6 unmapped items are individually well-scoped sweeps. The CAS-trio is the priority pick-up; OOM-watcher is a small (~200 LOC) follow-up.
