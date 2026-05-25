// SPDX-License-Identifier: AGPL-3.0-or-later
//! `/admin/vulns/products` — ProductType / Product hierarchy browser.
//!
//! Source: DefectDojo/django-DefectDojo@6eab8738 dojo/models.py:839,1128

use crate::admin::layout::shell::{ShellOptions, shell_v2};
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;
use crate::admin::vulns::VulnsViewError;

/// Reference data — DefectDojo Product business_criticality enum.
pub const BUSINESS_CRITICALITY: &[&str] = &["VeryHigh", "High", "Medium", "Low", "VeryLow", "None"];

pub const LIFECYCLE: &[&str] = &["Construction", "Production", "Retirement"];

pub fn render(_state: &AdminState, ctx: &RequestCtx) -> Result<String, VulnsViewError> {
    ctx.authorise(Permission::VulnsRead)?;
    let body = format!(
        r#"<section>
  <h2>Products</h2>
  <p>Hierarchy: <strong>ProductType → Product → Engagement → Test → Finding</strong>.</p>
  <p>Source: <code>cave_vulns::hierarchy</code> module (DefectDojo
  <code>dojo/models.py</code> 839/1128/1535/2163).</p>
  <h3>business_criticality enum ({n_b})</h3>
  {bc}
  <h3>lifecycle enum ({n_l})</h3>
  {lc}
  <h3>Live</h3>
  <p>Use <code>POST /api/vulns/product-types</code> and
  <code>POST /api/vulns/products</code> to create entries.
  List: <code>GET /api/vulns/products</code>.</p>
</section>"#,
        n_b = BUSINESS_CRITICALITY.len(),
        n_l = LIFECYCLE.len(),
        bc = table(
            &["criticality"],
            &BUSINESS_CRITICALITY
                .iter()
                .map(|s| vec![s.to_string()])
                .collect::<Vec<_>>()
        ),
        lc = table(
            &["lifecycle"],
            &LIFECYCLE
                .iter()
                .map(|s| vec![s.to_string()])
                .collect::<Vec<_>>()
        ),
    );
    Ok(shell_v2(ShellOptions {
        title: "vulns · products",
        persona: ctx.persona,
        tenant_id: ctx.tenant.as_str(),
        current_path: "/admin/vulns/products",
        body: &body,
        ..Default::default()
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn business_criticality_has_six_values() {
        assert_eq!(BUSINESS_CRITICALITY.len(), 6);
    }
    #[test]
    fn lifecycle_has_three_values() {
        assert_eq!(LIFECYCLE.len(), 3);
    }
    #[test]
    fn render_includes_hierarchy() {
        let ctx = RequestCtx::developer("acme", &[Permission::VulnsRead]);
        let html = render(&AdminState::seeded(), &ctx).unwrap();
        assert!(html.contains("ProductType"));
    }
}
