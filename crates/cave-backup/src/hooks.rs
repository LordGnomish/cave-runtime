// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Backup and restore hook validation utilities.

use crate::models::{BackupHook, ExecHook, HookErrorMode};

/// Validate hooks, returning a list of error strings for any misconfigured hooks.
pub fn validate_hooks(hooks: &[BackupHook]) -> Vec<String> {
    let mut errors = Vec::new();
    for hook in hooks {
        for pre in &hook.pre {
            if pre.command.is_empty() {
                errors.push(format!("hook {}: pre-exec command is empty", hook.name));
            }
        }
        for post in &hook.post {
            if post.command.is_empty() {
                errors.push(format!("hook {}: post-exec command is empty", hook.name));
            }
        }
    }
    errors
}

/// Create a default exec hook with Continue-on-error and 30s timeout.
pub fn default_exec_hook(container: &str, command: Vec<String>) -> ExecHook {
    ExecHook {
        container: container.to_string(),
        command,
        on_error: HookErrorMode::Continue,
        timeout_seconds: 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_hook(name: &str, pre_cmd: Option<Vec<String>>, post_cmd: Option<Vec<String>>) -> BackupHook {
        let pre = pre_cmd
            .map(|cmd| vec![default_exec_hook("app", cmd)])
            .unwrap_or_default();
        let post = post_cmd
            .map(|cmd| vec![default_exec_hook("app", cmd)])
            .unwrap_or_default();
        BackupHook {
            name: name.into(),
            namespace_selector: None,
            resource_selector: None,
            pre,
            post,
        }
    }

    #[test]
    fn validate_hooks_valid() {
        let hooks = vec![make_hook(
            "freeze",
            Some(vec!["fsfreeze".into(), "-f".into(), "/data".into()]),
            Some(vec!["fsfreeze".into(), "-u".into(), "/data".into()]),
        )];
        assert!(validate_hooks(&hooks).is_empty());
    }

    #[test]
    fn validate_hooks_catches_empty_pre_command() {
        let hooks = vec![make_hook("bad-pre", Some(vec![]), None)];
        let errors = validate_hooks(&hooks);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad-pre"));
        assert!(errors[0].contains("pre-exec"));
    }

    #[test]
    fn validate_hooks_catches_empty_post_command() {
        let hooks = vec![make_hook("bad-post", None, Some(vec![]))];
        let errors = validate_hooks(&hooks);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("bad-post"));
        assert!(errors[0].contains("post-exec"));
    }

    #[test]
    fn validate_hooks_multiple_errors() {
        let hooks = vec![
            make_hook("h1", Some(vec![]), None),
            make_hook("h2", None, Some(vec![])),
        ];
        assert_eq!(validate_hooks(&hooks).len(), 2);
    }

    #[test]
    fn default_exec_hook_defaults() {
        let hook = default_exec_hook("mycontainer", vec!["echo".into(), "hello".into()]);
        assert_eq!(hook.container, "mycontainer");
        assert_eq!(hook.command, vec!["echo", "hello"]);
        assert_eq!(hook.on_error, HookErrorMode::Continue);
        assert_eq!(hook.timeout_seconds, 30);
    }

    #[test]
    fn validate_hooks_empty_slice() {
        assert!(validate_hooks(&[]).is_empty());
    }
}
