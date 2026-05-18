// SPDX-License-Identifier: AGPL-3.0-or-later
//! Validations tab — Istio config linting results.
//!
//! Kiali's validation pipeline checks for issues like
//! `KIA0203` (host mismatch), `KIA1102` (gateway with no
//! virtualservice). Here we synthesise a small but representative
//! checker derived from the seeded mesh state.

use super::KialiViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::{escape, table};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationRow {
    pub code: &'static str,
    pub severity: &'static str, // "Error" | "Warning" | "Info"
    pub object_kind: String,
    pub object_name: String,
    pub message: String,
}

pub fn list_validations(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<ValidationRow>, KialiViewError> {
    let services = super::services::list_services(state, ctx)?;
    let traffic = super::traffic::list_traffic_rules(state, ctx)?;
    let mut out = Vec::new();
    // KIA1102 — Gateway present but no VirtualService binds to it.
    let vs_count = super::traffic::count_by_kind(&traffic, "VirtualService");
    if vs_count == 0 && super::traffic::count_by_kind(&traffic, "Gateway") > 0 {
        out.push(ValidationRow {
            code: "KIA1102",
            severity: "Warning",
            object_kind: "Gateway".into(),
            object_name: "mesh-gateway".into(),
            message: "Gateway has no VirtualService bound to it".into(),
        });
    }
    // KIA0203 — services without a DestinationRule on a busy path.
    for s in &services {
        if !s.destination_rule && s.traffic_rpm > 0 {
            out.push(ValidationRow {
                code: "KIA0203",
                severity: "Info",
                object_kind: "Service".into(),
                object_name: s.name.clone(),
                message: format!(
                    "Service {} has traffic but no DestinationRule — consider adding subsets",
                    s.name
                ),
            });
        }
    }
    Ok(out)
}

pub fn count_by_severity(rows: &[ValidationRow], severity: &str) -> usize {
    rows.iter().filter(|r| r.severity == severity).count()
}

pub(crate) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KialiViewError> {
    let rows = list_validations(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.code.into(),
                r.severity.into(),
                r.object_kind.clone(),
                r.object_name.clone(),
                escape(&r.message),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kiali-validations" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Validations ({n})</h2>
  <div class="text-xs text-gray-500 mb-2">
    {e} Error · {w} Warning · {i} Info
  </div>
  {tbl}
</section>"#,
        n = rows.len(),
        e = count_by_severity(&rows, "Error"),
        w = count_by_severity(&rows, "Warning"),
        i = count_by_severity(&rows, "Info"),
        tbl = table(&["code", "severity", "kind", "name", "message"], &table_rows),
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
    fn list_validations_runs_without_panic() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kiali/src/components/Validations.tsx",
            "Validations",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_validations(&s, &ctx(&[Permission::KialiRead])).unwrap();
        // Codes that do appear should match the known set.
        for r in &rows {
            assert!(r.code.starts_with("KIA"));
        }
    }

    #[test]
    fn list_validations_refuses_without_permission() {
        let s = AdminState::seeded();
        assert!(list_validations(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn count_by_severity_filters() {
        let s = AdminState::seeded();
        let rows = list_validations(&s, &ctx(&[Permission::KialiRead])).unwrap();
        let total = count_by_severity(&rows, "Error")
            + count_by_severity(&rows, "Warning")
            + count_by_severity(&rows, "Info");
        assert_eq!(total, rows.len());
    }

    #[test]
    fn render_section_emits_severity_legend() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KialiRead])).unwrap();
        assert!(html.contains("Error"));
        assert!(html.contains("Warning"));
        assert!(html.contains("Info"));
    }
}
