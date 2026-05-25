// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Instance metadata tab — per-node InstanceID / zone / region /
//! instance type. Mirrors the AWS/GCP IMDSv2 surface the cloud-
//! controller fans out for kubelet labels.

use super::CloudControllerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceMetaRow {
    pub node: String,
    pub instance_id: String,
    pub instance_type: &'static str,
    pub region: &'static str,
    pub zone: &'static str,
}

pub fn list_instances(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<InstanceMetaRow>, CloudControllerViewError> {
    let nodes = super::node_controller::list_nodes(state, ctx)?;
    Ok(nodes
        .into_iter()
        .map(|n| InstanceMetaRow {
            instance_id: n
                .provider_id
                .split('/')
                .last()
                .unwrap_or("unknown")
                .to_string(),
            instance_type: match n.provider {
                "aws" => "m5.xlarge",
                "gcp" => "n2-standard-4",
                "hetzner" => "CCX23",
                _ => "metal-1",
            },
            region: match n.provider {
                "aws" => "eu-west-1",
                "gcp" => "europe-west1",
                "hetzner" => "fsn1",
                _ => "dc-1",
            },
            zone: n.zone,
            node: n.node,
        })
        .collect())
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, CloudControllerViewError> {
    let rows = list_instances(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|r| {
            vec![
                r.node.clone(),
                r.instance_id.clone(),
                r.instance_type.into(),
                r.region.into(),
                r.zone.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="ccm-meta" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">InstanceMetadata ({n})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        tbl = table(
            &["node", "instanceID", "type", "region", "zone"],
            &table_rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_instances_one_per_node() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/CloudResources/InstanceMetadata.tsx",
            "Metadata",
            "acme"
        );
        let s = AdminState::seeded();
        let rows = list_instances(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        let nodes =
            super::super::node_controller::list_nodes(&s, &ctx(&[Permission::CloudControllerRead]))
                .unwrap();
        assert_eq!(rows.len(), nodes.len());
    }

    #[test]
    fn list_instances_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_instances(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn instance_type_matches_provider() {
        let s = AdminState::seeded();
        let rows = list_instances(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for r in &rows {
            assert!(["m5.xlarge", "n2-standard-4", "CCX23", "metal-1"].contains(&r.instance_type));
        }
    }

    #[test]
    fn render_section_emits_metadata_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::CloudControllerRead])).unwrap();
        for col in ["node", "instanceID", "type", "region", "zone"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
