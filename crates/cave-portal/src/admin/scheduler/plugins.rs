// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Plugins tab — active scheduling-framework plugins per phase.
//! Wraps the legacy `list_policies` / CRUD path (treats each policy
//! as a Score-phase plugin) plus a phase taxonomy.

use super::SchedulerViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::{scope, AdminState, SchedulerNode, SchedulerPolicy};

pub fn list_nodes(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<SchedulerNode>, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    Ok(scope(&state.scheduler_nodes.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect())
}

pub fn list_policies(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<SchedulerPolicy>, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    Ok(scope(&state.scheduler_policies.read().unwrap(), &ctx.tenant, |r| {
        &r.tenant
    })
    .into_iter()
    .cloned()
    .collect())
}

pub fn create_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
    predicate: &str,
    weight: u32,
) -> Result<(), SchedulerViewError> {
    ctx.authorise(Permission::SchedulerWrite)?;
    if predicate.trim().is_empty() {
        return Err(SchedulerViewError::EmptyPredicate);
    }
    if !(1..=100).contains(&weight) {
        return Err(SchedulerViewError::InvalidWeight);
    }
    let mut policies = state.scheduler_policies.write().unwrap();
    if policies.iter().any(|p| p.tenant == ctx.tenant && p.name == name) {
        return Err(SchedulerViewError::DuplicatePolicy(name.into()));
    }
    policies.push(SchedulerPolicy {
        tenant: ctx.tenant.clone(),
        name: name.into(),
        predicate: predicate.into(),
        weight,
    });
    Ok(())
}

pub fn delete_policy(
    state: &AdminState,
    ctx: &RequestCtx,
    name: &str,
) -> Result<(), SchedulerViewError> {
    ctx.authorise(Permission::SchedulerWrite)?;
    let mut policies = state.scheduler_policies.write().unwrap();
    let before = policies.len();
    policies.retain(|p| !(p.tenant == ctx.tenant && p.name == name));
    if policies.len() == before {
        return Err(SchedulerViewError::PolicyNotFound(name.into()));
    }
    Ok(())
}

/// One row in the scheduling-framework plugin catalogue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRow {
    pub phase: &'static str, // "PreFilter" | "Filter" | "Score" | "Reserve" | "Permit"
    pub name: &'static str,
    pub enabled: bool,
}

pub fn list_plugins(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<PluginRow>, SchedulerViewError> {
    ctx.authorise(Permission::SchedulerRead)?;
    Ok(vec![
        // PreFilter
        PluginRow { phase: "PreFilter", name: "NodeResourcesFit",      enabled: true },
        PluginRow { phase: "PreFilter", name: "InterPodAffinity",      enabled: true },
        PluginRow { phase: "PreFilter", name: "VolumeBinding",         enabled: true },
        // Filter
        PluginRow { phase: "Filter", name: "NodeUnschedulable",        enabled: true },
        PluginRow { phase: "Filter", name: "NodeName",                 enabled: true },
        PluginRow { phase: "Filter", name: "NodePorts",                enabled: true },
        PluginRow { phase: "Filter", name: "NodeAffinity",             enabled: true },
        PluginRow { phase: "Filter", name: "TaintToleration",          enabled: true },
        PluginRow { phase: "Filter", name: "VolumeRestrictions",       enabled: true },
        PluginRow { phase: "Filter", name: "VolumeBinding",            enabled: true },
        // Score
        PluginRow { phase: "Score", name: "NodeResourcesBalancedAllocation", enabled: true },
        PluginRow { phase: "Score", name: "NodeResourcesFit",          enabled: true },
        PluginRow { phase: "Score", name: "ImageLocality",             enabled: true },
        PluginRow { phase: "Score", name: "InterPodAffinity",          enabled: true },
        PluginRow { phase: "Score", name: "NodeAffinity",              enabled: true },
        PluginRow { phase: "Score", name: "PodTopologySpread",         enabled: true },
        PluginRow { phase: "Score", name: "TaintToleration",           enabled: true },
        // Reserve / Permit
        PluginRow { phase: "Reserve", name: "VolumeBinding",           enabled: true },
        PluginRow { phase: "Permit", name: "DefaultBinder",            enabled: true },
    ])
}

pub fn count_enabled(rows: &[PluginRow]) -> usize {
    rows.iter().filter(|r| r.enabled).count()
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, SchedulerViewError> {
    let plugins = list_plugins(state, ctx)?;
    let policies = list_policies(state, ctx)?;
    let plugin_rows: Vec<Vec<String>> = plugins
        .iter()
        .map(|p| vec![p.phase.into(), p.name.into(), if p.enabled { "Enabled" } else { "Disabled" }.into()])
        .collect();
    let policy_rows: Vec<Vec<String>> = policies
        .iter()
        .map(|p| vec![p.name.clone(), p.predicate.clone(), p.weight.to_string()])
        .collect();
    Ok(format!(
        r#"<section id="scheduler-plugins" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Plugins ({n} phase entries, {e} Enabled)</h2>
  {plugin_tbl}
  <h3 class="text-md font-semibold mt-3 mb-1">Custom policies ({np})</h3>
  {policy_tbl}
</section>"#,
        n = plugins.len(),
        e = count_enabled(&plugins),
        np = policies.len(),
        plugin_tbl = table(&["phase", "plugin", "state"], &plugin_rows),
        policy_tbl = table(&["name", "predicate", "weight"], &policy_rows),
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
    fn list_plugins_includes_every_phase() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/Pods.tsx",
            "Plugins",
            "acme"
        );
        let s = AdminState::seeded();
        let plugins = list_plugins(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        let phases: std::collections::HashSet<_> = plugins.iter().map(|p| p.phase).collect();
        for p in ["PreFilter", "Filter", "Score", "Reserve", "Permit"] {
            assert!(phases.contains(p), "missing phase {p}");
        }
    }

    #[test]
    fn list_plugins_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_plugins(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn create_policy_appends_and_validates() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::SchedulerRead, Permission::SchedulerWrite]);
        create_policy(&s, &c, "spread", "topology=zone", 3).unwrap();
        assert!(matches!(
            create_policy(&s, &c, "spread", "topology=zone", 3).unwrap_err(),
            SchedulerViewError::DuplicatePolicy(_)
        ));
        assert!(matches!(
            create_policy(&s, &c, "x", "p", 0).unwrap_err(),
            SchedulerViewError::InvalidWeight
        ));
        assert!(matches!(
            create_policy(&s, &c, "y", "  ", 5).unwrap_err(),
            SchedulerViewError::EmptyPredicate
        ));
    }

    #[test]
    fn delete_policy_removes_and_errors_on_missing() {
        let s = AdminState::seeded();
        let c = ctx(&[Permission::SchedulerRead, Permission::SchedulerWrite]);
        delete_policy(&s, &c, "least-utilised").unwrap();
        assert!(matches!(
            delete_policy(&s, &c, "least-utilised").unwrap_err(),
            SchedulerViewError::PolicyNotFound(_)
        ));
    }

    #[test]
    fn render_section_emits_phase_and_policy_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for col in ["phase", "plugin", "state"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
        assert!(html.contains("Custom policies"));
    }
}
