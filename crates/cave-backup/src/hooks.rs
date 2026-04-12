//! Pre/post backup hook validation.

use crate::types::{BackupHook, ExecHook, HookErrorMode};
use thiserror::Error;

#[derive(Error, Debug, PartialEq)]
pub enum HookValidationError {
    #[error("Hook '{name}' has empty command")]
    EmptyCommand { name: String },
    #[error("Hook '{name}' has zero timeout")]
    ZeroTimeout { name: String },
    #[error("Hook '{name}' has empty container")]
    EmptyContainer { name: String },
}

/// Validate a single exec hook.
pub fn validate_exec_hook(hook_name: &str, exec: &ExecHook) -> Result<(), HookValidationError> {
    if exec.container.is_empty() {
        return Err(HookValidationError::EmptyContainer {
            name: hook_name.to_string(),
        });
    }
    if exec.command.is_empty() {
        return Err(HookValidationError::EmptyCommand {
            name: hook_name.to_string(),
        });
    }
    if exec.timeout_seconds == 0 {
        return Err(HookValidationError::ZeroTimeout {
            name: hook_name.to_string(),
        });
    }
    Ok(())
}

/// Validate all hooks in a backup hook spec.
pub fn validate_backup_hook(hook: &BackupHook) -> Vec<HookValidationError> {
    let mut errors = Vec::new();
    for exec in &hook.pre_hooks {
        if let Err(e) = validate_exec_hook(&hook.name, exec) {
            errors.push(e);
        }
    }
    for exec in &hook.post_hooks {
        if let Err(e) = validate_exec_hook(&hook.name, exec) {
            errors.push(e);
        }
    }
    errors
}

/// Build a minimal valid exec hook for testing.
#[cfg(test)]
pub fn valid_exec_hook() -> ExecHook {
    ExecHook {
        container: "app".into(),
        command: vec!["sh".into(), "-c".into(), "echo done".into()],
        on_error: HookErrorMode::Continue,
        timeout_seconds: 30,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_exec_hook_passes() {
        let exec = valid_exec_hook();
        assert!(validate_exec_hook("my-hook", &exec).is_ok());
    }

    #[test]
    fn test_empty_container_fails() {
        let exec = ExecHook {
            container: "".into(),
            command: vec!["echo".into()],
            on_error: HookErrorMode::Fail,
            timeout_seconds: 10,
        };
        let err = validate_exec_hook("hook1", &exec).unwrap_err();
        assert!(matches!(err, HookValidationError::EmptyContainer { .. }));
    }

    #[test]
    fn test_empty_command_fails() {
        let exec = ExecHook {
            container: "app".into(),
            command: vec![],
            on_error: HookErrorMode::Continue,
            timeout_seconds: 10,
        };
        let err = validate_exec_hook("hook1", &exec).unwrap_err();
        assert!(matches!(err, HookValidationError::EmptyCommand { .. }));
    }

    #[test]
    fn test_zero_timeout_fails() {
        let exec = ExecHook {
            container: "app".into(),
            command: vec!["echo".into()],
            on_error: HookErrorMode::Continue,
            timeout_seconds: 0,
        };
        let err = validate_exec_hook("hook1", &exec).unwrap_err();
        assert!(matches!(err, HookValidationError::ZeroTimeout { .. }));
    }

    #[test]
    fn test_backup_hook_collects_all_errors() {
        let bad_exec = ExecHook {
            container: "".into(),
            command: vec![],
            on_error: HookErrorMode::Continue,
            timeout_seconds: 0,
        };
        let hook = BackupHook {
            name: "test-hook".into(),
            pod_selector: "app=myapp".into(),
            namespace: "default".into(),
            pre_hooks: vec![bad_exec],
            post_hooks: vec![],
        };
        let errors = validate_backup_hook(&hook);
        // EmptyContainer fires first (container check before command check).
        assert!(!errors.is_empty());
    }
}
