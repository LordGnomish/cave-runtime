// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Composition v2 pipeline executor — runs an ordered list of `Step`s through
//! the function registry, threading prior step's output into next step's input.
//!
//! Upstream: internal/controller/apiextensions/composite/composition_pipeline.go

use crate::composition::step::{Step, StepResult, StepSeverity};
use crate::error::{CrossplaneError, CrossplaneResult};
use crate::function::FunctionStore;
use crate::function::grpc_codec::{RunFunctionRequest, RunFunctionResponse};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PipelineResult {
    pub steps: Vec<StepResult>,
    pub desired: serde_json::Value,
    pub final_response: Option<RunFunctionResponse>,
}

impl PipelineResult {
    pub fn ok(&self) -> bool {
        !self.steps.iter().any(|s| s.severity == StepSeverity::Fatal)
    }
}

pub struct PipelineExecutor;

impl PipelineExecutor {
    pub fn new() -> Self {
        Self
    }

    /// Run a pipeline synchronously, in-process.
    pub fn run_sync(
        &self,
        steps: &[Step],
        store: &FunctionStore,
        initial: &RunFunctionRequest,
    ) -> CrossplaneResult<PipelineResult> {
        let mut req = initial.clone();
        let mut results: Vec<StepResult> = Vec::new();
        let mut last: Option<RunFunctionResponse> = None;
        let mut desired = serde_json::Value::Null;
        for step in steps {
            // Ensure function is installed
            if !store.contains(&step.function_ref) {
                results.push(StepResult::warn(
                    &step.step,
                    format!("function-ref not installed: {}", step.function_ref),
                ));
                continue;
            }
            // Set input from step
            if let Some(inp) = &step.input {
                req.input = inp.clone();
            }
            // Dispatch built-in functions inline; otherwise dispatch is a no-op (gRPC=Phase 2).
            let resp = dispatch_builtin(&step.function_ref, &req)?;
            results.push(StepResult::ok(&step.step));
            desired = resp.desired.clone();
            // Thread response.observed into next request as input context
            req.observed = resp.observed.clone();
            last = Some(resp);
        }
        Ok(PipelineResult {
            steps: results,
            desired,
            final_response: last,
        })
    }
}

impl Default for PipelineExecutor {
    fn default() -> Self {
        Self::new()
    }
}

/// Built-in dispatch table — function refs that the engine knows how to
/// evaluate in-process. Anything else returns Ok with empty desired.
pub fn dispatch_builtin(
    function_ref: &str,
    req: &RunFunctionRequest,
) -> CrossplaneResult<RunFunctionResponse> {
    match function_ref {
        "function-patch-and-transform" => {
            let desired = crate::function::patch_transform::run_patch_transform_fn(req)?;
            Ok(RunFunctionResponse::ready(desired))
        }
        "function-auto-ready" => {
            let mut resp = RunFunctionResponse::ready(req.observed.clone());
            let composed: Vec<serde_json::Value> = req
                .observed
                .get("composed")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let ready = crate::function::auto_ready::auto_ready_eval(&composed);
            resp.results.push(format!("auto-ready: {}", ready));
            Ok(resp)
        }
        "function-kcl" => {
            let src = req
                .input
                .get("source")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let v = crate::function::kcl::evaluate_kcl(src, &req.observed)?;
            Ok(RunFunctionResponse::ready(v))
        }
        "function-go-templating" => {
            let tmpl = req
                .input
                .get("template")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // Build render context: start from observed (the XR-side state),
            // then overlay anything from input (so callers can pass
            // {"template":"...","name":"world"}; `name` overrides observed.name).
            let mut ctx = req.observed.clone();
            if !ctx.is_object() {
                ctx = serde_json::Value::Object(serde_json::Map::new());
            }
            if let Some(input_obj) = req.input.as_object() {
                if let Some(ctx_obj) = ctx.as_object_mut() {
                    for (k, v) in input_obj {
                        if k != "template" && k != "context" {
                            ctx_obj.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            if let Some(explicit) = req.input.get("context") {
                if let (Some(c), Some(o)) = (explicit.as_object(), ctx.as_object_mut()) {
                    for (k, v) in c {
                        o.insert(k.clone(), v.clone());
                    }
                }
            }
            let rendered = crate::function::go_template::render_go_template(tmpl, &ctx)
                .map_err(|e| CrossplaneError::Internal(format!("go-template: {}", e)))?;
            Ok(RunFunctionResponse::ready(serde_json::json!({
                "rendered": rendered
            })))
        }
        _ => Ok(RunFunctionResponse::ready(serde_json::Value::Null)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn req() -> RunFunctionRequest {
        RunFunctionRequest::new("ctx", json!({}), json!({}))
    }

    #[test]
    fn empty_pipeline_ok() {
        let p = PipelineExecutor::new();
        let s = FunctionStore::new();
        let r = p.run_sync(&[], &s, &req()).unwrap();
        assert!(r.ok());
        assert!(r.steps.is_empty());
    }

    #[test]
    fn unknown_function_warns() {
        let p = PipelineExecutor::new();
        let s = FunctionStore::new();
        let steps = vec![Step::new("a", "function-x")];
        let r = p.run_sync(&steps, &s, &req()).unwrap();
        assert!(r.ok());
        assert_eq!(r.steps[0].severity, StepSeverity::Warning);
    }

    #[test]
    fn registered_function_runs() {
        let p = PipelineExecutor::new();
        let s = FunctionStore::new();
        s.install(
            "function-patch-and-transform",
            "v0.1.0",
            "xpkg.upbound.io/x/function-patch-and-transform:v0.1.0",
        )
        .unwrap();
        let steps = vec![
            Step::new("compose", "function-patch-and-transform")
                .with_input(json!({"resources":[]})),
        ];
        let r = p.run_sync(&steps, &s, &req()).unwrap();
        assert!(r.ok());
        assert_eq!(r.steps[0].severity, StepSeverity::Normal);
    }

    #[test]
    fn auto_ready_dispatch() {
        let s = FunctionStore::new();
        s.install("function-auto-ready", "v0.1.0", "x").unwrap();
        let p = PipelineExecutor::new();
        let req = RunFunctionRequest::new(
            "ctx",
            json!({"composed":[{"status":{"conditions":[{"type":"Ready","status":"True"}]}}]}),
            json!({}),
        );
        let r = p
            .run_sync(
                &[Step::new("ready", "function-auto-ready")],
                &s,
                &req,
            )
            .unwrap();
        assert!(r.ok());
    }

    #[test]
    fn go_template_dispatch_renders() {
        let s = FunctionStore::new();
        s.install("function-go-templating", "v0.1.0", "x").unwrap();
        let p = PipelineExecutor::new();
        // observed carries the template ctx; step input carries the template + override.
        let req = RunFunctionRequest::new(
            "ctx",
            json!({}),
            json!({"name":"world"}),
        );
        let r = p
            .run_sync(
                &[Step::new("t", "function-go-templating")
                    .with_input(json!({"template":"hi {{ .name }}"}))],
                &s,
                &req,
            )
            .unwrap();
        assert!(r.ok());
        let resp = r.final_response.unwrap();
        assert_eq!(resp.desired["rendered"], json!("hi world"));
    }

    #[test]
    fn pipeline_result_ok_with_fatal_returns_false() {
        let mut pr = PipelineResult::default();
        pr.steps.push(StepResult::fatal("a", "boom"));
        assert!(!pr.ok());
    }

    #[test]
    fn kcl_dispatch_returns_value() {
        let s = FunctionStore::new();
        s.install("function-kcl", "v0.1.0", "x").unwrap();
        let p = PipelineExecutor::new();
        let req = RunFunctionRequest::new("ctx", json!({}), json!({}));
        let r = p
            .run_sync(
                &[Step::new("kcl", "function-kcl")
                    .with_input(json!({"source":"x = 1"}))],
                &s,
                &req,
            )
            .unwrap();
        assert!(r.ok());
    }

    #[test]
    fn dispatch_unknown_returns_null() {
        let r = dispatch_builtin("unknown-fn", &req()).unwrap();
        assert_eq!(r.desired, serde_json::Value::Null);
    }
}
