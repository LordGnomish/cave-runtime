// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Step executor: runs steps as child processes with stdout/stderr capture,
//! exit code handling, and timeout enforcement.
//!
//! Ports: pkg/reconciler/taskrun/taskrun.go (Tekton Pipelines v0.55.0)

use crate::engine::resolve_param_string;
use crate::models::{Param, ParamValue, Step};
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
// Step execution result
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct StepOutput {
    pub step_name: String,
    pub exit_code: i32,
    pub stdout: String,
    pub stderr: String,
    pub started_at: chrono::DateTime<Utc>,
    pub completed_at: chrono::DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

pub struct StepExecutor;

/// Build a params lookup map from a slice of Param.
fn params_map(params: &[Param]) -> HashMap<String, ParamValue> {
    params.iter().map(|p| (p.name.clone(), p.value.clone())).collect()
}

impl StepExecutor {
    /// Execute a single step as a child process.
    pub async fn execute(
        step: &Step,
        params: &[Param],
        env_extra: &HashMap<String, String>,
    ) -> ExecutorResult<StepOutput> {
        let started_at = Utc::now();
        let pm = params_map(params);
        let empty_results: HashMap<String, HashMap<String, ParamValue>> = HashMap::new();

        // Determine program + args
        let (program, args) = if let Some(script) = &step.script {
            let interpolated = resolve_param_string(script, &pm, &empty_results);
            ("sh".to_string(), vec!["-c".to_string(), interpolated])
        } else if let Some(cmd) = &step.command {
            let prog = cmd
                .first()
                .map(|s| resolve_param_string(s, &pm, &empty_results))
                .unwrap_or_else(|| "true".to_string());
            let a: Vec<String> = cmd
                .iter()
                .skip(1)
                .chain(step.args.iter())
                .map(|s| resolve_param_string(s, &pm, &empty_results))
                .collect();
            (prog, a)
        } else {
            ("true".to_string(), vec![])
        };

        // Build environment
        let mut env: HashMap<String, String> = env_extra.clone();
        for p in params {
            let key = format!("PARAM_{}", p.name.to_uppercase().replace('-', "_"));
            if let ParamValue::String(v) = &p.value {
                env.insert(key, v.clone());
            }
        }
        for e in &step.env {
            let val = e
                .value
                .as_deref()
                .map(|v| resolve_param_string(v, &pm, &empty_results))
                .unwrap_or_default();
            env.insert(e.name.clone(), val);
        }

        // Default 1 hour; parse Tekton duration string ("1h", "10m") if set.
        let timeout_secs = parse_timeout(step.timeout.as_deref()).unwrap_or(3600);
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

        Ok(StepOutput { step_name: step.name.clone(), stdout, stderr, exit_code, started_at, completed_at })
    }
}

/// Parse a Tekton duration string to seconds. Accepts "Xs", "Xm", "Xh".
fn parse_timeout(s: Option<&str>) -> Option<u64> {
    let s = s?;
    if let Some(h) = s.strip_suffix('h') {
        return h.parse::<u64>().ok().map(|v| v * 3600);
    }
    if let Some(m) = s.strip_suffix('m') {
        return m.parse::<u64>().ok().map(|v| v * 60);
    }
    if let Some(sec) = s.strip_suffix('s') {
        return sec.parse::<u64>().ok();
    }
    None
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
            image: "busybox".to_string(),
            command: Some(command.into_iter().map(String::from).collect()),
            args: vec![],
            env: vec![],
            volume_mounts: vec![],
            working_dir: None,
            timeout: Some("10s".to_string()),
            script: None,
            ref_: None,
            results: vec![],
            resources: None,
            security_context: None,
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
            image: "busybox".to_string(),
            command: None,
            args: vec![],
            env: vec![],
            volume_mounts: vec![],
            working_dir: None,
            timeout: Some("10s".to_string()),
            script: Some("echo error_msg >&2".to_string()),
            ref_: None,
            results: vec![],
            resources: None,
            security_context: None,
        };
        let log = StepExecutor::execute(&step, &[], &HashMap::new()).await.unwrap();
        assert!(log.stderr.contains("error_msg"));
    }

    #[tokio::test]
    async fn test_step_with_script_and_param() {
        let step = Step {
            name: "script-step".to_string(),
            image: "busybox".to_string(),
            command: None,
            args: vec![],
            env: vec![],
            volume_mounts: vec![],
            working_dir: None,
            timeout: Some("10s".to_string()),
            script: Some("echo $(params.greeting)".to_string()),
            ref_: None,
            results: vec![],
            resources: None,
            security_context: None,
        };
        let params = vec![Param { name: "greeting".to_string(), value: ParamValue::String("world".to_string()) }];
        let log = StepExecutor::execute(&step, &params, &HashMap::new()).await.unwrap();
        assert!(log.stdout.contains("world"));
    }

    #[tokio::test]
    async fn test_step_env_var_injection() {
        let step = Step {
            name: "env-step".to_string(),
            image: "busybox".to_string(),
            command: Some(vec!["sh".to_string(), "-c".to_string(), "echo $MY_VAR".to_string()]),
            args: vec![],
            env: vec![EnvVar {
                name: "MY_VAR".to_string(),
                value: Some("injected".to_string()),
                value_from: None,
            }],
            volume_mounts: vec![],
            working_dir: None,
            timeout: Some("10s".to_string()),
            script: None,
            ref_: None,
            results: vec![],
            resources: None,
            security_context: None,
        };
        let log = StepExecutor::execute(&step, &[], &HashMap::new()).await.unwrap();
        assert!(log.stdout.contains("injected"));
    }
}
