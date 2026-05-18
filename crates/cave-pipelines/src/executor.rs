// SPDX-License-Identifier: AGPL-3.0-or-later
//! Step executor: runs steps as child processes with stdout/stderr capture,
//! exit code handling, and timeout enforcement.

use crate::engine::interpolate_params;
use crate::models::{ParameterValue, Step, StepLog};
use chrono::Utc;
use std::collections::HashMap;
use std::time::Duration;
use thiserror::Error;
use tracing::{info, warn};

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("Step '{step}' failed with exit code {exit_code}")]
    StepFailed { step: String, exit_code: i32 },
    #[error("Step '{step}' timed out after {seconds}s")]
    Timeout { step: String, seconds: u64 },
    #[error("IO error launching step '{step}': {source}")]
    Io { step: String, source: std::io::Error },
}

pub type ExecutorResult<T> = Result<T, ExecutorError>;

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

pub struct StepExecutor;

impl StepExecutor {
    /// Execute a single step as a child process.
    pub async fn execute(
        step: &Step,
        params: &[ParameterValue],
        env_extra: &HashMap<String, String>,
    ) -> ExecutorResult<StepLog> {
        let started_at = Utc::now();

        // Determine program + args
        let (program, args) = if let Some(script) = &step.script {
            let interpolated = interpolate_params(script, params);
            ("sh".to_string(), vec!["-c".to_string(), interpolated])
        } else {
            let prog = step
                .command
                .first()
                .map(|s| interpolate_params(s, params))
                .unwrap_or_else(|| "true".to_string());
            let a: Vec<String> = step
                .command
                .iter()
                .skip(1)
                .chain(step.args.iter())
                .map(|s| interpolate_params(s, params))
                .collect();
            (prog, a)
        };

        // Build environment
        let mut env: HashMap<String, String> = env_extra.clone();
        for p in params {
            let key = format!("PARAM_{}", p.name.to_uppercase().replace('-', "_"));
            env.insert(key, p.value.clone());
        }
        for e in &step.env {
            env.insert(e.name.clone(), interpolate_params(&e.value, params));
        }

        let timeout_secs = step.timeout_seconds.unwrap_or(3600);
        info!(step = %step.name, program = %program, "executing step");

        let mut cmd = tokio::process::Command::new(&program);
        cmd.args(&args).envs(&env);
        if let Some(wd) = &step.working_dir {
            cmd.current_dir(wd);
        }
        cmd.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let output =
            tokio::time::timeout(Duration::from_secs(timeout_secs), cmd.output())
                .await
                .map_err(|_| ExecutorError::Timeout {
                    step: step.name.clone(),
                    seconds: timeout_secs,
                })?
                .map_err(|e| ExecutorError::Io {
                    step: step.name.clone(),
                    source: e,
                })?;

        let exit_code = output.status.code().unwrap_or(-1);
        let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
        let completed_at = Utc::now();

        if !output.status.success() {
            warn!(step = %step.name, exit_code, "step failed");
            return Err(ExecutorError::StepFailed {
                step: step.name.clone(),
                exit_code,
            });
        }

        Ok(StepLog { step_name: step.name.clone(), stdout, stderr, exit_code, started_at, completed_at })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{EnvVar, Step};

    fn simple_step(name: &str, command: Vec<&str>) -> Step {
        Step {
            name: name.to_string(),
            image: None,
            command: command.into_iter().map(String::from).collect(),
            args: vec![],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(10),
            script: None,
        }
    }

    #[tokio::test]
    async fn test_step_success_exit_zero() {
        let step = simple_step("echo", vec!["echo", "hello"]);
        let log = StepExecutor::execute(&step, &[], &HashMap::new()).await.unwrap();
        assert_eq!(log.exit_code, 0);
        assert!(log.stdout.contains("hello"));
    }

    #[tokio::test]
    async fn test_step_failure_nonzero_exit() {
        let step = simple_step("fail", vec!["false"]);
        let result = StepExecutor::execute(&step, &[], &HashMap::new()).await;
        assert!(matches!(result, Err(ExecutorError::StepFailed { exit_code: 1, .. })));
    }

    #[tokio::test]
    async fn test_step_stderr_captured() {
        let step = Step {
            name: "stderr".to_string(),
            image: None,
            command: vec![],
            args: vec![],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(10),
            script: Some("echo error_msg >&2".to_string()),
        };
        let log = StepExecutor::execute(&step, &[], &HashMap::new()).await.unwrap();
        assert!(log.stderr.contains("error_msg"));
    }

    #[tokio::test]
    async fn test_step_with_script_and_param() {
        let step = Step {
            name: "script-step".to_string(),
            image: None,
            command: vec![],
            args: vec![],
            env: vec![],
            working_dir: None,
            timeout_seconds: Some(10),
            script: Some("echo $(params.greeting)".to_string()),
        };
        let params = vec![ParameterValue { name: "greeting".to_string(), value: "world".to_string() }];
        let log = StepExecutor::execute(&step, &params, &HashMap::new()).await.unwrap();
        assert!(log.stdout.contains("world"));
    }

    #[tokio::test]
    async fn test_step_env_var_injection() {
        let step = Step {
            name: "env-step".to_string(),
            image: None,
            command: vec!["sh".to_string(), "-c".to_string(), "echo $MY_VAR".to_string()],
            args: vec![],
            env: vec![EnvVar { name: "MY_VAR".to_string(), value: "injected".to_string() }],
            working_dir: None,
            timeout_seconds: Some(10),
            script: None,
        };
        let log = StepExecutor::execute(&step, &[], &HashMap::new()).await.unwrap();
        assert!(log.stdout.contains("injected"));
    }
}
