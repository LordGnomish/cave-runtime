// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Raft tab — surfaces the in-process Raft consensus state machine
//! (`cave-etcd` `src/raft_node.rs`) and its election-simulation endpoint.
//!
//! cave-etcd ports the etcd core `raft` package data-plane (role
//! transitions, election/heartbeat clocks, vote granting/tally, leader
//! log replication + quorum commit, follower append). The live state
//! machine is driven by the cave-runtime cluster transport; this tab
//! documents the surface and the read-only simulate endpoint.

use super::EtcdViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::state::AdminState;

pub(super) fn render_section(
    _state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, EtcdViewError> {
    ctx.authorise(Permission::EtcdRead)?;
    Ok(r##"<section id="etcd-raft" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Raft consensus</h2>
  <p class="text-sm text-gray-700 mb-2">
    In-process Raft state machine (etcd <code>raft.go</code> data-plane):
    leader election (randomized timeouts, PreVote-aware vote granting and
    tally over the joint voter config) and the log-replication round-trip
    (leader <code>propose</code> + quorum commit under the Figure-8
    current-term rule, follower append with prev-entry match check). The
    network message driver lives in the cave-runtime cluster transport.
  </p>
  <p class="text-sm text-gray-700">
    Preview an election round:
    <code>POST /api/etcd/v3/raft/election/simulate</code>
    <span class="text-gray-500">
      (body: <code>{"id":1,"peers":[1,2,3],"grants":[2]}</code> →
      <code>{"state":"Leader","term":1,"lead":1}</code>)
    </span>, or
    <code>cavectl etcd raft-election --peers 1,2,3 --grant 2</code>.
  </p>
</section>"##
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_section_describes_raft_and_endpoint() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::EtcdRead])).unwrap();
        assert!(html.contains("id=\"etcd-raft\""));
        assert!(html.contains("/api/etcd/v3/raft/election/simulate"));
        assert!(html.contains("raft-election"));
    }

    #[test]
    fn render_section_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(render_section(&s, &ctx(&[])).is_err());
    }
}
