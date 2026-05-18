// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/_audit` — consolidated portal-wide audit roll-up.
//!
//! The single page Burak should land on to answer "how is everything
//! going?" without having to dispatch any of the underlying audits
//! manually. Pulls live data from:
//!
//! * **Structural** — `compliance::cached_snapshot_or_refresh()`
//!   .grade() — Portal/cavectl/Observability presence.
//! * **Upstream Parity** — `parity_grade()` — average of declared
//!   `parity_ratio` over tier-1 crates.
//! * **Honest Parity** — `honest_parity_grade()` — same minus
//!   author-declared `[[partial]]` blocks.
//! * **Behavioral Parity** — `behavioral_grade()` — ported upstream
//!   tests over total declared.
//! * **Accessibility** — `layout::a11y::audit()` over the synthetic
//!   chrome (`shell_v2(opts)`). 0 violations → A; one → B; more → F.
//!
//! Five axes, one row of letter cards + score + sparkline of the
//! last 12 samples per axis. PlatformAdmin only — same gate as
//! `/admin/compliance` because the data is cross-tenant.

use crate::admin::compliance::{self, ComplianceSnapshot, ComplianceViewError};
use crate::admin::layout::a11y;
use crate::admin::layout::shell::{shell_v2, ShellOptions};
use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::{Mutex, OnceLock};

/// One audit axis on the roll-up dashboard.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GradeAxis {
    /// Stable machine name — used in the JSON wire + CLI columns.
    pub name: String,
    /// Human-readable label — shown on the card.
    pub label: String,
    /// 0-100 numeric score.
    pub score: u8,
    /// A-F letter derived from `score`.
    pub grade: char,
    /// One-line description that explains what the axis measures.
    pub description: String,
}

/// Top-level wire shape — what `/admin/_audit.json` returns and what
/// the `cavectl portal audit` CLI deserialises.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditSummary {
    /// Five-axis grade roll-up. Order matches the dashboard cards
    /// (Structural / Upstream / Honest / Behavioral / Accessibility).
    pub axes: Vec<GradeAxis>,
    /// Wall-clock instant the underlying compliance snapshot was
    /// last refreshed. The a11y check runs on the live render so it
    /// is implicitly current.
    pub last_audit: DateTime<Utc>,
    /// Number of crates in the structural snapshot (sanity number for
    /// the dashboard header).
    pub total_crates: usize,
    /// Total stub count (`unimplemented!()` + `todo!()` + ignored
    /// tests) across the workspace — the headline regression number.
    pub total_stubs: u32,
}

/// Errors shared with the rest of the admin/ module.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuditViewError {
    #[error("/admin/_audit requires the PlatformAdmin persona")]
    PersonaRequired,
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
    #[error(transparent)]
    Compliance(#[from] ComplianceViewError),
}

// ── Public API ────────────────────────────────────────────────────

/// Build the live summary. Reads the cached compliance snapshot
/// (refreshing if stale) and runs the a11y checker against the
/// synthetic chrome.
pub fn summary() -> AuditSummary {
    let snap = compliance::cached_snapshot_or_refresh();
    let last_audit = compliance::cache_cached_at().unwrap_or_else(Utc::now);
    summary_from(&snap, last_audit)
}

/// Pure variant — used by tests.
pub fn summary_from(snap: &ComplianceSnapshot, last_audit: DateTime<Utc>) -> AuditSummary {
    let a11y_score = compute_a11y_score();
    let a11y_grade = letter_for(a11y_score);

    let axes = vec![
        GradeAxis {
            name: "structural".into(),
            label: "Structural".into(),
            score: snap.aggregate_score(),
            grade: snap.grade(),
            description: "Portal / cavectl / Observability presence per crate".into(),
        },
        GradeAxis {
            name: "upstream_parity".into(),
            label: "Upstream Parity".into(),
            score: snap.aggregate_parity_score(),
            grade: snap.parity_grade(),
            description: "Average declared parity_ratio over tier-1 crates".into(),
        },
        GradeAxis {
            name: "honest_parity".into(),
            label: "Honest Parity".into(),
            score: snap.aggregate_honest_parity_score(),
            grade: snap.honest_parity_grade(),
            description: "Parity minus author-declared partial blocks".into(),
        },
        GradeAxis {
            name: "behavioral_parity".into(),
            label: "Behavioral Parity".into(),
            score: snap.behavioral_parity_avg(),
            grade: snap.behavioral_grade(),
            description: "Upstream tests ported over upstream tests declared".into(),
        },
        GradeAxis {
            name: "accessibility".into(),
            label: "Accessibility".into(),
            score: a11y_score,
            grade: a11y_grade,
            description: "WCAG 2.1 AA static check over the synthetic chrome".into(),
        },
    ];

    AuditSummary {
        axes,
        last_audit,
        total_crates: snap.crates.len(),
        total_stubs: snap.total_stub_count(),
    }
}

/// Render the dashboard HTML. Returns `PersonaRequired` for any
/// caller that isn't PlatformAdmin so the route handler can map to a
/// 403.
pub fn render(ctx: &RequestCtx) -> Result<String, AuditViewError> {
    if !ctx.persona.is_platform() {
        return Err(AuditViewError::PersonaRequired);
    }
    ctx.authorise(Permission::AdminComplianceView)?;

    let s = summary();
    push_history(&s);
    let history = match history_store().lock() {
        Ok(g) => g.snapshot(),
        Err(p) => p.into_inner().snapshot(),
    };

    let cards = s
        .axes
        .iter()
        .map(|a| render_card(a, &history))
        .collect::<Vec<_>>()
        .join("\n");

    let body = format!(
        r#"<section aria-labelledby="audit-heading">
  <header class="mb-4 flex items-baseline justify-between">
    <h2 id="audit-heading" class="text-lg font-semibold">Portal-wide audit</h2>
    <p class="text-xs text-zinc-500 dark:text-zinc-400">
      Last refreshed <time datetime="{ts}">{ts_short}</time> · {n} crates · {stubs} stubs
    </p>
  </header>
  <div class="grid grid-cols-1 sm:grid-cols-2 lg:grid-cols-5 gap-3 mb-4">
    {cards}
  </div>
  <div class="flex gap-3 text-sm">
    <a class="px-3 py-1 rounded bg-blue-600 text-white focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
       href="/admin/compliance/refresh?tenant_id={tid}">Refresh now</a>
    <a class="px-3 py-1 rounded border border-zinc-300 dark:border-zinc-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
       href="/admin/_audit.json?tenant_id={tid}">JSON feed</a>
    <a class="px-3 py-1 rounded border border-zinc-300 dark:border-zinc-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
       href="/admin/compliance?tenant_id={tid}">Open Compliance →</a>
    <a class="px-3 py-1 rounded border border-zinc-300 dark:border-zinc-700 focus-visible:outline focus-visible:outline-2 focus-visible:outline-blue-600"
       href="/admin/upstream?tenant_id={tid}">Open Upstream →</a>
  </div>
  <p class="mt-4 text-xs text-zinc-500 dark:text-zinc-400">
    Sparkline shows the last {hist} samples taken on each render of
    this page; samples are kept in-memory and reset across restarts.
  </p>
</section>"#,
        ts = s.last_audit.to_rfc3339(),
        ts_short = s.last_audit.format("%Y-%m-%d %H:%M UTC"),
        n = s.total_crates,
        stubs = s.total_stubs,
        tid = escape(ctx.tenant.as_str()),
        cards = cards,
        hist = HISTORY_CAP,
    );

    Ok(page_shell_full(
        ctx,
        "/admin/_audit",
        &format!("audit · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// JSON shape — same `AuditSummary` returned to the dashboard, but
/// without HTML wrapping. Used by `cavectl portal audit`.
pub fn render_json(ctx: &RequestCtx) -> Result<AuditSummary, AuditViewError> {
    if !ctx.persona.is_platform() {
        return Err(AuditViewError::PersonaRequired);
    }
    ctx.authorise(Permission::AdminComplianceView)?;
    Ok(summary())
}

// ── Card / sparkline helpers ──────────────────────────────────────

fn render_card(axis: &GradeAxis, history: &HistorySnapshot) -> String {
    let grade_color = match axis.grade {
        'A' => "text-emerald-700 dark:text-emerald-300",
        'B' => "text-lime-700 dark:text-lime-300",
        'C' => "text-amber-700 dark:text-amber-300",
        'D' => "text-orange-700 dark:text-orange-300",
        _ => "text-red-700 dark:text-red-300",
    };
    let series = history.series_for(&axis.name);
    let spark = sparkline_svg(&series);
    format!(
        r#"<article class="rounded border border-zinc-200 dark:border-zinc-800 bg-white dark:bg-zinc-900 p-3"
                aria-labelledby="card-{name}-h">
      <h3 id="card-{name}-h" class="text-xs uppercase tracking-wider text-zinc-500 mb-1">{label}</h3>
      <div class="flex items-baseline justify-between">
        <span class="text-3xl font-bold {color}" aria-label="grade {grade}">{grade}</span>
        <span class="text-sm text-zinc-700 dark:text-zinc-300 tabular-nums">{score}</span>
      </div>
      <div class="mt-1" aria-label="sparkline of last {n} samples">{spark}</div>
      <p class="text-[11px] text-zinc-500 dark:text-zinc-400 mt-1">{desc}</p>
    </article>"#,
        name = escape(&axis.name),
        label = escape(&axis.label),
        color = grade_color,
        grade = axis.grade,
        score = axis.score,
        n = series.len(),
        spark = spark,
        desc = escape(&axis.description),
    )
}

/// Render a tiny inline SVG sparkline. Empty series → an empty
/// horizontal rule so the card layout doesn't shift.
pub fn sparkline_svg(series: &[u8]) -> String {
    if series.is_empty() {
        return r#"<svg viewBox="0 0 100 20" class="w-full h-4" aria-hidden="true"><line x1="0" y1="10" x2="100" y2="10" stroke="currentColor" stroke-opacity="0.2" stroke-dasharray="2 2"/></svg>"#.into();
    }
    let n = series.len();
    let step = if n > 1 { 100.0 / (n as f64 - 1.0) } else { 0.0 };
    let pts = series
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = (i as f64) * step;
            // Invert: 100 → top (y=2), 0 → bottom (y=18).
            let y = 18.0 - (f64::from(v) / 100.0) * 16.0;
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        r#"<svg viewBox="0 0 100 20" class="w-full h-4" aria-hidden="true" preserveAspectRatio="none">
  <polyline points="{pts}" fill="none" stroke="currentColor" stroke-width="1.4"/>
</svg>"#,
        pts = pts,
    )
}

fn compute_a11y_score() -> u8 {
    let html = shell_v2(ShellOptions {
        title: "audit synthesised chrome",
        persona: Persona::PlatformAdmin,
        tenant_id: "audit",
        current_path: "/admin/_audit",
        theme_cookie: None,
        breadcrumb: None,
        extra_commands: Vec::new(),
        cluster_info: "audit",
        hide_sidebar: false,
        body: "<p>audit body</p>",
    });
    let issues = a11y::audit(&html);
    // 0 issues → 100; 1 → 80; 2 → 60; 3 → 40; 4 → 20; 5+ → 0.
    100u8.saturating_sub(20u8.saturating_mul(issues.len().min(5) as u8))
}

fn letter_for(score: u8) -> char {
    match score {
        90..=100 => 'A',
        80..=89 => 'B',
        70..=79 => 'C',
        60..=69 => 'D',
        _ => 'F',
    }
}

// ── In-memory sparkline history ───────────────────────────────────

const HISTORY_CAP: usize = 12;

#[derive(Debug, Default, Clone)]
pub struct HistorySnapshot {
    inner: std::collections::BTreeMap<String, Vec<u8>>,
}

impl HistorySnapshot {
    pub fn series_for(&self, axis_name: &str) -> Vec<u8> {
        self.inner.get(axis_name).cloned().unwrap_or_default()
    }
}

#[derive(Debug, Default)]
struct History {
    rings: std::collections::BTreeMap<String, VecDeque<u8>>,
}

impl History {
    fn push(&mut self, axis: &str, score: u8) {
        let ring = self.rings.entry(axis.to_string()).or_default();
        if ring.len() == HISTORY_CAP {
            ring.pop_front();
        }
        ring.push_back(score);
    }

    fn snapshot(&self) -> HistorySnapshot {
        HistorySnapshot {
            inner: self
                .rings
                .iter()
                .map(|(k, v)| (k.clone(), v.iter().copied().collect()))
                .collect(),
        }
    }
}

fn history_store() -> &'static Mutex<History> {
    static CELL: OnceLock<Mutex<History>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(History::default()))
}

fn push_history(s: &AuditSummary) {
    let mut guard = match history_store().lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    for axis in &s.axes {
        guard.push(&axis.name, axis.score);
    }
}

#[cfg(test)]
pub fn reset_history_for_tests() {
    if let Ok(mut g) = history_store().lock() {
        *g = History::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::compliance::{ComplianceSnapshot, CrateCompliance};
    use crate::admin::permission::Permission;

    fn snap(crates: Vec<CrateCompliance>) -> ComplianceSnapshot {
        ComplianceSnapshot { crates }
    }

    fn dev_crate(name: &str, score: u8, parity: f64) -> CrateCompliance {
        CrateCompliance {
            name: name.into(),
            upstream_version: None,
            upstream_org_repo: None,
            backend_loc: 100,
            backend_test_count: 5,
            ignored_test_count: 0,
            unimplemented_count: 0,
            todo_count: 0,
            portal_admin_present: true,
            cavectl_subcommand_present: true,
            obs_alerts_present: true,
            obs_dashboard_present: true,
            four_track_score: score,
            infra_only: false,
            parity_ratio: Some(parity),
            parity_ratio_source: Some("manifest".into()),
            parity_ratio_last_audit: None,
            honest_parity_ratio: Some(parity),
            parity_mapped_count: None,
            parity_partial_count: None,
            parity_skipped_count: None,
            parity_unmapped_count: None,
            parity_total_count: None,
            manifest_filled: Some(true),
            audit_tier: None,
            portal_ui_status: None,
            portal_ui_priority: None,
            portal_ui_upstream_url: None,
            portal_ui_score: None,
            behavioral_parity: Some(0.85),
            behavioral_ported: Some(17),
            behavioral_total: Some(20),
            behavioral_audit_scope: None,
            behavioral_audit_at: None,
        }
    }

    fn platform_ctx() -> RequestCtx {
        let mut c = RequestCtx::developer("acme", &[Permission::AdminComplianceView]);
        c.persona = Persona::PlatformAdmin;
        c
    }

    fn tenant_ctx() -> RequestCtx {
        RequestCtx::developer_as(
            "acme",
            &[Permission::AdminComplianceView],
            Persona::TenantAdmin,
        )
    }

    // ── summary axes ─────────────────────────────────────────────

    #[test]
    fn summary_returns_five_axes_in_canonical_order() {
        let s = summary_from(
            &snap(vec![dev_crate("c1", 100, 0.9)]),
            Utc::now(),
        );
        assert_eq!(s.axes.len(), 5);
        let names: Vec<&str> = s.axes.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(
            names,
            vec![
                "structural",
                "upstream_parity",
                "honest_parity",
                "behavioral_parity",
                "accessibility",
            ]
        );
    }

    #[test]
    fn summary_picks_up_structural_score_from_compliance() {
        let s = summary_from(
            &snap(vec![dev_crate("c1", 100, 0.9)]),
            Utc::now(),
        );
        let structural = &s.axes[0];
        assert_eq!(structural.name, "structural");
        assert_eq!(structural.score, 100);
        assert_eq!(structural.grade, 'A');
    }

    #[test]
    fn summary_picks_up_upstream_parity_from_compliance() {
        let s = summary_from(
            &snap(vec![dev_crate("c1", 100, 0.95)]),
            Utc::now(),
        );
        let parity = &s.axes[1];
        assert_eq!(parity.name, "upstream_parity");
        assert_eq!(parity.score, 95);
    }

    #[test]
    fn summary_a11y_axis_is_grade_a_when_chrome_clean() {
        let s = summary_from(
            &snap(vec![dev_crate("c1", 100, 0.9)]),
            Utc::now(),
        );
        let a11y = &s.axes[4];
        assert_eq!(a11y.name, "accessibility");
        // shell_v2 chrome has 0 violations (locked in by a11y::tests::shell_v2_passes_full_a11y_audit).
        assert_eq!(a11y.score, 100);
        assert_eq!(a11y.grade, 'A');
    }

    #[test]
    fn summary_serialises_to_json() {
        let s = summary_from(
            &snap(vec![dev_crate("c1", 100, 0.9)]),
            Utc::now(),
        );
        let json = serde_json::to_value(&s).unwrap();
        assert!(json["axes"].is_array());
        assert_eq!(json["axes"].as_array().unwrap().len(), 5);
        assert!(json["last_audit"].is_string());
        assert!(json["total_crates"].is_number());
    }

    // ── persona gate ──────────────────────────────────────────────

    #[test]
    fn render_refuses_tenant_admin() {
        let err = render(&tenant_ctx()).unwrap_err();
        assert!(matches!(err, AuditViewError::PersonaRequired));
    }

    #[test]
    fn render_json_refuses_tenant_admin() {
        let err = render_json(&tenant_ctx()).unwrap_err();
        assert!(matches!(err, AuditViewError::PersonaRequired));
    }

    #[test]
    fn render_succeeds_for_platform_admin() {
        let html = render(&platform_ctx()).expect("render must succeed for PlatformAdmin");
        assert!(html.contains("Portal-wide audit"));
        assert!(html.contains("Structural"));
        assert!(html.contains("Accessibility"));
    }

    #[test]
    fn render_chrome_is_a11y_clean() {
        let html = render(&platform_ctx()).unwrap();
        let issues = crate::admin::layout::a11y::audit(&html);
        assert!(
            issues.is_empty(),
            "/admin/_audit must be WCAG AA clean; got {issues:?}"
        );
    }

    #[test]
    fn render_includes_jsonn_and_refresh_action_links() {
        let html = render(&platform_ctx()).unwrap();
        assert!(html.contains("/admin/_audit.json?tenant_id=acme"));
        assert!(html.contains("/admin/compliance/refresh?tenant_id=acme"));
    }

    // ── sparkline ─────────────────────────────────────────────────

    #[test]
    fn sparkline_renders_polyline_with_n_points() {
        let svg = sparkline_svg(&[10, 20, 50, 100]);
        assert!(svg.contains("<polyline"));
        // 4 points → 4 coordinate pairs.
        assert_eq!(svg.matches(',').count(), 4);
    }

    #[test]
    fn sparkline_empty_series_emits_dashed_baseline() {
        let svg = sparkline_svg(&[]);
        assert!(svg.contains("stroke-dasharray"));
    }

    #[test]
    fn sparkline_normalises_y_axis_to_viewport() {
        let svg = sparkline_svg(&[100]);
        // 100 → y near 2 (top of 0..20 viewport).
        assert!(svg.contains("2.0"), "expected y=2.0 for score=100, got: {svg}");
        let svg = sparkline_svg(&[0]);
        // 0 → y near 18 (bottom).
        assert!(svg.contains("18.0"), "expected y=18.0 for score=0, got: {svg}");
    }

    // ── history ring ──────────────────────────────────────────────

    #[test]
    fn history_keeps_at_most_twelve_samples_per_axis() {
        reset_history_for_tests();
        let s = summary_from(
            &snap(vec![dev_crate("c1", 100, 0.9)]),
            Utc::now(),
        );
        for _ in 0..20 {
            push_history(&s);
        }
        let snap = history_store().lock().unwrap().snapshot();
        for axis in &s.axes {
            assert!(
                snap.series_for(&axis.name).len() <= HISTORY_CAP,
                "axis {} ring overflowed cap {HISTORY_CAP}",
                axis.name
            );
        }
    }

    #[test]
    fn history_records_each_render() {
        reset_history_for_tests();
        let _ = render(&platform_ctx()).unwrap();
        let snap = history_store().lock().unwrap().snapshot();
        // Every axis recorded one sample after a single render.
        assert!(snap.series_for("structural").len() >= 1);
        assert!(snap.series_for("accessibility").len() >= 1);
    }

    #[test]
    fn letter_for_grade_boundaries() {
        assert_eq!(letter_for(100), 'A');
        assert_eq!(letter_for(90), 'A');
        assert_eq!(letter_for(89), 'B');
        assert_eq!(letter_for(80), 'B');
        assert_eq!(letter_for(79), 'C');
        assert_eq!(letter_for(60), 'D');
        assert_eq!(letter_for(59), 'F');
        assert_eq!(letter_for(0), 'F');
    }
}
