//! cave-loki — line-by-line Rust reimplementation of grafana/loki.
//!
//! This crate is the Cave-side port target for the Loki log-aggregation
//! system. The intent is full behavioural parity with upstream Go modules:
//! distributor, ingester, querier, compactor, store, ruler, plus the
//! schema and chunkenc support layers.
//!
//! ## Out of scope
//!
//! - **UI**: Loki has no first-party UI (Grafana renders LogQL); cave-portal
//!   handles any portal-side surfaces.
//! - **CLI**: The upstream `loki` and `logcli` binaries are not ported here.
//!   `cavectl logs ...` lives in cave-cli.
//! - **Backwards compatibility**: We track the most recent stable upstream
//!   release. Legacy v1/boltdb-shipper paths are skipped (`status="skipped
//!   (backcompat)"` in `parity.manifest.toml`).
//!
//! ## Status
//!
//! Empty placeholder — port work is driven by Qwen via
//! `~/Library/Application Support/cave-qwen-pump/queue/loki-*.json`
//! payloads. See `parity.manifest.toml` for the live mapping table.

pub mod placeholder;
