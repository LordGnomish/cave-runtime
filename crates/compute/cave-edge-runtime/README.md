# cave-edge-runtime

Pure-Rust reimplementation of the edge-orchestration control logic from
[K3s](https://github.com/k3s-io/k3s) (edge mode) and
[KubeEdge](https://github.com/kubeedge/kubeedge).

Ports the decision logic for the seven edge subsystems:

1. **edged** — minimal kubelet: pod-worker queue, pod phase, orphan GC, status cadence.
2. **metamanager** — offline-first local metadata store (cache-through + serve-from-cache).
3. **eventbus** — MQTT-topic ↔ internal message bridge over a cave-streams local queue.
4. **edgehub** — reliable cloud-edge sync keeper (message IDs + ACK + retransmit + RV merge).
5. **devicetwin** — Expected/Actual twin state, version gating, delta computation.
6. **autonomy** — online/offline connection state machine + reconcile-on-reconnect.
7. **constrained** — 256 MB resource budget: admission + memory-pressure eviction ranking.

Live transports (WebSocket/QUIC, the MQTT broker, containerd CRI, real SQLite)
are out of scope; see `parity.manifest.toml`.

Upstreams are Apache-2.0; see `NOTICE`. This crate is `AGPL-3.0-or-later`.
