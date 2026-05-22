# cave-desktop

Native admin shell for CAVE Runtime, built with [GPUI](https://github.com/zed-industries/zed/tree/main/crates/gpui).

The web portal at `crates/cave-runtime/src/portal_index.html` remains the primary, canonical UI. This crate is a power-admin companion that runs as a native binary, talking to the same `cave-runtime` HTTP backend.

See `docs/adr/ADR-PORTAL-DESKTOP-001-gpui-native-admin-shell.md` for the design rationale.

## Status

**Scaffold.** ADR is accepted, crate compiles in the workspace, three UI primitives (`Panel`, `MetricCard`, `Table`) and one placeholder screen (`cluster_overview`) exist. No real GPUI rendering yet — the bring-up sequence (`App::new().run()`) is gated behind the `gpui-runtime` feature and currently exits with a TODO message until we pin a Zed `rev` that builds clean here.

## Build

```bash
# Default — fast, no GPUI, runs the headless TODO entry:
cargo run -p cave-desktop

# With GPUI — clones zed-industries/zed (~hundreds of MB) and pulls Metal/Vulkan deps:
cargo run -p cave-desktop --features gpui-runtime
```

The default-off feature flag exists so workspace-wide builds (`cargo check --workspace`) stay fast and don't depend on Zed's repo being reachable.

## Layout

```
src/
├── main.rs              # entry point (gated on `gpui-runtime`)
├── ui/
│   ├── panel.rs         # titled container primitive
│   ├── metric_card.rs   # big-number tile primitive
│   └── table.rs         # row-oriented table primitive
└── screens/
    └── cluster_overview.rs   # first screen, placeholder data
```
