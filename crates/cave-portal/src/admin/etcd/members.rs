// SPDX-License-Identifier: AGPL-3.0-or-later
//! Members tab — cluster membership (`etcdctl member list`).
//! Synthesised from a 3-node baseline so the page has the right
//! shape; a live cluster surfaces this via the etcd MemberList RPC.

use super::EtcdViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberRow {
    pub id: &'static str,
    pub name: &'static str,
    pub peer_url: &'static str,
    pub client_url: &'static str,
    pub is_learner: bool,
    pub state: &'static str, // "Leader" | "Follower" | "Candidate"
}

pub fn list_members(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<MemberRow>, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    Ok(vec![
        MemberRow {
            id: "8211f1d0f64f3269",
            name: "etcd-0",
            peer_url: "https://etcd-0.etcd.svc:2380",
            client_url: "https://etcd-0.etcd.svc:2379",
            is_learner: false,
            state: "Leader",
        },
        MemberRow {
            id: "91bc3c398fb3c146",
            name: "etcd-1",
            peer_url: "https://etcd-1.etcd.svc:2380",
            client_url: "https://etcd-1.etcd.svc:2379",
            is_learner: false,
            state: "Follower",
        },
        MemberRow {
            id: "fd422379fda50e48",
            name: "etcd-2",
            peer_url: "https://etcd-2.etcd.svc:2380",
            client_url: "https://etcd-2.etcd.svc:2379",
            is_learner: false,
            state: "Follower",
        },
    ])
}

pub fn leader_id(rows: &[MemberRow]) -> Option<&'static str> {
    rows.iter().find(|r| r.state == "Leader").map(|r| r.id)
}

pub(super) fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, EtcdViewError> {
    let rows = list_members(state, ctx)?;
    let table_rows: Vec<Vec<String>> = rows
        .iter()
        .map(|m| {
            vec![
                m.id.into(),
                m.name.into(),
                m.peer_url.into(),
                m.client_url.into(),
                if m.is_learner { "✓" } else { "" }.into(),
                m.state.into(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="etcd-members" class="mt-2">
  <h2 class="text-lg font-semibold mb-2">Members ({n}, leader {leader})</h2>
  {tbl}
</section>"#,
        n = rows.len(),
        leader = leader_id(&rows).unwrap_or("?"),
        tbl = table(
            &["id", "name", "peer url", "client url", "learner", "state"],
            &table_rows
        ),
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
    fn list_members_returns_three_node_baseline() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/explore/src/components/Tabs/DocsTab.tsx",
            "Members",
            "acme"
        );
        let s = AdminState::seeded();
        let m = list_members(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        assert_eq!(m.len(), 3);
    }

    #[test]
    fn list_members_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_members(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn leader_id_finds_leader() {
        let s = AdminState::seeded();
        let m = list_members(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        assert!(leader_id(&m).is_some());
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        for col in ["id", "name", "peer url", "client url", "learner", "state"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
