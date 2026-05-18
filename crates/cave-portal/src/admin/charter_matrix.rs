//! `/admin/_audit` — Charter v2 per-crate matrix.
//!
//! Five-axis grade roll-up (see [`meta_audit`]) tells you HOW the
//! workspace is doing in aggregate. The Charter v2 matrix tells you
//! WHICH CRATE is keeping the workspace honest and which is dragging
//! it down, rule-by-rule. The eight rules (in canonical column order
//! on the card) are:
//!
//!   1. **TDD** — has tests, no `#[ignore = "impl pending"]` markers
//!   2. **SPDX** — every `.rs` file in `src/**` carries an
//!      `SPDX-License-Identifier:` header
//!   3. **source-stamp** — manifest `[upstream]` block has
//!      `org`, `repo`, `version` non-empty
//!   4. **no-stub** — `unimplemented!()` + `todo!()` counts both 0
//!   5. **no-backcompat** — no `#[deprecated]` attribute anywhere in
//!      `src/**`
//!   6. **always-latest** — `parity.manifest.toml::last_audit` is at
//!      most 90 days old
//!   7. **4-track** — Portal + cavectl + alerts + dashboard all present
//!   8. **honest** — manifest declares `honest_ratio` + at least one
//!      filled section
//!
//! Each rule resolves to PASS / FAIL / N/A per crate; the matrix view
//! is a sortable + filterable grid of those eight pills plus
//! fill_ratio / test count / last audit / manifest pin / last commit.

use crate::admin::compliance::{CommitRow, ComplianceSnapshot, CrateCompliance};
use crate::admin::render::escape;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

// ── Charter v2 rule model ────────────────────────────────────────────

/// One of the eight Charter v2 rules. The variant order is canonical
/// — it matches the column order on the dashboard card and the array
/// indices into [`CrateCharter::verdicts`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CharterRule {
    Tdd,
    Spdx,
    SourceStamp,
    NoStub,
    NoBackcompat,
    AlwaysLatest,
    FourTrack,
    Honest,
}

impl CharterRule {
    pub const ALL: [CharterRule; 8] = [
        CharterRule::Tdd,
        CharterRule::Spdx,
        CharterRule::SourceStamp,
        CharterRule::NoStub,
        CharterRule::NoBackcompat,
        CharterRule::AlwaysLatest,
        CharterRule::FourTrack,
        CharterRule::Honest,
    ];

    pub fn slug(&self) -> &'static str {
        match self {
            CharterRule::Tdd => "tdd",
            CharterRule::Spdx => "spdx",
            CharterRule::SourceStamp => "source_stamp",
            CharterRule::NoStub => "no_stub",
            CharterRule::NoBackcompat => "no_backcompat",
            CharterRule::AlwaysLatest => "always_latest",
            CharterRule::FourTrack => "four_track",
            CharterRule::Honest => "honest",
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            CharterRule::Tdd => "TDD",
            CharterRule::Spdx => "SPDX",
            CharterRule::SourceStamp => "src-stamp",
            CharterRule::NoStub => "no-stub",
            CharterRule::NoBackcompat => "no-backcompat",
            CharterRule::AlwaysLatest => "always-latest",
            CharterRule::FourTrack => "4-track",
            CharterRule::Honest => "honest",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Pass,
    Fail,
    Na,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrateScan {
    pub src_rs_files: u32,
    pub spdx_marked_files: u32,
    pub deprecated_attr_count: u32,
}

impl CrateScan {
    pub fn empty() -> Self {
        Self {
            src_rs_files: 0,
            spdx_marked_files: 0,
            deprecated_attr_count: 0,
        }
    }
}

// ── Per-crate row ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CrateCharter {
    pub name: String,
    pub infra_only: bool,
    pub fill_ratio: Option<f64>,
    pub fill_ratio_source: Option<String>,
    pub honest_ratio: Option<f64>,
    pub test_count: u32,
    pub last_audit: Option<String>,
    pub manifest_pin: Option<String>,
    pub last_commit: Option<CommitRow>,
    pub verdicts: [Verdict; 8],
}

impl CrateCharter {
    pub fn pass_count(&self) -> u32 {
        self.verdicts.iter().filter(|v| **v == Verdict::Pass).count() as u32
    }
    pub fn fail_count(&self) -> u32 {
        self.verdicts.iter().filter(|v| **v == Verdict::Fail).count() as u32
    }
    pub fn verdict_for(&self, rule: CharterRule) -> Verdict {
        let idx = CharterRule::ALL
            .iter()
            .position(|r| *r == rule)
            .expect("CharterRule::ALL covers every variant");
        self.verdicts[idx]
    }
}

// ── Filter / sort ────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FilterMode {
    All,
    AnyFailing,
    Tier1,
    InfraOnly,
}

impl FilterMode {
    pub fn parse(s: &str) -> Self {
        match s {
            "any_failing" | "failing" => FilterMode::AnyFailing,
            "tier1" => FilterMode::Tier1,
            "infra" | "infra_only" => FilterMode::InfraOnly,
            _ => FilterMode::All,
        }
    }
    pub fn slug(&self) -> &'static str {
        match self {
            FilterMode::All => "all",
            FilterMode::AnyFailing => "any_failing",
            FilterMode::Tier1 => "tier1",
            FilterMode::InfraOnly => "infra",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SortKey {
    Name,
    FillRatio,
    TestCount,
    LastAudit,
    PassCount,
}

impl SortKey {
    pub fn parse(s: &str) -> Self {
        match s {
            "fill_ratio" | "ratio" => SortKey::FillRatio,
            "test_count" | "tests" => SortKey::TestCount,
            "last_audit" | "audit" => SortKey::LastAudit,
            "pass_count" | "passes" => SortKey::PassCount,
            _ => SortKey::Name,
        }
    }
    pub fn slug(&self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::FillRatio => "fill_ratio",
            SortKey::TestCount => "test_count",
            SortKey::LastAudit => "last_audit",
            SortKey::PassCount => "pass_count",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CharterMatrix {
    pub rows: Vec<CrateCharter>,
    pub total_crates: u32,
    pub crates_with_any_fail: u32,
    pub rule_pass_counts: [u32; 8],
    pub rule_fail_counts: [u32; 8],
    pub rule_na_counts: [u32; 8],
    pub filter: FilterMode,
    pub sort: SortKey,
}

// ── Public API ───────────────────────────────────────────────────────

/// Evaluate the eight Charter v2 rules for one crate. Pure function:
/// every signal it needs is in `c` (compliance row) and `scan` (I/O).
/// `today` is injected so the always-latest window is testable.
pub fn evaluate(c: &CrateCompliance, scan: &CrateScan, today: NaiveDate) -> [Verdict; 8] {
    [
        evaluate_tdd(c),
        evaluate_spdx(scan),
        evaluate_source_stamp(c),
        evaluate_no_stub(c),
        evaluate_no_backcompat(scan),
        evaluate_always_latest(c, today),
        evaluate_four_track(c),
        evaluate_honest(c),
    ]
}

fn evaluate_tdd(c: &CrateCompliance) -> Verdict {
    if c.infra_only && c.backend_test_count == 0 {
        return Verdict::Na;
    }
    if c.backend_test_count > 0 && c.ignored_test_count == 0 {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

fn evaluate_spdx(scan: &CrateScan) -> Verdict {
    if scan.src_rs_files == 0 {
        return Verdict::Na;
    }
    if scan.spdx_marked_files == scan.src_rs_files {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

fn evaluate_source_stamp(c: &CrateCompliance) -> Verdict {
    if c.infra_only {
        return Verdict::Na;
    }
    match (&c.upstream_org_repo, &c.upstream_version) {
        (Some(slug), Some(ver))
            if !slug.trim().is_empty()
                && slug.contains('/')
                && !ver.trim().is_empty() =>
        {
            Verdict::Pass
        }
        _ => Verdict::Fail,
    }
}

fn evaluate_no_stub(c: &CrateCompliance) -> Verdict {
    if c.unimplemented_count == 0 && c.todo_count == 0 {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

fn evaluate_no_backcompat(scan: &CrateScan) -> Verdict {
    if scan.deprecated_attr_count == 0 {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

fn evaluate_always_latest(c: &CrateCompliance, today: NaiveDate) -> Verdict {
    let Some(raw) = c.parity_ratio_last_audit.as_deref() else {
        return Verdict::Na;
    };
    let Ok(d) = NaiveDate::parse_from_str(raw, "%Y-%m-%d") else {
        return Verdict::Fail;
    };
    let age_days = today.signed_duration_since(d).num_days();
    if age_days <= 90 && age_days >= 0 {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

fn evaluate_four_track(c: &CrateCompliance) -> Verdict {
    if c.infra_only {
        return Verdict::Na;
    }
    if c.portal_admin_present
        && c.cavectl_subcommand_present
        && c.obs_alerts_present
        && c.obs_dashboard_present
    {
        Verdict::Pass
    } else {
        Verdict::Fail
    }
}

fn evaluate_honest(c: &CrateCompliance) -> Verdict {
    if c.infra_only {
        return Verdict::Na;
    }
    match (c.honest_parity_ratio, c.manifest_filled) {
        (Some(_), Some(true)) => Verdict::Pass,
        _ => Verdict::Fail,
    }
}

/// Build the matrix from a compliance snapshot plus per-crate scan +
/// last-commit lookups. `scans` and `last_commits` are keyed by crate
/// name; missing entries fall back to [`CrateScan::empty`] / `None`.
pub fn build_matrix(
    snap: &ComplianceSnapshot,
    today: NaiveDate,
    scans: &std::collections::BTreeMap<String, CrateScan>,
    last_commits: &std::collections::BTreeMap<String, CommitRow>,
    filter: FilterMode,
    sort: SortKey,
) -> CharterMatrix {
    let rows_unfiltered: Vec<CrateCharter> = snap
        .crates
        .iter()
        .map(|c| {
            let scan = scans.get(&c.name).copied().unwrap_or_else(CrateScan::empty);
            let verdicts = evaluate(c, &scan, today);
            let manifest_pin = match (&c.upstream_org_repo, &c.upstream_version) {
                (Some(slug), Some(ver))
                    if !slug.trim().is_empty() && !ver.trim().is_empty() =>
                {
                    Some(format!("{slug} @ {ver}"))
                }
                _ => None,
            };
            CrateCharter {
                name: c.name.clone(),
                infra_only: c.infra_only,
                fill_ratio: c.parity_ratio,
                fill_ratio_source: c.parity_ratio_source.clone(),
                honest_ratio: c.honest_parity_ratio,
                test_count: c.backend_test_count,
                last_audit: c.parity_ratio_last_audit.clone(),
                manifest_pin,
                last_commit: last_commits.get(&c.name).cloned(),
                verdicts,
            }
        })
        .collect();

    let total_crates = rows_unfiltered.len() as u32;
    let crates_with_any_fail = rows_unfiltered
        .iter()
        .filter(|r| r.fail_count() > 0)
        .count() as u32;

    let mut rule_pass_counts = [0u32; 8];
    let mut rule_fail_counts = [0u32; 8];
    let mut rule_na_counts = [0u32; 8];
    for r in &rows_unfiltered {
        for (i, v) in r.verdicts.iter().enumerate() {
            match v {
                Verdict::Pass => rule_pass_counts[i] += 1,
                Verdict::Fail => rule_fail_counts[i] += 1,
                Verdict::Na => rule_na_counts[i] += 1,
            }
        }
    }

    let mut rows: Vec<CrateCharter> = rows_unfiltered
        .into_iter()
        .filter(|r| match filter {
            FilterMode::All => true,
            FilterMode::AnyFailing => r.fail_count() > 0,
            FilterMode::Tier1 => !r.infra_only,
            FilterMode::InfraOnly => r.infra_only,
        })
        .collect();

    match sort {
        SortKey::Name => rows.sort_by(|a, b| a.name.cmp(&b.name)),
        SortKey::FillRatio => rows.sort_by(|a, b| match (a.fill_ratio, b.fill_ratio) {
            (Some(av), Some(bv)) => bv
                .partial_cmp(&av)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.name.cmp(&b.name)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        }),
        SortKey::TestCount => rows.sort_by(|a, b| {
            b.test_count
                .cmp(&a.test_count)
                .then(a.name.cmp(&b.name))
        }),
        SortKey::LastAudit => rows.sort_by(|a, b| match (&a.last_audit, &b.last_audit) {
            (Some(av), Some(bv)) => bv.cmp(av).then(a.name.cmp(&b.name)),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            (None, None) => a.name.cmp(&b.name),
        }),
        SortKey::PassCount => rows.sort_by(|a, b| {
            b.pass_count()
                .cmp(&a.pass_count())
                .then(a.name.cmp(&b.name))
        }),
    }

    CharterMatrix {
        rows,
        total_crates,
        crates_with_any_fail,
        rule_pass_counts,
        rule_fail_counts,
        rule_na_counts,
        filter,
        sort,
    }
}

/// Scan one crate's `src/` for SPDX coverage + `#[deprecated]` count.
pub fn scan_crate_io(workspace_root: &std::path::Path, crate_name: &str) -> CrateScan {
    let src = workspace_root.join("crates").join(crate_name).join("src");
    if !src.is_dir() {
        return CrateScan::empty();
    }
    let mut s = CrateScan::empty();
    for f in walk_rs_files(&src) {
        s.src_rs_files += 1;
        let Ok(text) = std::fs::read_to_string(&f) else {
            continue;
        };
        let first_lines: String = text.lines().take(8).collect::<Vec<_>>().join("\n");
        if first_lines.contains("SPDX-License-Identifier:") {
            s.spdx_marked_files += 1;
        }
        for line in text.lines() {
            let t = line.trim_start();
            if t.starts_with("#[deprecated") {
                s.deprecated_attr_count += 1;
            }
        }
    }
    s
}

fn walk_rs_files(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    fn inner(p: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(read) = std::fs::read_dir(p) else { return };
        for ent in read.flatten() {
            let path = ent.path();
            if path.is_dir() {
                inner(&path, out);
            } else if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    inner(root, &mut out);
    out
}

// ── HTML render ──────────────────────────────────────────────────────

/// Render the matrix as a standalone HTML section. Designed to be
/// stitched below the five-axis summary on `/admin/_audit`.
pub fn render_section(m: &CharterMatrix, tenant_id: &str) -> String {
    let header = render_header(m, tenant_id);
    let table = render_table(m);
    format!(
        r#"<section aria-labelledby="charter-matrix-heading" class="mt-6">
  <header class="mb-3 flex items-baseline justify-between flex-wrap gap-2">
    <h2 id="charter-matrix-heading" class="text-lg font-semibold">Charter v2 matrix</h2>
    <p class="text-xs text-zinc-500 dark:text-zinc-400">
      {n_show} of {n_total} crates · {n_fail} with at least one FAIL
    </p>
  </header>
  {header}
  {table}
</section>"#,
        n_show = m.rows.len(),
        n_total = m.total_crates,
        n_fail = m.crates_with_any_fail,
        header = header,
        table = table,
    )
}

fn render_header(m: &CharterMatrix, tenant_id: &str) -> String {
    let pills = CharterRule::ALL
        .iter()
        .enumerate()
        .map(|(i, rule)| {
            let pass = m.rule_pass_counts[i];
            let fail = m.rule_fail_counts[i];
            let na = m.rule_na_counts[i];
            format!(
                r#"<div class="rounded border border-zinc-200 dark:border-zinc-800 px-2 py-1 text-xs" aria-label="rule {slug} workspace tally">
  <div class="font-mono uppercase tracking-wider text-zinc-500">{label}</div>
  <div class="tabular-nums"><span class="text-emerald-600">{pass}</span> / <span class="text-red-600">{fail}</span> · <span class="text-zinc-400">{na}</span></div>
</div>"#,
                slug = rule.slug(),
                label = rule.label(),
                pass = pass,
                fail = fail,
                na = na,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    let filter_links = render_filter_links(m, tenant_id);
    let sort_links = render_sort_links(m, tenant_id);

    format!(
        r#"<div class="grid grid-cols-2 sm:grid-cols-4 lg:grid-cols-8 gap-2 mb-3">{pills}</div>
<div class="flex flex-wrap gap-3 text-xs text-zinc-600 dark:text-zinc-300 mb-3" aria-label="matrix controls">
  <div>Filter: {filters}</div>
  <div>Sort: {sorts}</div>
</div>"#,
        pills = pills,
        filters = filter_links,
        sorts = sort_links,
    )
}

fn render_filter_links(m: &CharterMatrix, tenant_id: &str) -> String {
    let modes = [
        (FilterMode::All, "all"),
        (FilterMode::AnyFailing, "failing"),
        (FilterMode::Tier1, "tier-1"),
        (FilterMode::InfraOnly, "infra"),
    ];
    modes
        .iter()
        .map(|(mode, label)| {
            let active = *mode == m.filter;
            let cls = if active {
                "font-semibold underline"
            } else {
                "underline-offset-2 hover:underline"
            };
            format!(
                r#"<a class="{cls}" href="/admin/_audit?tenant_id={tid}&filter={f}&sort={s}">{label}</a>"#,
                cls = cls,
                tid = escape(tenant_id),
                f = mode.slug(),
                s = m.sort.slug(),
                label = label,
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn render_sort_links(m: &CharterMatrix, tenant_id: &str) -> String {
    let keys = [
        (SortKey::Name, "name"),
        (SortKey::FillRatio, "fill_ratio"),
        (SortKey::TestCount, "test_count"),
        (SortKey::LastAudit, "last_audit"),
        (SortKey::PassCount, "pass_count"),
    ];
    keys.iter()
        .map(|(k, label)| {
            let active = *k == m.sort;
            let cls = if active {
                "font-semibold underline"
            } else {
                "underline-offset-2 hover:underline"
            };
            format!(
                r#"<a class="{cls}" href="/admin/_audit?tenant_id={tid}&filter={f}&sort={s}">{label}</a>"#,
                cls = cls,
                tid = escape(tenant_id),
                f = m.filter.slug(),
                s = k.slug(),
                label = label,
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn render_table(m: &CharterMatrix) -> String {
    if m.rows.is_empty() {
        return r#"<p class="text-sm text-zinc-500 italic">No crates match the current filter.</p>"#.into();
    }
    let head = CharterRule::ALL
        .iter()
        .map(|r| {
            format!(
                r#"<th scope="col" class="px-2 py-1 text-left font-mono uppercase text-[10px] text-zinc-500">{}</th>"#,
                r.label(),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let rows = m
        .rows
        .iter()
        .map(render_row)
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        r#"<div class="overflow-x-auto">
<table class="min-w-full text-sm border-collapse" aria-label="charter v2 matrix">
  <thead class="bg-zinc-50 dark:bg-zinc-900">
    <tr>
      <th scope="col" class="px-2 py-1 text-left text-xs uppercase text-zinc-500">Crate</th>
      <th scope="col" class="px-2 py-1 text-right text-xs uppercase text-zinc-500">fill</th>
      <th scope="col" class="px-2 py-1 text-right text-xs uppercase text-zinc-500">tests</th>
      <th scope="col" class="px-2 py-1 text-left text-xs uppercase text-zinc-500">audit</th>
      {head}
      <th scope="col" class="px-2 py-1 text-left text-xs uppercase text-zinc-500">pin</th>
      <th scope="col" class="px-2 py-1 text-left text-xs uppercase text-zinc-500">last commit</th>
    </tr>
  </thead>
  <tbody>
    {rows}
  </tbody>
</table>
</div>"#,
        head = head,
        rows = rows,
    )
}

fn render_row(r: &CrateCharter) -> String {
    let pills = r
        .verdicts
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let rule = CharterRule::ALL[i];
            let (cls, text, aria) = match v {
                Verdict::Pass => (
                    "bg-emerald-100 text-emerald-800 dark:bg-emerald-900/50 dark:text-emerald-200",
                    "PASS",
                    format!("{} PASS", rule.label()),
                ),
                Verdict::Fail => (
                    "bg-red-100 text-red-800 dark:bg-red-900/50 dark:text-red-200",
                    "FAIL",
                    format!("{} FAIL", rule.label()),
                ),
                Verdict::Na => (
                    "bg-zinc-100 text-zinc-500 dark:bg-zinc-800 dark:text-zinc-400",
                    "—",
                    format!("{} not applicable", rule.label()),
                ),
            };
            format!(
                r#"<td class="px-2 py-1"><span class="inline-block rounded px-1.5 py-0.5 text-[10px] font-mono {cls}" aria-label="{aria}">{text}</span></td>"#,
                cls = cls,
                text = text,
                aria = escape(&aria),
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let fill_cell = match r.fill_ratio {
        Some(v) => format!(r#"<span class="tabular-nums">{:.2}</span>"#, v),
        None => "<span class=\"text-zinc-400\">—</span>".into(),
    };
    let audit_cell = r
        .last_audit
        .as_deref()
        .map(|s| format!(r#"<time datetime="{0}" class="tabular-nums">{0}</time>"#, escape(s)))
        .unwrap_or_else(|| "<span class=\"text-zinc-400\">—</span>".into());
    let pin_cell = r
        .manifest_pin
        .as_deref()
        .map(|s| format!(r#"<span class="font-mono text-xs">{}</span>"#, escape(s)))
        .unwrap_or_else(|| "<span class=\"text-zinc-400\">—</span>".into());
    let commit_cell = match &r.last_commit {
        Some(c) => format!(
            r#"<code class="text-[11px] text-zinc-700 dark:text-zinc-300">{} {}</code>"#,
            escape(&c.sha),
            escape(&c.subject),
        ),
        None => "<span class=\"text-zinc-400\">—</span>".into(),
    };
    format!(
        r#"<tr class="border-t border-zinc-200 dark:border-zinc-800">
  <td class="px-2 py-1 font-mono text-xs">{name}</td>
  <td class="px-2 py-1 text-right">{fill}</td>
  <td class="px-2 py-1 text-right tabular-nums">{tests}</td>
  <td class="px-2 py-1">{audit}</td>
  {pills}
  <td class="px-2 py-1">{pin}</td>
  <td class="px-2 py-1">{commit}</td>
</tr>"#,
        name = escape(&r.name),
        fill = fill_cell,
        tests = r.test_count,
        audit = audit_cell,
        pills = pills,
        pin = pin_cell,
        commit = commit_cell,
    )
}

// ── Tests ────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::compliance::{ComplianceSnapshot, CrateCompliance};
    use std::collections::BTreeMap;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 5, 18).unwrap()
    }

    fn base_crate(name: &str) -> CrateCompliance {
        CrateCompliance {
            name: name.into(),
            upstream_version: Some("v1.0.0".into()),
            upstream_org_repo: Some("acme/upstream".into()),
            backend_loc: 1000,
            backend_test_count: 20,
            ignored_test_count: 0,
            unimplemented_count: 0,
            todo_count: 0,
            portal_admin_present: true,
            cavectl_subcommand_present: true,
            obs_alerts_present: true,
            obs_dashboard_present: true,
            four_track_score: 100,
            infra_only: false,
            parity_ratio: Some(0.8),
            parity_ratio_source: Some("manifest".into()),
            parity_ratio_last_audit: Some("2026-05-15".into()),
            honest_parity_ratio: Some(0.75),
            parity_mapped_count: Some(10),
            parity_partial_count: Some(2),
            parity_skipped_count: Some(1),
            parity_unmapped_count: Some(0),
            parity_total_count: Some(13),
            manifest_filled: Some(true),
            audit_tier: Some("A".into()),
            portal_ui_status: None,
            portal_ui_priority: None,
            portal_ui_upstream_url: None,
            portal_ui_score: None,
            behavioral_parity: None,
            behavioral_ported: None,
            behavioral_total: None,
            behavioral_audit_scope: None,
            behavioral_audit_at: None,
        }
    }

    fn full_scan(n_files: u32) -> CrateScan {
        CrateScan {
            src_rs_files: n_files,
            spdx_marked_files: n_files,
            deprecated_attr_count: 0,
        }
    }

    // ── rule evaluation: TDD ─────────────────────────────────────────

    #[test]
    fn tdd_passes_when_tests_present_and_no_ignored() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Pass, "TDD column");
    }

    #[test]
    fn tdd_fails_when_no_tests_in_tier_one_crate() {
        let mut c = base_crate("cave-x");
        c.backend_test_count = 0;
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Fail);
    }

    #[test]
    fn tdd_fails_when_ignored_tests_present() {
        let mut c = base_crate("cave-x");
        c.ignored_test_count = 1;
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Fail);
    }

    #[test]
    fn tdd_na_for_infra_only_without_tests() {
        let mut c = base_crate("cave-utils");
        c.infra_only = true;
        c.backend_test_count = 0;
        let v = evaluate(&c, &full_scan(5), today());
        assert_eq!(v[0], Verdict::Na);
    }

    // ── rule evaluation: SPDX ─────────────────────────────────────────

    #[test]
    fn spdx_passes_when_every_file_carries_header() {
        let c = base_crate("cave-x");
        let scan = CrateScan {
            src_rs_files: 10,
            spdx_marked_files: 10,
            deprecated_attr_count: 0,
        };
        let v = evaluate(&c, &scan, today());
        assert_eq!(v[1], Verdict::Pass);
    }

    #[test]
    fn spdx_fails_with_any_file_missing_header() {
        let c = base_crate("cave-x");
        let scan = CrateScan {
            src_rs_files: 10,
            spdx_marked_files: 9,
            deprecated_attr_count: 0,
        };
        let v = evaluate(&c, &scan, today());
        assert_eq!(v[1], Verdict::Fail);
    }

    #[test]
    fn spdx_na_when_no_source_files() {
        let c = base_crate("cave-empty");
        let v = evaluate(&c, &CrateScan::empty(), today());
        assert_eq!(v[1], Verdict::Na);
    }

    // ── rule evaluation: source-stamp ─────────────────────────────────

    #[test]
    fn source_stamp_passes_with_full_upstream() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[2], Verdict::Pass);
    }

    #[test]
    fn source_stamp_fails_when_version_missing() {
        let mut c = base_crate("cave-x");
        c.upstream_version = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[2], Verdict::Fail);
    }

    #[test]
    fn source_stamp_na_for_infra_only() {
        let mut c = base_crate("cave-utils");
        c.infra_only = true;
        c.upstream_org_repo = None;
        c.upstream_version = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[2], Verdict::Na);
    }

    // ── rule evaluation: no-stub ─────────────────────────────────────

    #[test]
    fn no_stub_passes_when_zero_unimpl_and_todo() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[3], Verdict::Pass);
    }

    #[test]
    fn no_stub_fails_with_unimpl_present() {
        let mut c = base_crate("cave-x");
        c.unimplemented_count = 1;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[3], Verdict::Fail);
    }

    #[test]
    fn no_stub_fails_with_todo_present() {
        let mut c = base_crate("cave-x");
        c.todo_count = 1;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[3], Verdict::Fail);
    }

    // ── rule evaluation: no-backcompat ───────────────────────────────

    #[test]
    fn no_backcompat_passes_with_zero_deprecated_attrs() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[4], Verdict::Pass);
    }

    #[test]
    fn no_backcompat_fails_with_any_deprecated_attr() {
        let c = base_crate("cave-x");
        let scan = CrateScan {
            src_rs_files: 1,
            spdx_marked_files: 1,
            deprecated_attr_count: 1,
        };
        let v = evaluate(&c, &scan, today());
        assert_eq!(v[4], Verdict::Fail);
    }

    // ── rule evaluation: always-latest ───────────────────────────────

    #[test]
    fn always_latest_passes_when_audit_under_90_days() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = Some("2026-05-15".into());
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Pass);
    }

    #[test]
    fn always_latest_fails_when_audit_older_than_90_days() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = Some("2025-01-01".into());
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Fail);
    }

    #[test]
    fn always_latest_na_when_no_audit_date() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Na);
    }

    #[test]
    fn always_latest_fails_on_unparseable_date() {
        let mut c = base_crate("cave-x");
        c.parity_ratio_last_audit = Some("yesterday".into());
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[5], Verdict::Fail);
    }

    // ── rule evaluation: 4-track ─────────────────────────────────────

    #[test]
    fn four_track_passes_when_all_present() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[6], Verdict::Pass);
    }

    #[test]
    fn four_track_fails_when_any_track_missing() {
        let mut c = base_crate("cave-x");
        c.obs_alerts_present = false;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[6], Verdict::Fail);
    }

    #[test]
    fn four_track_na_for_infra_only() {
        let mut c = base_crate("cave-utils");
        c.infra_only = true;
        c.portal_admin_present = false;
        c.cavectl_subcommand_present = false;
        c.obs_alerts_present = false;
        c.obs_dashboard_present = false;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[6], Verdict::Na);
    }

    // ── rule evaluation: honest ──────────────────────────────────────

    #[test]
    fn honest_passes_with_honest_ratio_and_filled_manifest() {
        let c = base_crate("cave-x");
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[7], Verdict::Pass);
    }

    #[test]
    fn honest_fails_when_manifest_empty() {
        let mut c = base_crate("cave-x");
        c.manifest_filled = Some(false);
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[7], Verdict::Fail);
    }

    #[test]
    fn honest_fails_when_honest_ratio_missing() {
        let mut c = base_crate("cave-x");
        c.honest_parity_ratio = None;
        let v = evaluate(&c, &full_scan(1), today());
        assert_eq!(v[7], Verdict::Fail);
    }

    // ── canonical ordering ────────────────────────────────────────────

    #[test]
    fn charter_rule_all_has_eight_entries_in_canonical_order() {
        let slugs: Vec<&str> = CharterRule::ALL.iter().map(|r| r.slug()).collect();
        assert_eq!(
            slugs,
            vec![
                "tdd",
                "spdx",
                "source_stamp",
                "no_stub",
                "no_backcompat",
                "always_latest",
                "four_track",
                "honest",
            ]
        );
    }

    #[test]
    fn crate_charter_pass_count_matches_verdict_array() {
        let mut c = base_crate("cave-x");
        c.unimplemented_count = 1;
        c.obs_alerts_present = false;
        let v = evaluate(&c, &full_scan(1), today());
        let row = CrateCharter {
            name: c.name.clone(),
            infra_only: c.infra_only,
            fill_ratio: c.parity_ratio,
            fill_ratio_source: c.parity_ratio_source.clone(),
            honest_ratio: c.honest_parity_ratio,
            test_count: c.backend_test_count,
            last_audit: c.parity_ratio_last_audit.clone(),
            manifest_pin: Some("acme/upstream @ v1.0.0".into()),
            last_commit: None,
            verdicts: v,
        };
        assert_eq!(row.fail_count(), 2);
        assert_eq!(row.pass_count(), 6);
        assert_eq!(row.verdict_for(CharterRule::NoStub), Verdict::Fail);
        assert_eq!(row.verdict_for(CharterRule::FourTrack), Verdict::Fail);
        assert_eq!(row.verdict_for(CharterRule::Tdd), Verdict::Pass);
    }

    // ── matrix build ─────────────────────────────────────────────────

    fn snap(crates: Vec<CrateCompliance>) -> ComplianceSnapshot {
        ComplianceSnapshot { crates }
    }

    fn empty_scans() -> BTreeMap<String, CrateScan> {
        BTreeMap::new()
    }
    fn empty_commits() -> BTreeMap<String, CommitRow> {
        BTreeMap::new()
    }

    #[test]
    fn build_matrix_produces_one_row_per_crate() {
        let s = snap(vec![base_crate("cave-a"), base_crate("cave-b"), base_crate("cave-c")]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        assert_eq!(m.total_crates, 3);
        assert_eq!(m.rows.len(), 3);
    }

    #[test]
    fn build_matrix_attaches_per_crate_scan_results_for_spdx() {
        let s = snap(vec![base_crate("cave-x")]);
        let mut scans = BTreeMap::new();
        scans.insert(
            "cave-x".into(),
            CrateScan {
                src_rs_files: 10,
                spdx_marked_files: 7,
                deprecated_attr_count: 0,
            },
        );
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.verdict_for(CharterRule::Spdx), Verdict::Fail);
    }

    #[test]
    fn build_matrix_carries_last_commit_when_available() {
        let s = snap(vec![base_crate("cave-x")]);
        let mut commits = BTreeMap::new();
        commits.insert(
            "cave-x".into(),
            CommitRow {
                sha: "deadbeef".into(),
                subject: "feat(cave-x): wire it up".into(),
            },
        );
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &commits,
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.last_commit.as_ref().unwrap().sha, "deadbeef");
    }

    #[test]
    fn build_matrix_carries_manifest_pin_from_upstream_fields() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.manifest_pin.as_deref(), Some("acme/upstream @ v1.0.0"));
    }

    #[test]
    fn build_matrix_carries_fill_ratio_and_test_count() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let row = &m.rows[0];
        assert_eq!(row.fill_ratio, Some(0.8));
        assert_eq!(row.test_count, 20);
        assert_eq!(row.last_audit.as_deref(), Some("2026-05-15"));
    }

    #[test]
    fn build_matrix_records_rule_pass_counts() {
        let a = base_crate("cave-a");
        let mut b = base_crate("cave-b");
        b.unimplemented_count = 1;
        let mut c = base_crate("cave-c");
        c.obs_alerts_present = false;
        let s = snap(vec![a, b, c]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        assert_eq!(m.rule_pass_counts[3], 2, "no_stub passes for a + c");
        assert_eq!(m.rule_fail_counts[3], 1, "no_stub fails for b");
        assert_eq!(m.rule_pass_counts[6], 2, "four_track passes for a + b");
        assert_eq!(m.rule_fail_counts[6], 1, "four_track fails for c");
    }

    // ── filter / sort ────────────────────────────────────────────────

    #[test]
    fn filter_any_failing_drops_clean_crates() {
        let a = base_crate("cave-a");
        let mut b = base_crate("cave-b");
        b.unimplemented_count = 1;
        let s = snap(vec![a, b]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::AnyFailing,
            SortKey::Name,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-b"]);
        assert_eq!(m.total_crates, 2);
    }

    #[test]
    fn filter_infra_only_keeps_infra_drops_tier1() {
        let a = base_crate("cave-tier1");
        let mut b = base_crate("cave-utils");
        b.infra_only = true;
        let s = snap(vec![a, b]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::InfraOnly,
            SortKey::Name,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-utils"]);
    }

    #[test]
    fn filter_tier1_drops_infra() {
        let a = base_crate("cave-tier1");
        let mut b = base_crate("cave-utils");
        b.infra_only = true;
        let s = snap(vec![a, b]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::Tier1,
            SortKey::Name,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-tier1"]);
    }

    #[test]
    fn sort_fill_ratio_orders_higher_first_none_last() {
        let mut a = base_crate("cave-low");
        a.parity_ratio = Some(0.2);
        let mut b = base_crate("cave-high");
        b.parity_ratio = Some(0.9);
        let mut c = base_crate("cave-none");
        c.parity_ratio = None;
        let s = snap(vec![a, b, c]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::FillRatio,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-high", "cave-low", "cave-none"]);
    }

    #[test]
    fn sort_pass_count_orders_clean_first() {
        let a = base_crate("cave-clean");
        let mut b = base_crate("cave-broken");
        b.unimplemented_count = 1;
        b.obs_alerts_present = false;
        b.honest_parity_ratio = None;
        let s = snap(vec![b, a]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::PassCount,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-clean", "cave-broken"]);
    }

    #[test]
    fn sort_last_audit_orders_newest_first() {
        let mut a = base_crate("cave-old");
        a.parity_ratio_last_audit = Some("2025-01-01".into());
        let mut b = base_crate("cave-new");
        b.parity_ratio_last_audit = Some("2026-05-10".into());
        let s = snap(vec![a, b]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::LastAudit,
        );
        let names: Vec<&str> = m.rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["cave-new", "cave-old"]);
    }

    #[test]
    fn filter_mode_parse_handles_aliases() {
        assert_eq!(FilterMode::parse("failing"), FilterMode::AnyFailing);
        assert_eq!(FilterMode::parse("any_failing"), FilterMode::AnyFailing);
        assert_eq!(FilterMode::parse("tier1"), FilterMode::Tier1);
        assert_eq!(FilterMode::parse("infra"), FilterMode::InfraOnly);
        assert_eq!(FilterMode::parse("infra_only"), FilterMode::InfraOnly);
        assert_eq!(FilterMode::parse("nonsense"), FilterMode::All);
    }

    #[test]
    fn sort_key_parse_handles_aliases() {
        assert_eq!(SortKey::parse("ratio"), SortKey::FillRatio);
        assert_eq!(SortKey::parse("tests"), SortKey::TestCount);
        assert_eq!(SortKey::parse("audit"), SortKey::LastAudit);
        assert_eq!(SortKey::parse("passes"), SortKey::PassCount);
        assert_eq!(SortKey::parse("nonsense"), SortKey::Name);
    }

    // ── render ───────────────────────────────────────────────────────

    #[test]
    fn render_section_includes_heading_and_table() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("Charter v2 matrix"));
        assert!(html.contains("<table"));
        assert!(html.contains("cave-x"));
    }

    #[test]
    fn render_section_includes_eight_rule_pills_in_header() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        for rule in CharterRule::ALL.iter() {
            assert!(
                html.contains(rule.label()),
                "header pill for {} missing",
                rule.label()
            );
        }
    }

    #[test]
    fn render_section_emits_per_rule_pass_fail_pills_on_row() {
        let mut bad = base_crate("cave-bad");
        bad.unimplemented_count = 1;
        let s = snap(vec![bad]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("FAIL"));
        assert!(html.contains("PASS"));
    }

    #[test]
    fn render_section_links_to_filter_and_sort_variants_preserving_tenant() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("tenant_id=acme"));
        assert!(html.contains("filter=any_failing"));
        assert!(html.contains("sort=fill_ratio"));
    }

    #[test]
    fn render_section_shows_empty_state_when_filter_excludes_everything() {
        let s = snap(vec![base_crate("cave-clean")]);
        let scans: BTreeMap<String, CrateScan> = s
            .crates
            .iter()
            .map(|c| (c.name.clone(), full_scan(1)))
            .collect();
        let m = build_matrix(
            &s,
            today(),
            &scans,
            &empty_commits(),
            FilterMode::AnyFailing,
            SortKey::Name,
        );
        let html = render_section(&m, "acme");
        assert!(html.contains("No crates match"));
    }

    // ── scan_crate_io smoke ──────────────────────────────────────────

    #[test]
    fn scan_crate_io_returns_empty_for_missing_crate() {
        let tmp = tempfile::TempDir::new().unwrap();
        let s = scan_crate_io(tmp.path(), "does-not-exist");
        assert_eq!(s, CrateScan::empty());
    }

    #[test]
    fn scan_crate_io_counts_spdx_and_deprecated() {
        let tmp = tempfile::TempDir::new().unwrap();
        let crate_dir = tmp.path().join("crates/cave-y/src");
        std::fs::create_dir_all(&crate_dir).unwrap();
        std::fs::write(
            crate_dir.join("a.rs"),
            "// SPDX-License-Identifier: AGPL-3.0-or-later\n// Copyright (C) 2026\npub fn ok() {}\n",
        )
        .unwrap();
        std::fs::write(
            crate_dir.join("b.rs"),
            "// no header here\n#[deprecated]\npub fn old() {}\n",
        )
        .unwrap();
        let s = scan_crate_io(tmp.path(), "cave-y");
        assert_eq!(s.src_rs_files, 2);
        assert_eq!(s.spdx_marked_files, 1);
        assert_eq!(s.deprecated_attr_count, 1);
    }

    // ── JSON ─────────────────────────────────────────────────────────

    #[test]
    fn charter_matrix_serialises_to_json() {
        let s = snap(vec![base_crate("cave-x")]);
        let m = build_matrix(
            &s,
            today(),
            &empty_scans(),
            &empty_commits(),
            FilterMode::All,
            SortKey::Name,
        );
        let j = serde_json::to_value(&m).unwrap();
        assert!(j["rows"].is_array());
        assert_eq!(j["rows"].as_array().unwrap().len(), 1);
        assert!(j["rule_pass_counts"].is_array());
        assert_eq!(j["rule_pass_counts"].as_array().unwrap().len(), 8);
        assert!(j["total_crates"].is_number());
    }
}
