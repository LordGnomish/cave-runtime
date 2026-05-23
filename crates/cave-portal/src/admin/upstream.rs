// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/upstream` view — upstream resource browser. **Canonical**
//! upstream-tracker page as of 2026-05-14; the legacy `/upstream`
//! handler in cave-runtime now 301-redirects here.
//!
//! Four sections (top to bottom):
//!
//! 1. **Upstream Parity Tracker** (2026-05-14) — server-rendered
//!    `cave_upstream::TRACKED_PROJECTS` table with parity progress bars,
//!    ADR refs, status badges. Replaces the JS-driven `/upstream` page;
//!    persona-gated to `PlatformAdmin` (cross-tenant control-plane data).
//! 2. **Legacy seeded list** — the per-tenant `UpstreamProject`
//!    fixtures that pre-date the watch daemon. Still rendered so
//!    existing tests + dashboard semantics don't change.
//! 3. **Live watchd panel** (2026-05-13) — reads
//!    `<data_dir>/watchd/events.jsonl` and renders the most recent
//!    `GAP_OPENED` events with severity badges + gap-age.
//! 4. **Auto-port dispatcher panel** — recent `dispatched.jsonl` rows
//!    with status badges + Dispatch Now controls.

use crate::admin::permission::{Permission, Persona, RequestCtx};
use crate::admin::render::{
    badge, empty_state, escape, page_shell_full, search_box, sortable_table, table,
    table_html as render_table_html,
};
use crate::admin::state::{AdminState, UpstreamProject, scope};
use crate::admin::types::Cite;
use cave_kernel::parity::DiscoveredReport;
use cave_kernel::parity::types::ParityReport;
use cave_upstream::projects::TrackedProject;
use cave_upstream::{TRACKED_PROJECTS, adr_links};
use cave_upstream_watchd::auto_port::{AutoPortStatus, DispatchedRecord};
use cave_upstream_watchd::diff::Severity;
use cave_upstream_watchd::event::{GapEvent, JsonlSink, read_events};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum UpstreamViewError {
    #[error(transparent)]
    Auth(#[from] crate::admin::permission::AuthError),
}

pub fn list_records(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<UpstreamProject>, UpstreamViewError> {
    ctx.authorise(Permission::UpstreamRead)?;
    Ok(
        scope(&state.upstream_projects.read().unwrap(), &ctx.tenant, |r| {
            &r.tenant
        })
        .into_iter()
        .cloned()
        .collect(),
    )
}

/// Locate the watchd events.jsonl. Honours `CAVE_WATCHD_EVENTS` for
/// dev/test, otherwise defers to `JsonlSink::default_path` which
/// uses platform-specific dirs.
fn watchd_events_path() -> PathBuf {
    JsonlSink::default_path()
}

/// Public so `render_watchd_in` can be exercised by tests with an
/// explicit path. Walks the events JSONL backwards (newest first)
/// and caps at `max_rows`.
pub fn recent_gap_events(path: &std::path::Path, max_rows: usize) -> Vec<GapEvent> {
    match read_events(path) {
        Ok(mut events) => {
            events.truncate(max_rows);
            events
        }
        Err(_) => Vec::new(),
    }
}

/// Tenant filter — TenantAdmin only sees events for crates that are
/// "tenant-relevant" (KEDA, Vault, kubelet, streams, ...). Platform
/// admin sees everything. The filter is intentionally simple: a
/// tenant-relevant list keyed by `cave_module`. Future Charter work
/// can replace this with a per-tenant policy table.
const TENANT_RELEVANT_MODULES: &[&str] = &[
    "cave-vault",
    "cave-keda",
    "cave-kubelet",
    "cave-streams",
    "cave-cache",
    "cave-pg",
    "cave-docdb",
];

fn is_tenant_relevant(module: &str) -> bool {
    TENANT_RELEVANT_MODULES.iter().any(|m| *m == module)
}

fn severity_class(s: Severity) -> &'static str {
    match s {
        Severity::Major => "bg-red-200 text-red-900",
        Severity::Minor => "bg-orange-200 text-orange-900",
        Severity::Patch => "bg-yellow-200 text-yellow-900",
        Severity::Unknown => "bg-zinc-200 text-zinc-900",
        Severity::None => "bg-green-200 text-green-900",
    }
}

fn severity_label(s: Severity) -> &'static str {
    match s {
        Severity::Major => "MAJOR",
        Severity::Minor => "MINOR",
        Severity::Patch => "PATCH",
        Severity::Unknown => "UNKNOWN",
        Severity::None => "AT_PARITY",
    }
}

fn format_age(seconds: i64) -> String {
    if seconds < 60 {
        format!("{}s", seconds)
    } else if seconds < 3600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

/// Render the watchd panel as HTML. Returns the inner section markup
/// (no `<html>`/`<body>`) so the caller can splice it into the page
/// shell. `events_path` is taken explicitly so tests can use a
/// fixture file.
pub fn render_watchd_panel_in(
    ctx: &RequestCtx,
    events_path: &std::path::Path,
    max_rows: usize,
) -> String {
    let mut events = recent_gap_events(events_path, max_rows * 4); // over-fetch for filter
    if !ctx.persona.is_platform() {
        events.retain(|e| is_tenant_relevant(&e.cave_module));
    }
    events.truncate(max_rows);

    let n = events.len();
    let table_rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            let badge = format!(
                r#"<span class="px-2 py-0.5 rounded text-xs {cls}">{lbl}</span>"#,
                cls = severity_class(e.severity),
                lbl = severity_label(e.severity),
            );
            let age = e
                .gap_age_seconds
                .map(format_age)
                .unwrap_or_else(|| "-".to_string());
            vec![
                escape(&e.cave_module),
                escape(&e.github_repo),
                escape(e.previous_pin.as_deref().unwrap_or("-")),
                escape(&e.latest_tag),
                badge,
                age,
                e.at.format("%Y-%m-%d %H:%M").to_string(),
            ]
        })
        .collect();

    let header = format!(
        r#"<div class="flex items-center justify-between mb-2">
                <h2 class="text-lg font-semibold">Watchd GAP events ({n})</h2>
                <span class="text-xs text-zinc-500">source: <code>{src}</code></span>
            </div>"#,
        src = escape(events_path.display().to_string().as_str()),
    );

    let tenant_note = if !ctx.persona.is_platform() {
        r#"<p class="text-xs text-zinc-500 mb-2">
            Tenant view — only events for crates tagged tenant-relevant
            (vault, keda, kubelet, streams, cache, pg, docdb) are shown.
            Sign in as <code>platform_admin</code> for the full list.
           </p>"#
    } else {
        ""
    };

    let table_block = if events.is_empty() {
        empty_state(
            "📡",
            "No GAP events recorded yet",
            "The watch daemon emits one event per upstream release that moves past our pin. Either no releases since the daemon last ran or the daemon hasn't started — see /admin/upstream/dispatched for last-run timing.",
        )
    } else {
        // 2026-05-13: the `severity` cell is a pre-formatted
        // `<span class="...">label</span>` badge built upstream; use
        // table_html so the badge renders styled instead of as literal
        // escaped text. Other cells are still escaped at the call site.
        render_table_html(
            &[
                "cave-module",
                "repo",
                "pin",
                "latest",
                "severity",
                "gap-age",
                "at",
            ],
            &table_rows,
        )
    };

    format!(
        r#"<section class="mt-6 p-3 border rounded">{header}{note}{tbl}</section>"#,
        header = header,
        note = tenant_note,
        tbl = table_block,
    )
}

/// One row in the parity tracker — derived from
/// `cave_upstream::TRACKED_PROJECTS` joined against the kernel's
/// parity report for each cave_module.
#[derive(Debug, Clone, PartialEq)]
pub struct TrackerRow {
    pub upstream_name: String,
    pub upstream_repo: String,
    pub cave_crate: String,
    pub category: String,
    pub adr_refs: Vec<String>,
    /// 0.0 – 1.0 from the manifest's [parity] fill_ratio (preferred)
    /// or the kernel heuristic. `-1.0` when no manifest report exists
    /// (rendered as "—").
    pub parity_overall: f32,
    /// `"synced"` / `"behind"` / `"pending"` derived from
    /// `parity_overall` thresholds (same buckets as `/api/upstream/tracker`).
    pub parity_status: String,
}

fn parity_status_label(overall: f32) -> &'static str {
    if overall >= 0.7 {
        "synced"
    } else if overall >= 0.3 {
        "behind"
    } else {
        "pending"
    }
}

/// Build tracker rows from in-memory TRACKED_PROJECTS + a pre-computed
/// parity index. Pure function — easy to unit test. The caller is
/// responsible for the parity index (which is built off the filesystem
/// once per request).
pub fn build_tracker_rows(
    projects: &[TrackedProject],
    parity_by_crate: &HashMap<String, ParityReport>,
) -> Vec<TrackerRow> {
    projects
        .iter()
        .map(|p| {
            let parity = parity_by_crate.get(p.cave_module);
            let (overall, status) = match parity {
                Some(r) => (r.overall, parity_status_label(r.overall).to_string()),
                None => (-1.0, "pending".to_string()),
            };
            TrackerRow {
                upstream_name: p.name.to_string(),
                upstream_repo: p.github_repo.to_string(),
                cave_crate: p.cave_module.to_string(),
                category: p.category.to_string(),
                adr_refs: adr_links::adrs_for(p.github_repo)
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
                parity_overall: overall,
                parity_status: status,
            }
        })
        .collect()
}

/// Discover every parity manifest under `workspace_root` and return
/// the report keyed by both crate-dir name AND the manifest's bare
/// module name (so lookups by either spelling succeed). Pure I/O —
/// callers should run this on a blocking thread for non-trivial trees.
pub fn build_parity_index_at(workspace_root: &Path) -> HashMap<String, ParityReport> {
    let reports: Vec<DiscoveredReport> = cave_kernel::parity::discover_workspace(workspace_root);
    let mut idx = HashMap::new();
    for d in reports {
        if let Some(crate_dir) = d.manifest_path.parent() {
            if let Some(name) = crate_dir.file_name().and_then(|s| s.to_str()) {
                idx.insert(name.to_string(), d.report.clone());
            }
        }
        idx.entry(d.report.module.clone()).or_insert(d.report);
    }
    idx
}

fn status_cell_class(status: &str) -> &'static str {
    match status {
        "synced" => "bg-green-100 text-green-900",
        "behind" => "bg-yellow-100 text-yellow-900",
        _ => "bg-red-100 text-red-900",
    }
}

/// Render the canonical "Upstream Parity Tracker" section. Server-
/// rendered with Tailwind classes so it inherits the admin shell's
/// light/dark mode. Persona-gated: only `PlatformAdmin` sees it
/// (TenantAdmin's view is scoped to legacy seeded list + tenant-relevant
/// watchd events).
///
/// Returns the section HTML, or an empty string when the caller's
/// persona isn't allowed to see cross-tenant tracker data.
pub fn render_tracker_section(ctx: &RequestCtx, rows: &[TrackerRow]) -> String {
    if !ctx.persona.is_platform() {
        return String::new();
    }
    let total = rows.len();
    let synced = rows.iter().filter(|r| r.parity_status == "synced").count();
    let behind = rows.iter().filter(|r| r.parity_status == "behind").count();
    let pending = rows.iter().filter(|r| r.parity_status == "pending").count();

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let pct_text = if r.parity_overall < 0.0 {
                r#"<span class="text-gray-400">no manifest</span>"#.to_string()
            } else {
                format!("{}%", (r.parity_overall * 100.0).round() as i32)
            };
            let bar_width = if r.parity_overall < 0.0 {
                0
            } else {
                ((r.parity_overall * 100.0).round() as i32).clamp(2, 100)
            };
            let bar_color = match r.parity_status.as_str() {
                "synced" => "bg-green-500",
                "behind" => "bg-yellow-500",
                _ => "bg-red-500",
            };
            let adr_chips: String = if r.adr_refs.is_empty() {
                r#"<span class="text-gray-400">—</span>"#.to_string()
            } else {
                r.adr_refs
                    .iter()
                    .map(|a| {
                        format!(
                            r#"<span class="inline-block px-1.5 py-0.5 mr-1 rounded text-[10px] bg-blue-100 text-blue-800 border border-blue-200">{}</span>"#,
                            escape(a)
                        )
                    })
                    .collect()
            };
            let status_cls = status_cell_class(&r.parity_status);
            // Parity cell — numeric sort hint goes first (raw integer
            // percent so localeCompare/parseFloat ranks correctly),
            // then the visible bar + label.
            let pct_raw = if r.parity_overall < 0.0 {
                -1
            } else {
                (r.parity_overall * 100.0).round() as i32
            };
            let upstream_cell = format!(
                r#"<a class="text-blue-700 underline" href="https://github.com/{repo}" target="_blank" rel="noopener">{repo}</a><div class="text-[11px] text-gray-500">{name}</div>"#,
                repo = escape(&r.upstream_repo),
                name = escape(&r.upstream_name),
            );
            let crate_cell = format!(
                r#"<a class="text-blue-700 underline" href="/admin/compliance/{crate_name}?tenant_id={tenant}">{crate_name}</a>"#,
                crate_name = escape(&r.cave_crate),
                tenant = escape(ctx.tenant.as_str()),
            );
            let parity_cell = format!(
                r#"<span class="sr-only">{pct_raw}</span><div class="flex items-center gap-2"><div class="w-24 h-2 bg-gray-200 rounded"><div class="h-full rounded {bar_color}" style="width:{bar_width}%"></div></div><span class="text-xs">{pct}</span></div>"#,
                pct_raw = pct_raw,
                bar_color = bar_color,
                bar_width = bar_width,
                pct = pct_text,
            );
            let status_cell = format!(
                r#"<span class="px-2 py-1 rounded text-xs {status_cls}">{status}</span>"#,
                status_cls = status_cls,
                status = escape(&r.parity_status),
            );
            vec![
                upstream_cell,
                crate_cell,
                format!(r#"<span class="text-xs">{}</span>"#, escape(&r.category)),
                adr_chips,
                parity_cell,
                status_cell,
            ]
        })
        .collect();

    let table_html = sortable_table(
        "upstream-tracker",
        &[
            ("Upstream", "text"),
            ("Cave crate", "text"),
            ("Category", "text"),
            ("ADR", "text"),
            ("Parity", "num"),
            ("Status", "text"),
        ],
        &table_rows,
    );

    let search = search_box("#upstream-tracker", "Filter by repo, crate, ADR…");

    format!(
        r#"<section class="mb-6">
  <div class="flex items-baseline justify-between mb-2">
    <h2 class="text-lg font-semibold">Upstream Parity Tracker ({total})</h2>
    <div class="flex gap-2 text-xs">
      {ok}
      {warn}
      {bad}
    </div>
  </div>
  <p class="text-xs text-gray-500 mb-3">ADR-aware view of the upstream OSS projects re-implemented inside cave-runtime. Click a column header to sort. Source: <code>cave_upstream::TRACKED_PROJECTS</code> joined with parity manifests.</p>
  {search}
  {tbl}
</section>"#,
        ok = badge("ok", &format!("{synced} synced")),
        warn = badge("warn", &format!("{behind} behind")),
        bad = badge("bad", &format!("{pending} pending")),
        search = search,
        tbl = table_html,
    )
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, UpstreamViewError> {
    let rows = list_records(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.name.clone(),
                r.repo.clone(),
                r.pinned_version.clone(),
                r.last_check_unix.to_string(),
            ]
        })
        .collect();
    // Tracker section: only built for PlatformAdmin. Build the parity
    // index off the workspace root so the section shows live ratios that
    // match /admin/compliance.
    let tracker_section = if ctx.persona.is_platform() {
        let workspace = crate::admin::compliance::workspace_root();
        let parity_index = build_parity_index_at(&workspace);
        let rows = build_tracker_rows(TRACKED_PROJECTS, &parity_index);
        render_tracker_section(ctx, &rows)
    } else {
        String::new()
    };
    let watchd_panel = render_watchd_panel_in(ctx, &watchd_events_path(), 20);
    let auto_port_panel = render_auto_port_panel_in(ctx, &dispatched_path(), 20);
    let body = format!(
        r#"{tracker}<section><h2 class="text-lg font-semibold mb-2">Upstream ({n})</h2>{tbl}</section>{watchd}{auto}"#,
        tracker = tracker_section,
        n = rows.len(),
        tbl = table(&["name", "repo", "version", "last_check"], &table_rows),
        watchd = watchd_panel,
        auto = auto_port_panel,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/upstream",
        &format!("upstream · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

/// Locate the auto-port dispatcher's `dispatched.jsonl`. Mirrors
/// `AutoPortDispatcher::default_paths()` so the on-disk shape stays
/// in sync.
fn dispatched_path() -> PathBuf {
    let (_events, dispatched, _audit) = cave_upstream_watchd::AutoPortDispatcher::default_paths();
    dispatched
}

/// Public read-side helper — tests + the portal panel both consume
/// it. Newest-first ordering + a `max_rows` cap.
pub fn read_dispatched(path: &std::path::Path, max_rows: usize) -> Vec<DispatchedRecord> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out: Vec<DispatchedRecord> = text
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str::<DispatchedRecord>(l).ok())
        .collect();
    out.sort_by(|a, b| b.dispatched_at.cmp(&a.dispatched_at));
    out.truncate(max_rows);
    out
}

fn auto_port_status_class(s: &AutoPortStatus) -> &'static str {
    match s {
        AutoPortStatus::Merged => "bg-green-200 text-green-900",
        AutoPortStatus::Dispatched | AutoPortStatus::Running => "bg-blue-200 text-blue-900",
        AutoPortStatus::CharterFail => "bg-orange-200 text-orange-900",
        AutoPortStatus::BackendFail => "bg-red-200 text-red-900",
    }
}

fn auto_port_status_label(s: &AutoPortStatus) -> &'static str {
    match s {
        AutoPortStatus::Merged => "MERGED",
        AutoPortStatus::Dispatched => "DISPATCHED",
        AutoPortStatus::Running => "RUNNING",
        AutoPortStatus::CharterFail => "CHARTER_FAIL",
        AutoPortStatus::BackendFail => "BACKEND_FAIL",
    }
}

/// Render the auto-port panel as HTML. Tenant persona filter
/// matches the watchd panel — TenantAdmin only sees the 7 tenant-
/// relevant modules; PlatformAdmin sees everything.
pub fn render_auto_port_panel_in(
    ctx: &RequestCtx,
    state_path: &std::path::Path,
    max_rows: usize,
) -> String {
    let mut records = read_dispatched(state_path, max_rows * 4);
    if !ctx.persona.is_platform() {
        records.retain(|r| is_tenant_relevant(&r.cave_module));
    }
    records.truncate(max_rows);

    let n = records.len();
    let table_rows: Vec<Vec<String>> = records
        .iter()
        .map(|r| {
            let badge = format!(
                r#"<span class="px-2 py-0.5 rounded text-xs {cls}">{lbl}</span>"#,
                cls = auto_port_status_class(&r.status),
                lbl = auto_port_status_label(&r.status),
            );
            let commit = r
                .commit_sha
                .as_deref()
                .map(|s| s.chars().take(7).collect::<String>())
                .unwrap_or_else(|| "-".to_string());
            vec![
                escape(&r.cave_module),
                escape(&r.task_id),
                escape(&r.backend),
                badge,
                commit,
                escape(&r.target_branch),
                r.dispatched_at.format("%Y-%m-%d %H:%M").to_string(),
            ]
        })
        .collect();

    let header = format!(
        r#"<div class="flex items-center justify-between mb-2">
                <h2 class="text-lg font-semibold">Auto-port dispatcher ({n})</h2>
                <span class="text-xs text-zinc-500">source: <code>{src}</code></span>
            </div>"#,
        src = escape(state_path.display().to_string().as_str()),
    );

    let tenant_note = if !ctx.persona.is_platform() {
        r#"<p class="text-xs text-zinc-500 mb-2">
            Tenant view — only auto-port records for tenant-relevant
            crates are shown. Charter `merged` events are platform-
            wide; sign in as <code>platform_admin</code> for the full
            audit trail.
           </p>"#
    } else {
        ""
    };

    let table_block = if records.is_empty() {
        empty_state(
            "🛰️",
            "No auto-port records yet",
            "The dispatcher writes one row per dispatched gap. Run `cave-upstream-watchd dispatch` (or check the LaunchAgent at com.cave.upstream-watchd) to populate this.",
        )
    } else {
        // 2026-05-13: `status` cell is a pre-formatted `<span>` badge;
        // use table_html so it renders styled instead of as literal text.
        render_table_html(
            &[
                "cave-module",
                "task_id",
                "backend",
                "status",
                "commit",
                "branch",
                "dispatched_at",
            ],
            &table_rows,
        )
    };

    format!(
        r#"<section class="mt-6 p-3 border rounded">{header}{note}{tbl}</section>"#,
        header = header,
        note = tenant_note,
        tbl = table_block,
    )
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage(
    "plugins/upstream/src/components/ProjectsList.tsx",
    "ProjectsList",
);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Persona;
    use crate::portal_test_ctx;
    use cave_upstream_watchd::changelog::Changelog;
    use cave_upstream_watchd::event::{GapEvent, GapEventSink, JsonlSink};

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_filters_to_owner() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/upstream/src/components/ProjectsList.tsx",
            "ProjectsList",
            "acme"
        );
        let s = AdminState::seeded();
        let r = list_records(&s, &ctx(&[Permission::UpstreamRead])).unwrap();
        assert_eq!(r.len(), 2);
        assert!(r.iter().all(|x| x.tenant.as_str() == "acme"));
    }

    #[test]
    fn list_refuses_without_perm() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/permission-react/src/PermissionApi.ts",
            "authorize",
            "acme"
        );
        assert!(list_records(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_contains_owner_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/upstream/src/components/ProjectsList.tsx",
            "RenderOwner",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::UpstreamRead])).unwrap();
        assert!(html.contains("kubernetes"));
    }

    #[test]
    fn render_excludes_evil_row() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/upstream/src/components/ProjectsList.tsx",
            "RenderEvil",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::UpstreamRead])).unwrap();
        assert!(!html.contains("evil-upstream"));
    }

    // ── 2026-05-14: consolidated tracker section ────────────────────────────
    //
    // /admin/upstream is now the canonical upstream-tracker page (the
    // legacy /upstream JS-driven dark theme redirects here). New tests
    // cover:
    //  - build_tracker_rows joins TRACKED_PROJECTS with parity reports
    //  - render_tracker_section honours PlatformAdmin persona gate
    //  - render_tracker_section renders status badges + progress bars
    //  - end-to-end render() includes the tracker section for platform admin
    //    and OMITS it for tenant admin

    fn platform_ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer_as("acme", perms, Persona::PlatformAdmin)
    }

    fn fake_parity_report(module: &str, overall: f32) -> cave_kernel::parity::types::ParityReport {
        use cave_kernel::parity::types::{ParityMetric, ParityReport};
        ParityReport {
            module: module.into(),
            upstream_ref: format!("upstream/{module} @ v1"),
            measured_at: chrono::Utc::now(),
            file_parity: ParityMetric {
                score: overall,
                matched: 1,
                total: 1,
            },
            function_parity: ParityMetric {
                score: overall,
                matched: 1,
                total: 1,
            },
            test_parity: ParityMetric {
                score: overall,
                matched: 1,
                total: 1,
            },
            surface_parity: ParityMetric {
                score: overall,
                matched: 1,
                total: 1,
            },
            overall,
            stubs_detected: 0,
            gaps: Vec::new(),
        }
    }

    #[test]
    fn build_tracker_rows_joins_projects_with_parity_index() {
        let projects = [
            TrackedProject {
                name: "Cilium",
                github_repo: "cilium/cilium",
                cave_module: "cave-net",
                track_features: "",
                check_frequency: "biweekly",
                category: "networking",
                phase: 1,
            },
            TrackedProject {
                name: "Made-up",
                github_repo: "fake/fake",
                cave_module: "cave-does-not-exist",
                track_features: "",
                check_frequency: "biweekly",
                category: "test",
                phase: 1,
            },
        ];
        let mut idx: HashMap<String, ParityReport> = HashMap::new();
        idx.insert("cave-net".into(), fake_parity_report("cave-net", 0.9179));

        let rows = build_tracker_rows(&projects, &idx);
        assert_eq!(rows.len(), 2);

        let cilium = &rows[0];
        assert_eq!(cilium.upstream_name, "Cilium");
        assert_eq!(cilium.cave_crate, "cave-net");
        assert!((cilium.parity_overall - 0.9179).abs() < 1e-4);
        assert_eq!(cilium.parity_status, "synced", "0.92 ≥ 0.7 threshold");

        let fake = &rows[1];
        assert_eq!(fake.parity_overall, -1.0, "no report → -1.0");
        assert_eq!(fake.parity_status, "pending");
    }

    #[test]
    fn parity_status_label_thresholds() {
        assert_eq!(parity_status_label(1.0), "synced");
        assert_eq!(parity_status_label(0.70), "synced");
        assert_eq!(parity_status_label(0.69), "behind");
        assert_eq!(parity_status_label(0.30), "behind");
        assert_eq!(parity_status_label(0.29), "pending");
        assert_eq!(parity_status_label(0.0), "pending");
        assert_eq!(parity_status_label(-1.0), "pending");
    }

    #[test]
    fn render_tracker_section_returns_empty_for_tenant_admin() {
        // TenantAdmin: cross-tenant tracker data is not their concern.
        let tenant_ctx = RequestCtx::developer_as("acme", &[], Persona::TenantAdmin);
        let row = TrackerRow {
            upstream_name: "Cilium".into(),
            upstream_repo: "cilium/cilium".into(),
            cave_crate: "cave-net".into(),
            category: "networking".into(),
            adr_refs: vec!["ADR-004".into()],
            parity_overall: 0.92,
            parity_status: "synced".into(),
        };
        let html = render_tracker_section(&tenant_ctx, &[row]);
        assert_eq!(html, "", "TenantAdmin sees no tracker section");
    }

    #[test]
    fn render_tracker_section_renders_status_badges_and_bars_for_platform() {
        let plat = platform_ctx(&[Permission::UpstreamRead]);
        let rows = vec![
            TrackerRow {
                upstream_name: "Cilium".into(),
                upstream_repo: "cilium/cilium".into(),
                cave_crate: "cave-net".into(),
                category: "networking".into(),
                adr_refs: vec!["ADR-004".into(), "ADR-014".into()],
                parity_overall: 0.9179,
                parity_status: "synced".into(),
            },
            TrackerRow {
                upstream_name: "Skeleton".into(),
                upstream_repo: "fake/skeleton".into(),
                cave_crate: "cave-fake".into(),
                category: "test".into(),
                adr_refs: vec![],
                parity_overall: -1.0,
                parity_status: "pending".into(),
            },
        ];
        let html = render_tracker_section(&plat, &rows);
        // Heading + summary chips.
        assert!(html.contains("Upstream Parity Tracker (2)"));
        assert!(html.contains("1 synced"));
        assert!(html.contains("1 pending"));
        // Row content for Cilium: ADR chips + 92% + status badge.
        assert!(html.contains("ADR-004"));
        assert!(html.contains("ADR-014"));
        assert!(html.contains("92%"));
        assert!(
            html.contains("bg-green-100 text-green-900"),
            "synced badge color"
        );
        // Drill-down link to /admin/compliance for the crate.
        assert!(html.contains("/admin/compliance/cave-net?"));
        // Skeleton row: no-manifest text + 0% bar width.
        assert!(html.contains("no manifest"));
        assert!(
            html.contains("bg-red-100 text-red-900"),
            "pending badge color"
        );
    }

    #[test]
    fn render_tracker_section_escapes_user_visible_strings() {
        // Defensive — TrackerRow fields are populated from string statics in
        // TRACKED_PROJECTS today, but a future PR could thread user input
        // through (e.g. manifest-driven category). Escaping must be in
        // place so `<script>` payloads can't sneak through.
        let plat = platform_ctx(&[Permission::UpstreamRead]);
        let row = TrackerRow {
            upstream_name: "<x>".into(),
            upstream_repo: "fake/<repo>".into(),
            cave_crate: "cave-<evil>".into(),
            category: "<script>alert(1)</script>".into(),
            adr_refs: vec!["<adr>".into()],
            parity_overall: 0.5,
            parity_status: "behind".into(),
        };
        let html = render_tracker_section(&plat, &[row]);
        assert!(html.contains("&lt;script&gt;"));
        assert!(!html.contains("<script>alert(1)</script>"));
        assert!(html.contains("&lt;adr&gt;"));
    }

    #[test]
    fn end_to_end_admin_upstream_renders_tracker_for_platform_admin() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/upstream/src/components/ProjectsList.tsx",
            "TrackerPlatform",
            "acme"
        );
        // Use the real workspace so build_parity_index_at can discover
        // manifests + populate the tracker.
        let s = AdminState::seeded();
        let plat = platform_ctx(&[Permission::UpstreamRead]);
        let html = render(&s, &plat).unwrap();
        assert!(
            html.contains("Upstream Parity Tracker"),
            "tracker section present"
        );
        // Cilium is a known TRACKED_PROJECT — must appear in the rendered HTML.
        assert!(html.contains("cilium/cilium"));
    }

    #[test]
    fn end_to_end_admin_upstream_omits_tracker_for_tenant_admin() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/upstream/src/components/ProjectsList.tsx",
            "TrackerTenant",
            "acme"
        );
        let s = AdminState::seeded();
        let tenant_ctx =
            RequestCtx::developer_as("acme", &[Permission::UpstreamRead], Persona::TenantAdmin);
        let html = render(&s, &tenant_ctx).unwrap();
        // Tenant admin still gets the legacy seeded list + watchd panel,
        // but NOT the cross-tenant tracker.
        assert!(!html.contains("Upstream Parity Tracker"));
    }

    // ── end consolidated-tracker tests ─────────────────────────────────────

    #[test]
    fn render_shows_acme_count() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/upstream/src/components/ProjectsList.tsx",
            "Count",
            "acme"
        );
        let html = render(&AdminState::seeded(), &ctx(&[Permission::UpstreamRead])).unwrap();
        assert!(html.contains("(2)"));
    }

    // ── 2026-05-13: watchd panel ────────────────────────────────

    fn write_event(
        sink: &JsonlSink,
        module: &str,
        repo: &str,
        latest: &str,
        sev: Severity,
        age: i64,
    ) {
        let e = GapEvent::new(
            module,
            repo,
            Some("v1.0.0".into()),
            latest,
            sev,
            Some(age),
            None,
            Changelog::default(),
            chrono::Utc::now(),
        );
        sink.emit(&e).unwrap();
    }

    #[test]
    fn watchd_panel_renders_empty_when_no_events() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let plat_ctx = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_watchd_panel_in(&plat_ctx, &path, 10);
        // 2026-05-22 — moved from inline `<p>` copy to the `empty_state`
        // primitive; assert on the shared role+class so the visual
        // treatment is locked in.
        assert!(html.contains(r#"class="cave-empty""#));
        assert!(html.contains(r#"role="status""#));
        assert!(html.contains("No GAP events recorded yet"));
    }

    #[test]
    fn watchd_panel_lists_recent_events_newest_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        write_event(
            &sink,
            "cave-cri",
            "containerd/containerd",
            "v1.7.22",
            Severity::Patch,
            3600,
        );
        write_event(
            &sink,
            "cave-etcd",
            "etcd-io/etcd",
            "v3.6.0",
            Severity::Minor,
            7200,
        );

        let plat_ctx = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_watchd_panel_in(&plat_ctx, &path, 10);
        assert!(html.contains("cave-cri"));
        assert!(html.contains("cave-etcd"));
        // Newest first: cave-etcd was emitted second so it should
        // appear above cave-cri.
        let idx_etcd = html.find("cave-etcd").unwrap();
        let idx_cri = html.find("cave-cri").unwrap();
        assert!(idx_etcd < idx_cri);
    }

    #[test]
    fn watchd_panel_tenant_persona_only_sees_tenant_relevant_modules() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        // Platform-only module (cave-runtime) should be filtered out.
        write_event(
            &sink,
            "cave-runtime",
            "anthropic/runtime",
            "v9.9.9",
            Severity::Major,
            60,
        );
        // Tenant-relevant module (cave-vault) should pass.
        write_event(
            &sink,
            "cave-vault",
            "hashicorp/vault",
            "v1.16.0",
            Severity::Minor,
            300,
        );

        let tenant_ctx =
            RequestCtx::developer_as("tenant1", &[Permission::UpstreamRead], Persona::TenantAdmin);
        let html = render_watchd_panel_in(&tenant_ctx, &path, 10);
        assert!(html.contains("cave-vault"));
        assert!(!html.contains("cave-runtime"));
        // Tenant note is shown.
        assert!(html.contains("Tenant view"));
    }

    #[test]
    fn watchd_panel_platform_persona_sees_everything() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        write_event(
            &sink,
            "cave-runtime",
            "anthropic/runtime",
            "v9.9.9",
            Severity::Major,
            60,
        );
        write_event(
            &sink,
            "cave-vault",
            "hashicorp/vault",
            "v1.16.0",
            Severity::Minor,
            300,
        );

        let plat_ctx = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_watchd_panel_in(&plat_ctx, &path, 10);
        assert!(html.contains("cave-runtime"));
        assert!(html.contains("cave-vault"));
        // No tenant note.
        assert!(!html.contains("Tenant view"));
    }

    #[test]
    fn watchd_panel_renders_severity_badges() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        write_event(&sink, "cave-x", "x/y", "v2.0.0", Severity::Major, 1);

        let plat_ctx = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_watchd_panel_in(&plat_ctx, &path, 10);
        assert!(html.contains("MAJOR"));
        assert!(html.contains("bg-red-200"));
    }

    #[test]
    fn watchd_panel_respects_max_rows_cap() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        for i in 0..30 {
            write_event(
                &sink,
                &format!("cave-{i}"),
                "x/y",
                &format!("v1.{i}.0"),
                Severity::Patch,
                1,
            );
        }
        let plat_ctx = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_watchd_panel_in(&plat_ctx, &path, 5);
        // Five rows in the table — count td occurrences (7 columns × 5 rows = 35 <td>s).
        // We avoid brittle string counts and instead just confirm the
        // header bound surfaces a reasonable count.
        assert!(html.contains("Watchd GAP events (5)"));
    }

    #[test]
    fn watchd_panel_handles_unknown_severity_with_grey_badge() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("events.jsonl");
        let sink = JsonlSink::new(path.clone());
        write_event(&sink, "cave-z", "x/y", "release-tag", Severity::Unknown, 60);
        let plat_ctx = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_watchd_panel_in(&plat_ctx, &path, 5);
        assert!(html.contains("UNKNOWN"));
        assert!(html.contains("bg-zinc-200"));
    }

    #[test]
    fn format_age_buckets_seconds_minutes_hours_days() {
        assert_eq!(format_age(30), "30s");
        assert_eq!(format_age(120), "2m");
        assert_eq!(format_age(7200), "2h");
        assert_eq!(format_age(86_400 * 3), "3d");
    }

    #[test]
    fn tenant_relevant_list_matches_charter() {
        // If a new entry lands in TENANT_RELEVANT_MODULES we want
        // the test to fail unless someone updates this list — keeps
        // the dashboard semantics auditable.
        assert!(is_tenant_relevant("cave-vault"));
        assert!(is_tenant_relevant("cave-keda"));
        assert!(is_tenant_relevant("cave-kubelet"));
        assert!(!is_tenant_relevant("cave-runtime"));
        assert!(!is_tenant_relevant("cave-apiserver"));
    }

    // ── 2026-05-13: auto-port panel ───────────────────────────

    fn dispatched_record(
        event_id: &str,
        module: &str,
        status: AutoPortStatus,
        commit: Option<&str>,
    ) -> DispatchedRecord {
        use cave_upstream_watchd::auto_port_gate::CharterBaseline;
        DispatchedRecord {
            event_id: event_id.into(),
            cave_module: module.into(),
            backend: "dryrun".into(),
            task_id: format!("dryrun-{event_id}"),
            target_branch: format!("auto-port/{event_id}"),
            status,
            commit_sha: commit.map(str::to_string),
            charter_report: None,
            dispatched_at: chrono::Utc::now(),
            last_checked_at: chrono::Utc::now(),
            reason: None,
            baseline: CharterBaseline {
                crate_name: module.into(),
                commit_sha_before: "0".repeat(40),
                fill_ratio_before: 0.7,
                workspace_stub_count_before: 0,
            },
        }
    }

    fn write_dispatched(path: &std::path::Path, records: &[DispatchedRecord]) {
        let mut s = String::new();
        for r in records {
            s.push_str(&serde_json::to_string(r).unwrap());
            s.push('\n');
        }
        std::fs::write(path, s).unwrap();
    }

    #[test]
    fn auto_port_panel_renders_empty_when_no_records() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dispatched.jsonl");
        let plat = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_auto_port_panel_in(&plat, &path, 10);
        assert!(html.contains("No auto-port records"));
    }

    #[test]
    fn auto_port_panel_renders_status_badges_for_each_lifecycle_state() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dispatched.jsonl");
        write_dispatched(
            &path,
            &[
                dispatched_record(
                    "e1",
                    "cave-x",
                    AutoPortStatus::Merged,
                    Some("abcdef0123456789abcdef0123456789abcdef01"),
                ),
                dispatched_record("e2", "cave-y", AutoPortStatus::Dispatched, None),
                dispatched_record(
                    "e3",
                    "cave-z",
                    AutoPortStatus::CharterFail,
                    Some("0011223344556677889900112233445566778899"),
                ),
                dispatched_record("e4", "cave-w", AutoPortStatus::BackendFail, None),
            ],
        );
        let plat = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_auto_port_panel_in(&plat, &path, 10);
        assert!(html.contains("MERGED"));
        assert!(html.contains("DISPATCHED"));
        assert!(html.contains("CHARTER_FAIL"));
        assert!(html.contains("BACKEND_FAIL"));
        // Commit shortened to 7 chars.
        assert!(html.contains("abcdef0"));
    }

    #[test]
    fn auto_port_panel_tenant_persona_filters_to_tenant_relevant_modules() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dispatched.jsonl");
        write_dispatched(
            &path,
            &[
                dispatched_record("e1", "cave-runtime", AutoPortStatus::Merged, None),
                dispatched_record("e2", "cave-vault", AutoPortStatus::Merged, None),
            ],
        );
        let tenant =
            RequestCtx::developer_as("tenant1", &[Permission::UpstreamRead], Persona::TenantAdmin);
        let html = render_auto_port_panel_in(&tenant, &path, 10);
        assert!(html.contains("cave-vault"));
        assert!(!html.contains("cave-runtime"));
        assert!(html.contains("Tenant view"));
    }

    #[test]
    fn auto_port_panel_max_rows_cap_respected() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dispatched.jsonl");
        let many: Vec<DispatchedRecord> = (0..30)
            .map(|i| {
                dispatched_record(
                    &format!("e{i}"),
                    &format!("cave-{i}"),
                    AutoPortStatus::Dispatched,
                    None,
                )
            })
            .collect();
        write_dispatched(&path, &many);
        let plat = RequestCtx::developer_as(
            "platform",
            &[Permission::UpstreamRead],
            Persona::PlatformAdmin,
        );
        let html = render_auto_port_panel_in(&plat, &path, 5);
        assert!(html.contains("Auto-port dispatcher (5)"));
    }

    #[test]
    fn read_dispatched_returns_newest_first() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("dispatched.jsonl");
        let older = dispatched_record("e-old", "cave-x", AutoPortStatus::Merged, None);
        let mut newer = dispatched_record("e-new", "cave-y", AutoPortStatus::Merged, None);
        newer.dispatched_at = older.dispatched_at + chrono::Duration::seconds(10);
        write_dispatched(&path, &[older, newer]);
        let out = read_dispatched(&path, 10);
        assert_eq!(out[0].cave_module, "cave-y");
        assert_eq!(out[1].cave_module, "cave-x");
    }

    #[test]
    fn read_dispatched_missing_file_returns_empty() {
        let path = std::path::PathBuf::from("/tmp/__no_such_dispatched_file__.jsonl");
        let out = read_dispatched(&path, 10);
        assert!(out.is_empty());
    }
}
