// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/adr` view — Architecture Decision Record browser.
//!
//! Walks the workspace's `docs/adr/` directory (`.md` files at the
//! root level only) and renders a list + detail surface for the
//! platform-public ADRs. The `internal/` sub-directory is explicitly
//! excluded — those are vendor- or deployment-branded decisions that
//! belong to a specific operator's deployment, not the OSS runtime
//! charter the public dashboard exposes.
//!
//! **Persona gate**: ADR Browser is `Persona::PlatformAdmin` only.
//! Tenant admins reading the dashboard would see decisions that
//! affect cross-tenant infrastructure and could confuse them about
//! what their tenant is allowed to do — the routing handler in
//! `crate::admin::mod` enforces the gate before this module's
//! `render` runs, but `list_records` also re-checks defensively.

use crate::admin::permission::{AuthError, Persona, RequestCtx};
use crate::admin::render::{
    badge, empty_state, escape, markdown_lite, page_shell_full, search_box, sortable_table,
};
use crate::admin::types::Cite;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AdrViewError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("ADR id '{0}' not found")]
    NotFound(String),
    #[error("ADR id '{0}' is reserved (internal/ or non-public)")]
    Forbidden(String),
    #[error("failed to read ADR directory: {0}")]
    Io(String),
}

/// One row in the ADR Browser list view.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdrEntry {
    /// Short identifier extracted from the file name, e.g. `ADR-014`.
    pub id: String,
    /// Human-readable title parsed from the first `# …` heading in
    /// the file body, or fallback to the file stem.
    pub title: String,
    /// Stem of the file ("ADR-014_Zero_Trust_Network_Architecture"),
    /// used as the route param in `/admin/adr/<stem>`.
    pub stem: String,
    /// Status: Accepted / Proposed / Superseded — parsed from the
    /// document body when present, defaults to "Unknown".
    pub status: AdrStatus,
    /// True when this ADR affects the runtime (control plane, security,
    /// data plane, cross-tenant infra). False when it documents a
    /// deployment-specific or process-only decision (vendor branding,
    /// release process, doc layout) that should be hidden from the
    /// portal's runtime-focused view.
    ///
    /// 2026-05-22 — pairs with the parallel `c110135a` ADR cleanup ray
    /// to keep the public ADR Browser focused on decisions that affect
    /// the running system. Heuristic-derived from a `Scope:` line in
    /// the document body; see [`extract_is_runtime`] for the rules.
    #[serde(default = "default_is_runtime")]
    pub is_runtime: bool,
}

fn default_is_runtime() -> bool {
    true
}

/// ADR lifecycle phase mirroring upstream Backstage ADR plugin.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdrStatus {
    Accepted,
    Proposed,
    Superseded,
    Unknown,
}

impl AdrStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            AdrStatus::Accepted => "Accepted",
            AdrStatus::Proposed => "Proposed",
            AdrStatus::Superseded => "Superseded",
            AdrStatus::Unknown => "Unknown",
        }
    }
}

#[allow(dead_code)]
const FILE_CITE: Cite = Cite::backstage("plugins/adr/src/components/AdrList.tsx", "AdrList");

/// Resolve the workspace's `docs/adr/` directory. Returns the path
/// even if it doesn't exist on disk so error messages stay
/// deterministic. Honours `CAVE_ADR_DIR` for overriding in tests.
pub fn adr_dir() -> PathBuf {
    if let Ok(p) = std::env::var("CAVE_ADR_DIR") {
        return PathBuf::from(p);
    }
    // Workspace root resolution mirrors compliance::workspace_root() —
    // walk up until the Cargo.lock is found, fall back to CWD.
    let mut cur = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    loop {
        if cur.join("Cargo.lock").exists() {
            return cur.join("docs/adr");
        }
        if !cur.pop() {
            break;
        }
    }
    PathBuf::from("docs/adr")
}

/// Walk the ADR directory and return one entry per top-level `.md`
/// file. `internal/` is explicitly skipped — even with a manual
/// override of the dir variable, any nested subdirectory is filtered
/// out, so a future `docs/adr/draft/` wouldn't leak either.
pub fn list_records(ctx: &RequestCtx) -> Result<Vec<AdrEntry>, AdrViewError> {
    // Platform gate FIRST. Lets tests assert the rejection path
    // without needing the on-disk directory.
    ctx.require_persona(Persona::PlatformAdmin)?;

    let dir = adr_dir();
    list_records_in(&dir)
}

/// Pure variant used by tests with a tempdir fixture. Does NOT
/// re-check the persona gate — callers must check it via
/// `list_records` or directly.
pub fn list_records_in(dir: &Path) -> Result<Vec<AdrEntry>, AdrViewError> {
    let read = match std::fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) => {
            // Missing dir is not a hard error — render empty list
            // (deployment-time docs/adr/ may not exist yet).
            if e.kind() == std::io::ErrorKind::NotFound {
                return Ok(vec![]);
            }
            return Err(AdrViewError::Io(e.to_string()));
        }
    };

    let mut out: Vec<AdrEntry> = Vec::new();
    for ent in read.flatten() {
        let path = ent.path();
        // Top-level *.md only — skip subdirectories (notably internal/).
        if !path.is_file() {
            continue;
        }
        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if !ext.eq_ignore_ascii_case("md") {
            continue;
        }
        if !stem.to_uppercase().starts_with("ADR-") && !stem.starts_with("adr-") {
            // Tolerate alternative naming but skip non-ADR docs.
            continue;
        }
        // Read first 8 KB for title + status — full body lives in
        // the detail view.
        let body = std::fs::read_to_string(&path).unwrap_or_default();
        out.push(AdrEntry {
            id: extract_id(&stem),
            title: extract_title(&body, &stem),
            stem: stem.clone(),
            status: extract_status(&body),
            is_runtime: extract_is_runtime(&body, &stem),
        });
    }

    // Stable order: by id ascending so the list is deterministic.
    out.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(out)
}

/// Load one ADR's full body. Refuses `internal/` paths explicitly
/// (defence-in-depth even if a caller hands in a crafted stem).
pub fn load_body(ctx: &RequestCtx, stem: &str) -> Result<String, AdrViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    if stem.contains('/') || stem.contains("..") {
        return Err(AdrViewError::Forbidden(stem.to_string()));
    }
    let dir = adr_dir();
    let path = dir.join(format!("{stem}.md"));
    // If the path resolves into internal/ via a relative traversal
    // the canonicalise would catch it; reject anyway by enforcing
    // direct parent match.
    if let (Ok(cp), Ok(cd)) = (std::fs::canonicalize(&path), std::fs::canonicalize(&dir)) {
        if cp.parent() != Some(&cd) {
            return Err(AdrViewError::Forbidden(stem.to_string()));
        }
    }
    std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            AdrViewError::NotFound(stem.to_string())
        } else {
            AdrViewError::Io(e.to_string())
        }
    })
}

fn extract_id(stem: &str) -> String {
    // "ADR-014_Zero_Trust" → "ADR-014"
    if let Some(idx) = stem.find('_') {
        return stem[..idx].to_string();
    }
    if let Some(idx) = stem.find('-') {
        // "ADR-001-foo" → "ADR-001"
        if let Some(second) = stem[idx + 1..].find('-') {
            return stem[..idx + 1 + second].to_string();
        }
    }
    stem.to_string()
}

fn extract_title(body: &str, fallback_stem: &str) -> String {
    for line in body.lines().take(40) {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("# ") {
            return rest.trim().to_string();
        }
    }
    // Fall back to a humanised stem ("ADR-014_Zero_Trust_Network…"
    // → "Zero Trust Network…").
    let after_id = fallback_stem.splitn(2, '_').nth(1).unwrap_or(fallback_stem);
    after_id.replace('_', " ")
}

/// Decide whether an ADR documents a runtime decision (control plane,
/// security, data plane, charter parity) versus a process / branding /
/// deployment-vendor concern that should be hidden from the portal's
/// runtime-focused list view.
///
/// Heuristic, in order of precedence:
///   1. Explicit `Scope: runtime` / `Scope: process` line in the
///      document body — case-insensitive prefix match, just like
///      `Status:`. `runtime` wins, `process` / `branding` / `vendor` /
///      `deployment` / `oss-launch` lose.
///   2. File name heuristics — stems containing `_branding_`,
///      `_oss_launch_`, `_release_process_`, `_internal_` are
///      classified non-runtime. The `internal/` directory is already
///      stripped by `list_records_in`; this is a belt-and-braces guard
///      for files that slipped to the top level by accident.
///   3. Default true — when the ADR doesn't declare scope, assume
///      runtime so existing decisions don't disappear. Authors opting
///      a process-only ADR out of the runtime list must add an
///      explicit `Scope: process` line.
pub fn extract_is_runtime(body: &str, stem: &str) -> bool {
    for line in body.lines().take(120) {
        let lower = line.to_lowercase();
        let value = if let Some(v) = lower.strip_prefix("scope:") {
            v
        } else if let Some(v) = lower.strip_prefix("**scope:**") {
            v
        } else if let Some(v) = lower.strip_prefix("scope :") {
            v
        } else {
            continue;
        };
        let t = value.trim();
        if t.starts_with("runtime") || t.starts_with("control-plane") || t.starts_with("data-plane")
        {
            return true;
        }
        for bad in [
            "process",
            "branding",
            "vendor",
            "deployment",
            "oss-launch",
            "oss_launch",
            "release-process",
            "release_process",
            "internal",
        ] {
            if t.starts_with(bad) {
                return false;
            }
        }
    }
    let lower_stem = stem.to_lowercase();
    for marker in [
        "_branding_",
        "_oss_launch_",
        "_oss-launch_",
        "_release_process_",
        "_release-process_",
        "_internal_",
    ] {
        if lower_stem.contains(marker) {
            return false;
        }
    }
    true
}

fn extract_status(body: &str) -> AdrStatus {
    // Scan first 80 lines for "Status: …" or "**Status:** …".
    for line in body.lines().take(80) {
        let lower = line.to_lowercase();
        let value = if let Some(v) = lower.strip_prefix("status:") {
            v
        } else if let Some(v) = lower.strip_prefix("**status:**") {
            v
        } else if let Some(v) = lower.strip_prefix("status :") {
            v
        } else {
            continue;
        };
        let trimmed = value.trim();
        if trimmed.starts_with("accepted") {
            return AdrStatus::Accepted;
        }
        if trimmed.starts_with("proposed") {
            return AdrStatus::Proposed;
        }
        if trimmed.starts_with("superseded") {
            return AdrStatus::Superseded;
        }
    }
    AdrStatus::Unknown
}

pub fn render(ctx: &RequestCtx) -> Result<String, AdrViewError> {
    render_in_filtered(ctx, &adr_dir(), false)
}

/// Test-friendly variant — render against an explicit directory so
/// unit tests don't rely on the `CAVE_ADR_DIR` env var (which is
/// process-global and races under parallel `cargo test`).
pub fn render_in(ctx: &RequestCtx, dir: &Path) -> Result<String, AdrViewError> {
    render_in_filtered(ctx, dir, false)
}

/// Render the ADR Browser, optionally including non-runtime ADRs.
///
/// `include_non_runtime = false` (the default surface for /admin/adr)
/// filters out ADRs classified as process-only / branding / vendor.
/// `include_non_runtime = true` is reached by appending `?show=all` to
/// the URL — useful for the rare audit moments where someone wants the
/// full surface. The toggle link is rendered above the table so users
/// can flip without typing in the URL.
pub fn render_in_filtered(
    ctx: &RequestCtx,
    dir: &Path,
    include_non_runtime: bool,
) -> Result<String, AdrViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    let all_rows = list_records_in(dir)?;
    let total_disk = all_rows.len();
    let rows: Vec<&AdrEntry> = all_rows
        .iter()
        .filter(|r| include_non_runtime || r.is_runtime)
        .collect();
    let hidden = total_disk - rows.len();
    let n = rows.len();

    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            let status_tone = match r.status {
                AdrStatus::Accepted => "ok",
                AdrStatus::Proposed => "warn",
                AdrStatus::Superseded => "bad",
                AdrStatus::Unknown => "neutral",
            };
            let title_link = format!(
                r#"<a href="/admin/adr/{}?tenant_id={}">{}</a>"#,
                escape(&r.stem),
                escape(ctx.tenant.as_str()),
                escape(&r.title),
            );
            let scope_badge = if r.is_runtime {
                badge("info", "runtime")
            } else {
                badge("neutral", "process")
            };
            vec![
                format!("<code>{}</code>", escape(&r.id)),
                title_link,
                scope_badge,
                badge(status_tone, r.status.as_str()),
            ]
        })
        .collect();

    let body = if rows.is_empty() {
        empty_state(
            "🗂",
            "No ADRs to show",
            if total_disk == 0 {
                "Nothing in docs/adr/ — drop a Markdown file named ADR-XXX_…md to get started."
            } else {
                "All ADRs are tagged non-runtime in their Scope: line. Click \"Show non-runtime\" to see them."
            },
        )
    } else {
        sortable_table(
            "adr-list",
            &[
                ("id", "text"),
                ("title", "text"),
                ("scope", "text"),
                ("status", "text"),
            ],
            &table_rows,
        )
    };

    let toggle = if include_non_runtime {
        format!(
            r#"<a class="text-xs" href="/admin/adr?tenant_id={t}">Hide non-runtime</a>"#,
            t = escape(ctx.tenant.as_str()),
        )
    } else {
        let hidden_note = if hidden > 0 {
            format!(" ({hidden} hidden)")
        } else {
            String::new()
        };
        format!(
            r#"<a class="text-xs" href="/admin/adr?tenant_id={t}&amp;show=all">Show non-runtime{hint}</a>"#,
            t = escape(ctx.tenant.as_str()),
            hint = hidden_note,
        )
    };

    let body_html = format!(
        r#"<section>
            <div class="flex items-baseline justify-between mb-2">
                <h2 class="text-lg font-semibold">ADR Browser ({n})</h2>
                {toggle}
            </div>
            <p class="text-xs text-zinc-500 mb-3">
                Architecture Decision Records that affect the running
                runtime. Vendor-branded ADRs live under
                <code>docs/adr/internal/</code> and are never surfaced
                here. Process-only ADRs (release process, branding,
                OSS-launch checklists) are filtered out unless you
                ask for them — see the toggle above.
            </p>
            {search}
            {body}
        </section>"#,
        search = if rows.is_empty() {
            String::new()
        } else {
            search_box("#adr-list", "Filter by id, title, scope, status…")
        },
        body = body,
    );

    Ok(page_shell_full(
        ctx,
        "/admin/adr",
        &format!("ADR Browser · {}", escape(ctx.tenant.as_str())),
        &body_html,
    ))
}

pub fn render_detail(ctx: &RequestCtx, stem: &str) -> Result<String, AdrViewError> {
    let body = load_body(ctx, stem)?;
    let id = extract_id(stem);
    let title = extract_title(&body, stem);
    let status = extract_status(&body);
    let is_runtime = extract_is_runtime(&body, stem);
    let status_tone = match status {
        AdrStatus::Accepted => "ok",
        AdrStatus::Proposed => "warn",
        AdrStatus::Superseded => "bad",
        AdrStatus::Unknown => "neutral",
    };
    // 2026-05-22 — the previous detail surface rendered the raw markdown
    // inside a `<pre>` because the team didn't trust an MD parser. The
    // new `markdown_lite` is a hand-rolled, pure-Rust, escape-first
    // converter (input is XML-escaped first, then a closed grammar of
    // headings / lists / inline `code` / bold / italic / fenced code /
    // links is upgraded back) so the round-trip is auditable and the
    // detail view becomes readable.
    let md_html = markdown_lite(&body);
    let scope_badge = if is_runtime {
        badge("info", "runtime")
    } else {
        badge("neutral", "process")
    };
    let html = format!(
        r#"<section>
            <div class="flex items-baseline justify-between mb-3">
                <h2 class="text-lg font-semibold">{id} · {title}</h2>
                <div class="flex gap-2">
                    {scope}
                    {status_badge}
                </div>
            </div>
            <p class="text-xs text-zinc-500 mb-2">
                <a href="/admin/adr?tenant_id={tenant}">← back to list</a>
            </p>
            <div class="cave-md">{md}</div>
        </section>"#,
        id = escape(&id),
        title = escape(&title),
        scope = scope_badge,
        status_badge = badge(status_tone, status.as_str()),
        tenant = escape(ctx.tenant.as_str()),
        md = md_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/adr",
        &format!("{id} · ADR Browser", id = escape(&id)),
        &html,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;
    use std::fs;

    fn write(dir: &Path, name: &str, body: &str) {
        fs::write(dir.join(name), body).unwrap();
    }

    fn platform_ctx() -> RequestCtx {
        RequestCtx::developer_as(
            "platform",
            &[Permission::DashboardRead],
            Persona::PlatformAdmin,
        )
    }

    fn tenant_ctx() -> RequestCtx {
        RequestCtx::developer_as(
            "tenant1",
            &[Permission::DashboardRead],
            Persona::TenantAdmin,
        )
    }

    fn anon_ctx() -> RequestCtx {
        RequestCtx::developer_as(
            "anon-tenant",
            &[Permission::DashboardRead],
            Persona::Anonymous,
        )
    }

    fn fixture_dir() -> tempfile::TempDir {
        let d = tempfile::TempDir::new().unwrap();
        write(
            d.path(),
            "ADR-001-sovereign-bare-metal-hosting.md",
            "# Sovereign bare-metal hosting\n\nStatus: Accepted\n\nBody.\n",
        );
        write(
            d.path(),
            "ADR-014_Zero_Trust_Network_Architecture.md",
            "# Zero Trust Network Architecture\n\n**Status:** Proposed\n",
        );
        write(
            d.path(),
            "ADR-005_Buildah_for_Container_Image_Building.md",
            "Superseded by ADR-020.\n\nStatus: Superseded",
        );
        write(
            d.path(),
            "README.md", // Should be filtered (no ADR- prefix).
            "Just a readme.",
        );
        // internal/ sub-dir with one vendor-branded ADR — must NOT leak.
        let internal = d.path().join("internal");
        fs::create_dir(&internal).unwrap();
        write(
            &internal,
            "ADR-001_Hetzner_Cloud_as_Sovereign_Infrastructure_Provider.md",
            "# Hetzner Cloud\n\nStatus: Accepted\n",
        );
        d
    }

    #[test]
    fn list_records_in_filters_top_level_md_only() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/adr/src/components/AdrList.tsx",
            "AdrList",
            "adr-list-filter"
        );
        let dir = fixture_dir();
        let rows = list_records_in(dir.path()).unwrap();
        // Three real ADRs (README + internal/ both excluded).
        assert_eq!(rows.len(), 3);
        // Sorted by id ascending.
        assert_eq!(rows[0].id, "ADR-001");
        assert_eq!(rows[1].id, "ADR-005");
        assert_eq!(rows[2].id, "ADR-014");
        // No internal-prefixed entry leaked.
        assert!(!rows.iter().any(|r| r.stem.starts_with("ADR-001_Hetzner")));
    }

    #[test]
    fn list_records_in_extracts_title_from_h1_or_falls_back_to_stem() {
        let dir = fixture_dir();
        let rows = list_records_in(dir.path()).unwrap();
        let zero_trust = rows.iter().find(|r| r.id == "ADR-014").unwrap();
        assert_eq!(zero_trust.title, "Zero Trust Network Architecture");
        // ADR-005 has no `# heading` → fall back to humanised stem.
        let buildah = rows.iter().find(|r| r.id == "ADR-005").unwrap();
        assert!(buildah.title.contains("Buildah"));
    }

    #[test]
    fn list_records_in_extracts_status() {
        let dir = fixture_dir();
        let rows = list_records_in(dir.path()).unwrap();
        let zero_trust = rows.iter().find(|r| r.id == "ADR-014").unwrap();
        assert_eq!(zero_trust.status, AdrStatus::Proposed);
        let buildah = rows.iter().find(|r| r.id == "ADR-005").unwrap();
        assert_eq!(buildah.status, AdrStatus::Superseded);
        let sov = rows.iter().find(|r| r.id == "ADR-001").unwrap();
        assert_eq!(sov.status, AdrStatus::Accepted);
    }

    #[test]
    fn list_records_in_missing_dir_returns_empty() {
        let dir = tempfile::TempDir::new().unwrap();
        let nope = dir.path().join("never-exists");
        let rows = list_records_in(&nope).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn list_records_persona_gate_rejects_tenant_admin() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/adr/src/components/AdrList.tsx",
            "PersonaGate",
            "adr-persona-tenant"
        );
        // Tenant admin is rejected by the persona gate inside
        // list_records before the directory is even consulted, so
        // env-var fixture isn't needed.
        let err = list_records(&tenant_ctx()).unwrap_err();
        assert!(matches!(
            err,
            AdrViewError::Auth(AuthError::PersonaForbidden { .. })
        ));
    }

    #[test]
    fn list_records_persona_gate_rejects_anonymous() {
        let err = list_records(&anon_ctx()).unwrap_err();
        assert!(matches!(
            err,
            AdrViewError::Auth(AuthError::PersonaForbidden { .. })
        ));
    }

    #[test]
    fn render_persona_gate_blocks_render_for_tenant_admin() {
        let dir = fixture_dir();
        // render_in takes the dir directly — no env-var race.
        assert!(render_in(&tenant_ctx(), dir.path()).is_err());
    }

    #[test]
    fn render_succeeds_for_platform_admin_and_excludes_internal() {
        let dir = fixture_dir();
        let html = render_in(&platform_ctx(), dir.path()).unwrap();
        // List contains the 3 ADRs.
        assert!(html.contains("ADR-001"));
        assert!(html.contains("ADR-005"));
        assert!(html.contains("ADR-014"));
        // Must NOT contain the internal Hetzner ADR.
        assert!(!html.contains("Hetzner"));
    }

    #[test]
    fn load_body_rejects_traversal_paths() {
        let ctx = platform_ctx();
        let err = load_body(&ctx, "../README").unwrap_err();
        assert!(matches!(err, AdrViewError::Forbidden(_)));
        let err = load_body(&ctx, "internal/ADR-001_Hetzner").unwrap_err();
        assert!(matches!(err, AdrViewError::Forbidden(_)));
    }

    #[test]
    fn render_detail_blocks_tenant_admin() {
        assert!(render_detail(&tenant_ctx(), "ADR-001-sovereign-bare-metal-hosting").is_err());
    }

    #[test]
    fn extract_id_handles_underscore_and_dash_separators() {
        assert_eq!(extract_id("ADR-014_Zero_Trust"), "ADR-014");
        assert_eq!(
            extract_id("ADR-001-sovereign-bare-metal-hosting"),
            "ADR-001"
        );
        assert_eq!(extract_id("ADR-100"), "ADR-100");
    }

    #[test]
    fn extract_status_recognises_each_phase() {
        assert_eq!(extract_status("Status: Accepted"), AdrStatus::Accepted);
        assert_eq!(extract_status("**Status:** Proposed"), AdrStatus::Proposed);
        assert_eq!(extract_status("status: Superseded"), AdrStatus::Superseded);
        assert_eq!(extract_status("no status line"), AdrStatus::Unknown);
    }

    // ── is_runtime + filtered list view (2026-05-22) ───────────────

    #[test]
    fn extract_is_runtime_defaults_to_true_when_no_scope_line() {
        assert!(extract_is_runtime("just body content", "ADR-001_anything"));
    }

    #[test]
    fn extract_is_runtime_honours_explicit_runtime_scope() {
        assert!(extract_is_runtime("Status: Accepted\nScope: runtime", "x"));
        assert!(extract_is_runtime(
            "**Scope:** control-plane",
            "x"
        ));
    }

    #[test]
    fn extract_is_runtime_returns_false_for_process_and_branding_scopes() {
        assert!(!extract_is_runtime("Scope: process", "x"));
        assert!(!extract_is_runtime("Scope: branding", "x"));
        assert!(!extract_is_runtime("scope: deployment", "x"));
        assert!(!extract_is_runtime("scope: oss-launch", "x"));
        assert!(!extract_is_runtime("scope: release-process", "x"));
    }

    #[test]
    fn extract_is_runtime_uses_stem_heuristic_when_no_scope_line() {
        assert!(!extract_is_runtime("no scope line", "ADR-099_branding_polish"));
        assert!(!extract_is_runtime(
            "no scope line",
            "ADR-100_oss_launch_metadata"
        ));
        assert!(!extract_is_runtime(
            "no scope line",
            "ADR-101_release_process_v2"
        ));
        assert!(extract_is_runtime(
            "no scope line",
            "ADR-102_kubelet_pid_namespace"
        ));
    }

    #[test]
    fn render_filtered_hides_non_runtime_by_default() {
        let d = tempfile::TempDir::new().unwrap();
        write(
            d.path(),
            "ADR-001_runtime_thing.md",
            "# Runtime thing\n\nStatus: Accepted\nScope: runtime",
        );
        write(
            d.path(),
            "ADR-099_branding_polish.md",
            "# Branding polish\n\nStatus: Accepted\nScope: branding",
        );
        let html = render_in_filtered(&platform_ctx(), d.path(), false).unwrap();
        assert!(html.contains("Runtime thing"));
        assert!(!html.contains("Branding polish"));
        // Toggle to show non-runtime ADRs appears with the hidden count.
        assert!(html.contains("Show non-runtime"));
        assert!(html.contains("(1 hidden)"));
        assert!(html.contains("ADR Browser (1)"));
    }

    #[test]
    fn render_filtered_shows_all_when_include_non_runtime_true() {
        let d = tempfile::TempDir::new().unwrap();
        write(
            d.path(),
            "ADR-001_runtime_thing.md",
            "# Runtime thing\n\nStatus: Accepted",
        );
        write(
            d.path(),
            "ADR-099_branding_polish.md",
            "# Branding polish\n\nStatus: Accepted\nScope: branding",
        );
        let html = render_in_filtered(&platform_ctx(), d.path(), true).unwrap();
        assert!(html.contains("Runtime thing"));
        assert!(html.contains("Branding polish"));
        assert!(html.contains("Hide non-runtime"));
        assert!(html.contains("ADR Browser (2)"));
    }

    #[test]
    fn render_filtered_empty_state_when_no_runtime_adrs_after_filter() {
        let d = tempfile::TempDir::new().unwrap();
        write(
            d.path(),
            "ADR-099_branding_polish.md",
            "# Branding polish\n\nStatus: Accepted\nScope: branding",
        );
        let html = render_in_filtered(&platform_ctx(), d.path(), false).unwrap();
        assert!(html.contains("cave-empty"));
        assert!(html.contains("non-runtime"));
        assert!(html.contains("ADR Browser (0)"));
    }

    #[test]
    fn render_detail_renders_markdown_not_raw_pre() {
        let d = tempfile::TempDir::new().unwrap();
        write(
            d.path(),
            "ADR-042_demo.md",
            "# Demo title\n\nStatus: Accepted\n\n## Context\n\nA paragraph.\n\n- item 1\n- item 2\n",
        );
        unsafe {
            std::env::set_var("CAVE_ADR_DIR", d.path());
        }
        let html = render_detail(&platform_ctx(), "ADR-042_demo").unwrap();
        // Markdown was upgraded to HTML — not wrapped in a literal <pre>.
        assert!(html.contains("cave-md"));
        assert!(html.contains("<h2>Context</h2>"));
        assert!(html.contains("<ul><li>item 1</li>"));
        // Scope badge is present (runtime by default — no Scope: line).
        assert!(html.contains(">runtime</span>"));
        unsafe {
            std::env::remove_var("CAVE_ADR_DIR");
        }
    }
}
