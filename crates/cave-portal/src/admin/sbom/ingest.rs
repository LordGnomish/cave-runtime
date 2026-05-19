// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/sbom/ingest` — Dependency-Track "BOM upload" panel. Static
//! form HTML; live POST routes to cave-sbom's `/api/v1/bom`.
//!
//! Upstream: <https://dependencytrack.org/docs/usage/bom-uploads/>

use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};
use crate::admin::state::AdminState;
use super::SbomViewError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IngestStats {
    pub supported_formats: Vec<&'static str>,
    pub max_size_mb: u32,
}

pub fn current_stats() -> IngestStats {
    IngestStats {
        supported_formats: vec!["CycloneDX 1.4/1.5/1.6 JSON", "CycloneDX 1.4/1.5/1.6 XML", "SPDX 2.3 JSON", "SPDX 2.3 tag-value"],
        max_size_mb: 50,
    }
}

pub fn render(state: &AdminState, ctx: &RequestCtx) -> Result<String, SbomViewError> {
    ctx.authorise(Permission::SbomRead)?;
    let _ = state;
    let stats = current_stats();
    let formats_html: String = stats
        .supported_formats
        .iter()
        .map(|f| format!(r#"<li class="ml-4 list-disc">{}</li>"#, escape(f)))
        .collect();
    let body = format!(
        r#"<section>
  <h2 class="text-lg font-semibold mb-2">Upload SBOM</h2>
  <p class="text-sm text-gray-600 mb-3">POST a BOM to <code>/api/v1/bom</code> (base64-encoded body). Max size {sz} MiB.</p>
  <h3 class="font-semibold mt-3">Supported formats</h3>
  <ul>{li}</ul>
  <form class="mt-4" hx-post="/api/v1/bom" hx-encoding="multipart/form-data">
    <label class="block text-sm font-medium">Project UUID (optional)
      <input class="border rounded p-1 w-full" name="project_uuid"/>
    </label>
    <label class="block text-sm font-medium mt-2">BOM content (paste raw JSON/XML; the route accepts base64 wrapper)
      <textarea class="border rounded p-1 w-full" rows="10" name="bom" placeholder="{{...}}"></textarea>
    </label>
    <button class="mt-2 px-3 py-1 bg-blue-600 text-white rounded" type="submit">Upload</button>
  </form>
</section>"#,
        sz = stats.max_size_mb,
        li = formats_html,
    );
    Ok(page_shell_full(
        ctx,
        "/admin/sbom/ingest",
        &format!("sbom/ingest · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ctx(perms: &[Permission]) -> RequestCtx { RequestCtx::developer("acme", perms) }

    #[test]
    fn stats_include_all_four_formats() {
        let s = current_stats();
        assert!(s.supported_formats.iter().any(|f| f.contains("CycloneDX")));
        assert!(s.supported_formats.iter().any(|f| f.contains("SPDX")));
        assert_eq!(s.supported_formats.len(), 4);
    }

    #[test]
    fn render_requires_perm() {
        assert!(render(&AdminState::seeded(), &ctx(&[])).is_err());
    }

    #[test]
    fn render_includes_form_and_route() {
        let html = render(&AdminState::seeded(), &ctx(&[Permission::SbomRead])).unwrap();
        assert!(html.contains("/api/v1/bom"));
        assert!(html.contains("<form"));
        assert!(html.contains("CycloneDX"));
    }
}
