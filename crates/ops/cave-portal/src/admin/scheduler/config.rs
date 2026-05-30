// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Config tab — KubeSchedulerConfiguration validation rules + API surface.
//!
//! Mirrors the operator-facing view of `pkg/scheduler/apis/config/validation`.
//! The cave-scheduler backend validates a posted configuration via
//! `POST /api/scheduler/config/validate`, returning `{valid, errors}`; this tab
//! documents the rules that endpoint enforces.

use super::SchedulerViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

/// One validation rule the scheduler config validator enforces.
pub struct ValidationRule {
    pub field: &'static str,
    pub rule: &'static str,
}

/// The validation rules surfaced to operators, mirroring the backend's
/// `KubeSchedulerConfiguration::validate`.
pub fn validation_rules() -> Vec<ValidationRule> {
    vec![
        ValidationRule {
            field: "parallelism",
            rule: "must be a positive value (> 0)",
        },
        ValidationRule {
            field: "percentageOfNodesToScore",
            rule: "when set, must be in range [0, 100]",
        },
        ValidationRule {
            field: "profiles",
            rule: "at least one profile is required",
        },
        ValidationRule {
            field: "profiles[].schedulerName",
            rule: "required and unique across profiles",
        },
        ValidationRule {
            field: "profiles[].plugins.score.weight",
            rule: "positive; MaxNodeScore × weight must not overflow int32",
        },
        ValidationRule {
            field: "profiles[].plugins.score.enabled",
            rule: "a plugin may not be enabled more than once",
        },
        ValidationRule {
            field: "profiles[].pluginConfig.name",
            rule: "plugin config names must be unique within a profile",
        },
    ]
}

pub fn render_section(_state: &AdminState, ctx: &RequestCtx) -> Result<String, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    let rows = validation_rules();
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| vec![r.field.to_string(), r.rule.to_string()])
        .collect();
    Ok(format!(
        r#"<section id="scheduler-config" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Configuration validation ({n} rules)</h2>
  <p class="text-sm text-gray-600 mb-2">
    Validate a <code>KubeSchedulerConfiguration</code> with
    <code>POST /api/scheduler/config/validate</code> or
    <code>cavectl scheduler validate-config --file &lt;path&gt;</code>.
    Errors aggregate like Kubernetes' <code>field.ErrorList</code>.
  </p>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(&["field", "rule"], &table_rows),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_section_requires_scheduler_read() {
        let s = AdminState::seeded();
        assert!(render_section(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn render_section_lists_core_rules() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        assert!(html.contains(r#"id="scheduler-config""#));
        assert!(html.contains("parallelism"));
        assert!(html.contains("percentageOfNodesToScore"));
        assert!(html.contains("/api/scheduler/config/validate"));
    }

    #[test]
    fn validation_rules_cover_all_seven_checks() {
        assert_eq!(validation_rules().len(), 7);
    }
}
