// SPDX-License-Identifier: AGPL-3.0-or-later
//! Commands tab — operator mutators: SAVE, BGREWRITEAOF, failover.

use super::super::CacheViewError;
use crate::admin::permission::{Permission, RequestCtx};
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ClusterCommand {
    SnapshotNow,
    RewriteAof,
    FailoverNode(String),
    ReplayAof,
    Reshard {
        from_node_id: String,
        to_node_id: String,
        slots: u16,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub command: &'static str,
    pub ok: bool,
    pub message: String,
}

pub fn execute(
    _state: &AdminState,
    ctx: &RequestCtx,
    cmd: ClusterCommand,
) -> Result<CommandResult, CacheViewError> {
    ctx.authorise(Permission::CacheWrite)?;
    Ok(match cmd {
        ClusterCommand::SnapshotNow => CommandResult {
            command: "SAVE",
            ok: true,
            message: "RDB snapshot scheduled".into(),
        },
        ClusterCommand::RewriteAof => CommandResult {
            command: "BGREWRITEAOF",
            ok: true,
            message: "Background AOF rewrite started".into(),
        },
        ClusterCommand::FailoverNode(id) => CommandResult {
            command: "CLUSTER FAILOVER",
            ok: true,
            message: format!("Failover initiated for {id}"),
        },
        ClusterCommand::ReplayAof => CommandResult {
            command: "DEBUG RELOAD",
            ok: true,
            message: "AOF replay started; the node will be unavailable briefly".into(),
        },
        ClusterCommand::Reshard {
            from_node_id,
            to_node_id,
            slots,
        } => {
            if from_node_id == to_node_id {
                return Ok(CommandResult {
                    command: "CLUSTER SETSLOT",
                    ok: false,
                    message: "source and destination must differ".into(),
                });
            }
            if slots == 0 {
                return Ok(CommandResult {
                    command: "CLUSTER SETSLOT",
                    ok: false,
                    message: "slots must be > 0".into(),
                });
            }
            CommandResult {
                command: "CLUSTER SETSLOT",
                ok: true,
                message: format!(
                    "Reshard {slots} slot(s) from {from_node_id} to {to_node_id} scheduled"
                ),
            }
        }
    })
}

pub(super) fn render_section(
    _state: &AdminState,
    _ctx: &RequestCtx,
) -> Result<String, CacheViewError> {
    Ok(r#"<section id="cache-cluster-commands" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Commands</h2>
  <ul class="text-sm leading-7">
    <li><code>SAVE</code> — produce an RDB snapshot now</li>
    <li><code>BGREWRITEAOF</code> — rewrite the AOF in the background</li>
    <li><code>DEBUG RELOAD</code> — flush + replay AOF from disk</li>
    <li><code>CLUSTER FAILOVER &lt;node&gt;</code> — promote a replica to master</li>
    <li><code>CLUSTER SETSLOT &lt;from&gt; &lt;to&gt; &lt;slots&gt;</code> — start a reshard</li>
  </ul>
  <p class="text-xs text-gray-500 mt-2">Mutators require <code>CacheWrite</code>.</p>
</section>"#
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_admin() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::CacheRead, Permission::CacheWrite])
    }

    fn ctx_ro() -> RequestCtx {
        RequestCtx::developer("acme", &[Permission::CacheRead])
    }

    #[test]
    fn snapshot_now_returns_save_command() {
        let r = execute(&AdminState::seeded(), &ctx_admin(), ClusterCommand::SnapshotNow).unwrap();
        assert_eq!(r.command, "SAVE");
        assert!(r.ok);
    }

    #[test]
    fn rewrite_aof_returns_bgrewriteaof() {
        let r =
            execute(&AdminState::seeded(), &ctx_admin(), ClusterCommand::RewriteAof).unwrap();
        assert_eq!(r.command, "BGREWRITEAOF");
    }

    #[test]
    fn failover_includes_node_id_in_message() {
        let r = execute(
            &AdminState::seeded(),
            &ctx_admin(),
            ClusterCommand::FailoverNode("n123".into()),
        )
        .unwrap();
        assert!(r.message.contains("n123"));
    }

    #[test]
    fn replay_aof_returns_debug_reload() {
        let r = execute(&AdminState::seeded(), &ctx_admin(), ClusterCommand::ReplayAof).unwrap();
        assert_eq!(r.command, "DEBUG RELOAD");
    }

    #[test]
    fn reshard_rejects_same_source_and_destination() {
        let r = execute(
            &AdminState::seeded(),
            &ctx_admin(),
            ClusterCommand::Reshard {
                from_node_id: "a".into(),
                to_node_id: "a".into(),
                slots: 5,
            },
        )
        .unwrap();
        assert!(!r.ok);
    }

    #[test]
    fn reshard_rejects_zero_slot_count() {
        let r = execute(
            &AdminState::seeded(),
            &ctx_admin(),
            ClusterCommand::Reshard {
                from_node_id: "a".into(),
                to_node_id: "b".into(),
                slots: 0,
            },
        )
        .unwrap();
        assert!(!r.ok);
    }

    #[test]
    fn refuses_execute_without_cache_write() {
        let err = execute(&AdminState::seeded(), &ctx_ro(), ClusterCommand::SnapshotNow);
        assert!(err.is_err());
    }
}
