//! TSDB Status tab — head series, chunks, symbol table, WAL size.
//!
//! Mirrors Prometheus `/tsdb-status`. Numbers are derived from the
//! seeded series set so the page is meaningful without a live TSDB.

use super::PrometheusViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::escape;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TsdbStatusRow {
    pub head_series: u64,
    pub head_chunks: u64,
    pub head_samples: u64,
    pub symbol_table_size_bytes: u64,
    pub wal_size_bytes: u64,
    pub blocks_loaded: u32,
    pub avg_retention_days: u32,
}

pub fn tsdb_status(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<TsdbStatusRow, PrometheusViewError> {
    let targets = super::targets::list_targets(state, ctx)?;
    let head_series = targets.len() as u64;
    let head_samples: u64 = targets.iter().map(|t| t.sample_count).sum();
    // ~ 120 samples per chunk (Prometheus's default chunk size).
    let head_chunks = head_samples / 120 + targets.len() as u64;
    let symbol_table_size_bytes = head_series.saturating_mul(48); // 48 B / interned string (rough)
    let wal_size_bytes = head_samples.saturating_mul(16); // 16 B / sample
    let blocks_loaded = (targets.iter().map(|t| t.retention_days).max().unwrap_or(0) / 2) as u32;
    let avg_retention_days = if targets.is_empty() {
        0
    } else {
        (targets.iter().map(|t| t.retention_days as u64).sum::<u64>()
            / targets.len() as u64) as u32
    };
    Ok(TsdbStatusRow {
        head_series,
        head_chunks,
        head_samples,
        symbol_table_size_bytes,
        wal_size_bytes,
        blocks_loaded,
        avg_retention_days,
    })
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, PrometheusViewError> {
    let s = tsdb_status(state, ctx)?;
    Ok(format!(
        r#"<section id="prometheus-tsdb" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">TSDB Status</h2>
  <dl class="grid grid-cols-[16rem_1fr] gap-x-4 gap-y-1 text-sm">
    <dt class="text-gray-500">head series</dt><dd>{hs}</dd>
    <dt class="text-gray-500">head chunks</dt><dd>{hc}</dd>
    <dt class="text-gray-500">head samples</dt><dd>{hsmp}</dd>
    <dt class="text-gray-500">symbol table size</dt><dd>{sts} B</dd>
    <dt class="text-gray-500">WAL size</dt><dd>{wal} B</dd>
    <dt class="text-gray-500">blocks loaded</dt><dd>{bl}</dd>
    <dt class="text-gray-500">avg retention</dt><dd>{ret}d</dd>
  </dl>
  <p class="text-xs text-gray-500 mt-2">Derived from <code>cave-metrics</code> series catalog (Prometheus serves these via <code>/api/v1/status/tsdb</code>).</p>
</section>"#,
        hs = s.head_series,
        hc = s.head_chunks,
        hsmp = s.head_samples,
        sts = s.symbol_table_size_bytes,
        wal = s.wal_size_bytes,
        bl = s.blocks_loaded,
        ret = s.avg_retention_days,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn tsdb_status_head_series_matches_target_count() {
        use super::super::targets;
        let (_c, _t) = portal_test_ctx!(
            "plugins/prometheus/src/components/TsdbStatus.tsx",
            "Status",
            "acme"
        );
        let s = AdminState::seeded();
        let status = tsdb_status(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        let targets = targets::list_targets(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert_eq!(status.head_series, targets.len() as u64);
    }

    #[test]
    fn tsdb_status_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(tsdb_status(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn tsdb_status_handles_empty_seed() {
        let s = AdminState::empty();
        let status = tsdb_status(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        assert_eq!(status.head_series, 0);
        assert_eq!(status.wal_size_bytes, 0);
        assert_eq!(status.avg_retention_days, 0);
    }

    #[test]
    fn render_section_emits_all_status_fields() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::PrometheusRead])).unwrap();
        for label in ["head series", "head chunks", "WAL size", "blocks loaded", "avg retention"] {
            assert!(html.contains(label));
        }
    }
}
