// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Cycle 4 (RED→GREEN): per-tool + per-user permission system.

use cave_tools::permission::{PermissionPolicy, Target};

#[test]
fn default_allow_lets_everything_through() {
    let p = PermissionPolicy::default_allow();
    assert!(p.check("alice", "file_read", "fs").is_ok());
}

#[test]
fn default_deny_blocks_unlisted() {
    let p = PermissionPolicy::default_deny();
    let err = p.check("alice", "file_read", "fs").unwrap_err();
    assert_eq!(err.code(), "permission_denied");
}

#[test]
fn per_user_per_tool_allow() {
    let p = PermissionPolicy::default_deny()
        .allow("alice", Target::tool("file_read"));
    assert!(p.check("alice", "file_read", "fs").is_ok());
    // Different user is still denied.
    assert!(p.check("bob", "file_read", "fs").is_err());
    // Different tool is still denied.
    assert!(p.check("alice", "file_write", "fs").is_err());
}

#[test]
fn toolset_wildcard_allow() {
    let p = PermissionPolicy::default_deny()
        .allow("alice", Target::toolset("fs"));
    assert!(p.check("alice", "file_read", "fs").is_ok());
    assert!(p.check("alice", "file_write", "fs").is_ok());
    assert!(p.check("alice", "web_search", "net").is_err());
}

#[test]
fn any_user_wildcard() {
    let p = PermissionPolicy::default_deny()
        .allow_any_user(Target::tool("calendar_list"));
    assert!(p.check("alice", "calendar_list", "calendar").is_ok());
    assert!(p.check("bob", "calendar_list", "calendar").is_ok());
}

#[test]
fn deny_overrides_allow() {
    // alice may use the whole fs toolset, but file_write is explicitly denied.
    let p = PermissionPolicy::default_deny()
        .allow("alice", Target::toolset("fs"))
        .deny("alice", Target::tool("file_write"));
    assert!(p.check("alice", "file_read", "fs").is_ok());
    let err = p.check("alice", "file_write", "fs").unwrap_err();
    assert_eq!(err.code(), "permission_denied");
}

#[test]
fn deny_overrides_default_allow() {
    let p = PermissionPolicy::default_allow()
        .deny_any_user(Target::tool("code_exec"));
    assert!(p.check("alice", "file_read", "fs").is_ok());
    assert!(p.check("alice", "code_exec", "code").is_err());
}

#[test]
fn filter_visible_keeps_only_authorized() {
    use cave_tools::tool::ToolSpec;
    use serde_json::json;
    let specs = vec![
        ToolSpec { name: "file_read".into(), description: "".into(), input_schema: json!({}), toolset: "fs".into() },
        ToolSpec { name: "code_exec".into(), description: "".into(), input_schema: json!({}), toolset: "code".into() },
    ];
    let p = PermissionPolicy::default_deny().allow("alice", Target::toolset("fs"));
    let visible = p.filter_visible("alice", &specs);
    assert_eq!(visible.len(), 1);
    assert_eq!(visible[0].name, "file_read");
}
