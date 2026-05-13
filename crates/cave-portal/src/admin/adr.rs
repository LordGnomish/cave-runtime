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
use crate::admin::render::{escape, page_shell, table};
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
const FILE_CITE: Cite =
    Cite::backstage("plugins/adr/src/components/AdrList.tsx", "AdrList");

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
    if let (Ok(cp), Ok(cd)) = (
        std::fs::canonicalize(&path),
        std::fs::canonicalize(&dir),
    ) {
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
    let after_id = fallback_stem
        .splitn(2, '_')
        .nth(1)
        .unwrap_or(fallback_stem);
    after_id.replace('_', " ")
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
    render_in(ctx, &adr_dir())
}

/// Test-friendly variant — render against an explicit directory so
/// unit tests don't rely on the `CAVE_ADR_DIR` env var (which is
/// process-global and races under parallel `cargo test`).
pub fn render_in(ctx: &RequestCtx, dir: &Path) -> Result<String, AdrViewError> {
    ctx.require_persona(Persona::PlatformAdmin)?;
    let rows = list_records_in(dir)?;
    let n = rows.len();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                escape(&r.id),
                format!(
                    r#"<a class="text-blue-500 underline" href="/admin/adr/{}?tenant_id={}">{}</a>"#,
                    escape(&r.stem),
                    escape(ctx.tenant.as_str()),
                    escape(&r.title),
                ),
                r.status.as_str().into(),
            ]
        })
        .collect();

    let body = format!(
        r#"<section>
            <div class="flex items-center justify-between mb-2">
                <h2 class="text-lg font-semibold">ADR Browser ({n})</h2>
            </div>
            <p class="text-xs text-zinc-500 mb-3">
                Platform-public Architecture Decision Records.
                Vendor-branded ADRs live under
                <code>docs/adr/internal/</code> and are not surfaced
                here.
            </p>
            {tbl}
        </section>"#,
        tbl = table(&["id", "title", "status"], &table_rows),
    );

    Ok(page_shell(
        &format!("ADR Browser · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

pub fn render_detail(ctx: &RequestCtx, stem: &str) -> Result<String, AdrViewError> {
    let body = load_body(ctx, stem)?;
    // Very light markdown affordance — we render the raw body in a
    // <pre> so admins can verify the on-disk content without trusting
    // an embedded MD parser. The list view links here.
    let escaped = escape(&body);
    let id = extract_id(stem);
    let title = extract_title(&body, stem);
    let status = extract_status(&body);
    let html = format!(
        r#"<section>
            <div class="flex items-center justify-between mb-3">
                <h2 class="text-lg font-semibold">{id} · {title}</h2>
                <span class="text-xs px-2 py-1 rounded bg-zinc-100">{status}</span>
            </div>
            <p class="text-xs text-zinc-500 mb-2">
                <a class="text-blue-500 underline" href="/admin/adr?tenant_id={tenant}">← back to list</a>
            </p>
            <pre class="text-xs whitespace-pre-wrap p-3 bg-zinc-50 border rounded">{escaped}</pre>
        </section>"#,
        id = escape(&id),
        title = escape(&title),
        status = status.as_str(),
        tenant = escape(ctx.tenant.as_str()),
        escaped = escaped,
    );
    Ok(page_shell(
        &format!("{id} · ADR Browser"),
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
        assert!(matches!(err, AdrViewError::Auth(AuthError::PersonaForbidden { .. })));
    }

    #[test]
    fn list_records_persona_gate_rejects_anonymous() {
        let err = list_records(&anon_ctx()).unwrap_err();
        assert!(matches!(err, AdrViewError::Auth(AuthError::PersonaForbidden { .. })));
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
        assert_eq!(extract_id("ADR-001-sovereign-bare-metal-hosting"), "ADR-001");
        assert_eq!(extract_id("ADR-100"), "ADR-100");
    }

    #[test]
    fn extract_status_recognises_each_phase() {
        assert_eq!(extract_status("Status: Accepted"), AdrStatus::Accepted);
        assert_eq!(extract_status("**Status:** Proposed"), AdrStatus::Proposed);
        assert_eq!(extract_status("status: Superseded"), AdrStatus::Superseded);
        assert_eq!(extract_status("no status line"), AdrStatus::Unknown);
    }
}
