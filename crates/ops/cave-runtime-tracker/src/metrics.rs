// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prometheus text-exposition rendering for the daily report.
//!
//! [`render_prometheus`] is a pure function from a [`DailyReport`] (plus
//! any LOC [`Measurement`]s) to the OpenMetrics-compatible text format
//! served at `/metrics` by the `serve` subcommand and written as a
//! node_exporter textfile by `metrics`. Keeping it pure means the whole
//! exposition is unit-tested without binding a socket.

use crate::measure::Measurement;
use crate::registry::DriftStatus;
use crate::report::DailyReport;

const NS: &str = "cave_runtime_tracker";

/// Numeric encoding of a drift verdict for the per-subsystem gauge.
fn drift_value(s: DriftStatus) -> u8 {
    match s {
        DriftStatus::InSync => 0,
        DriftStatus::Behind => 1,
        DriftStatus::Unknown => 2,
    }
}

/// Escape a Prometheus label *value* (backslash, double-quote, newline).
fn esc(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Render the full `/metrics` body for `report` + `measurements`.
pub fn render_prometheus(report: &DailyReport, measurements: &[Measurement]) -> String {
    let mut s = String::new();

    // ── headline totals ───────────────────────────────────────────────
    metric(
        &mut s,
        "tracked",
        "Total upstream subsystems tracked.",
        "gauge",
        &format!("{NS}_tracked {}", report.totals.tracked),
    );
    metric(
        &mut s,
        "status",
        "Subsystem count per drift status.",
        "gauge",
        &format!(
            "{NS}_status{{status=\"in_sync\"}} {}\n\
             {NS}_status{{status=\"behind\"}} {}\n\
             {NS}_status{{status=\"unknown\"}} {}",
            report.totals.in_sync, report.totals.behind, report.totals.unknown
        ),
    );
    metric(
        &mut s,
        "unresolved",
        "Upstream repos whose latest tag could not be fetched this run.",
        "gauge",
        &format!("{NS}_unresolved {}", report.poll.unresolved.len()),
    );
    metric(
        &mut s,
        "phase0_no_auto_bump",
        "1 while the tracker is report-only (no automatic version bumps).",
        "gauge",
        &format!("{NS}_phase0_no_auto_bump {}", report.phase_0_no_auto_bump as u8),
    );

    // ── per-subsystem drift gauge (0=in_sync,1=behind,2=unknown) ──────
    s.push_str(&format!(
        "# HELP {NS}_drift Per-subsystem drift state (0=in_sync,1=behind,2=unknown).\n\
         # TYPE {NS}_drift gauge\n"
    ));
    for r in &report.poll.results {
        s.push_str(&format!(
            "{NS}_drift{{name=\"{}\",module=\"{}\",repo=\"{}\",pinned=\"{}\",latest=\"{}\"}} {}\n",
            esc(&r.upstream.name),
            esc(&r.upstream.cave_module),
            esc(&r.upstream.repo),
            esc(r.upstream.pinned.as_deref().unwrap_or("")),
            esc(r.latest.as_deref().unwrap_or("")),
            drift_value(r.status),
        ));
    }
    s.push('\n');

    // ── LOC port-depth (only when measurements were taken) ────────────
    if !measurements.is_empty() {
        s.push_str(&format!(
            "# HELP {NS}_upstream_loc Upstream lines of code (tokei).\n\
             # TYPE {NS}_upstream_loc gauge\n"
        ));
        // De-dup the shared upstream rows — one line per distinct repo.
        let mut seen = std::collections::BTreeSet::new();
        for m in measurements {
            if let Some(up) = m.upstream
                && seen.insert(m.upstream_repo.clone())
            {
                s.push_str(&format!(
                    "{NS}_upstream_loc{{repo=\"{}\"}} {}\n",
                    esc(&m.upstream_repo),
                    up.code
                ));
            }
        }
        s.push('\n');

        s.push_str(&format!(
            "# HELP {NS}_cave_loc Cave-crate lines of code (tokei).\n\
             # TYPE {NS}_cave_loc gauge\n"
        ));
        for m in measurements {
            if let Some(cave) = m.cave {
                s.push_str(&format!(
                    "{NS}_cave_loc{{module=\"{}\",repo=\"{}\"}} {}\n",
                    esc(&m.cave_module),
                    esc(&m.upstream_repo),
                    cave.code
                ));
            }
        }
        s.push('\n');

        s.push_str(&format!(
            "# HELP {NS}_port_ratio Port-depth ratio cave_code/upstream_code.\n\
             # TYPE {NS}_port_ratio gauge\n"
        ));
        for m in measurements {
            if let Some(ratio) = m.ratio {
                s.push_str(&format!(
                    "{NS}_port_ratio{{module=\"{}\",repo=\"{}\"}} {ratio}\n",
                    esc(&m.cave_module),
                    esc(&m.upstream_repo),
                ));
            }
        }
        s.push('\n');
    }

    s
}

/// Emit a `# HELP`/`# TYPE` header for `<NS>_<name>` followed by `body`.
fn metric(buf: &mut String, name: &str, help: &str, kind: &str, body: &str) {
    buf.push_str(&format!("# HELP {NS}_{name} {help}\n"));
    buf.push_str(&format!("# TYPE {NS}_{name} {kind}\n"));
    buf.push_str(body);
    buf.push_str("\n\n");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TrackerConfig;
    use crate::measure::LocStats;
    use crate::poll::PollSummary;

    fn report() -> DailyReport {
        let cfg = TrackerConfig::default_config();
        DailyReport::assemble(PollSummary::from_registry_only(&cfg))
    }

    #[test]
    fn exposition_has_headers_and_totals() {
        let r = report();
        let out = render_prometheus(&r, &[]);
        assert!(out.contains("# TYPE cave_runtime_tracker_tracked gauge"));
        assert!(out.contains(&format!(
            "cave_runtime_tracker_tracked {}",
            r.totals.tracked
        )));
        assert!(out.contains("cave_runtime_tracker_status{status=\"unknown\"}"));
        assert!(out.contains("cave_runtime_tracker_phase0_no_auto_bump 1"));
    }

    #[test]
    fn drift_gauge_has_one_line_per_subsystem() {
        let r = report();
        let out = render_prometheus(&r, &[]);
        let lines = out
            .lines()
            .filter(|l| l.starts_with("cave_runtime_tracker_drift{"))
            .count();
        assert_eq!(lines, r.poll.results.len());
        // Registry-only → every row Unknown → drift value 2.
        assert!(out.contains("\",latest=\"\"} 2"));
    }

    #[test]
    fn loc_metrics_appear_only_with_measurements() {
        let r = report();
        assert!(!render_prometheus(&r, &[]).contains("upstream_loc"));

        let m = Measurement {
            upstream_repo: "cilium/cilium".to_string(),
            cave_module: "cave-net".to_string(),
            upstream: Some(LocStats { code: 400_000, ..Default::default() }),
            cave: Some(LocStats { code: 12_000, ..Default::default() }),
            ratio: Some(0.03),
        };
        let out = render_prometheus(&r, &[m]);
        assert!(out.contains("cave_runtime_tracker_upstream_loc{repo=\"cilium/cilium\"} 400000"));
        assert!(out.contains(
            "cave_runtime_tracker_cave_loc{module=\"cave-net\",repo=\"cilium/cilium\"} 12000"
        ));
        assert!(out.contains(
            "cave_runtime_tracker_port_ratio{module=\"cave-net\",repo=\"cilium/cilium\"} 0.03"
        ));
    }

    #[test]
    fn shared_upstream_loc_is_deduped() {
        let r = report();
        let mk = |module: &str| Measurement {
            upstream_repo: "kubernetes/kubernetes".to_string(),
            cave_module: module.to_string(),
            upstream: Some(LocStats { code: 5_000_000, ..Default::default() }),
            cave: Some(LocStats { code: 10_000, ..Default::default() }),
            ratio: Some(0.002),
        };
        let out = render_prometheus(&r, &[mk("cave-apiserver"), mk("cave-scheduler")]);
        // One upstream_loc line for the shared repo, two cave_loc lines.
        assert_eq!(
            out.matches("cave_runtime_tracker_upstream_loc{repo=\"kubernetes/kubernetes\"}")
                .count(),
            1
        );
        assert_eq!(out.matches("cave_runtime_tracker_cave_loc{").count(), 2);
    }

    #[test]
    fn label_values_are_escaped() {
        assert_eq!(esc(r#"a"b\c"#), r#"a\"b\\c"#);
    }
}
