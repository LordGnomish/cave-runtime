# cave-loki

Cave-side reimplementation of [grafana/loki](https://github.com/grafana/loki),
line-by-line in Rust. No UI port (cave-portal handles), no CLI port (cavectl
handles), no backwards-compat shims. Tracked at high-priority cadence
(15 minutes) by `cave-upstream-watchd`. Port work is driven by Qwen via the
pump queue under `~/Library/Application Support/cave-qwen-pump/queue/`.

See `parity.manifest.toml` for live module status.
