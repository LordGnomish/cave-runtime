// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Self-registering tool registry.
//!
//! Ports `tools/registry.py`. Hermes maintains a global mutable registry
//! that built-in tool modules populate at import time; we replace that
//! with an explicit [`ToolRegistry`] that callers construct and pass into
//! [`HermesRuntime`](crate::HermesRuntime). The four MVP built-ins live
//! in [`crate::tools_builtin`].
//!
//! Each entry carries:
//! * `name` — unique identifier exposed to the LLM
//! * `toolset` — coarse grouping (e.g. "fs", "shell", "net")
//! * `description` / `schema` — declared at registration time
//! * `check_fn` — optional gate (env var present, OS supported, …)
//! * `handler` — synchronous closure returning a [`ToolResult`]
//!
//! Async tools are out-of-scope for the MVP; the schema reserves an
//! `is_async` flag for the next sprint.

use std::collections::BTreeMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::HermesError;

/// Outcome of a tool invocation. Models the same `{"ok": bool, ...}` shape
/// that Hermes' Python tools return.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub output: String,
    /// Free-form structured metadata. Tools may surface byte counts,
    /// exit codes, redacted headers, etc.
    #[serde(default)]
    pub meta: BTreeMap<String, String>,
}

impl ToolResult {
    pub fn ok(output: impl Into<String>) -> Self {
        Self {
            ok: true,
            output: output.into(),
            meta: BTreeMap::new(),
        }
    }

    pub fn err(reason: impl Into<String>) -> Self {
        Self {
            ok: false,
            output: reason.into(),
            meta: BTreeMap::new(),
        }
    }

    pub fn with_meta(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.meta.insert(key.into(), value.into());
        self
    }
}

/// Callable surface for a tool. Takes the JSON-encoded arguments dict the
/// LLM produced; returns either a successful [`ToolResult`] or a
/// [`HermesError`]. Errors are wrapped into a non-`ok` ToolResult by the
/// registry so the LLM gets a structured response either way.
pub type ToolHandler =
    Arc<dyn Fn(&serde_json::Value) -> crate::error::Result<ToolResult> + Send + Sync>;

/// Optional gate evaluated before invocation. Returns `Ok(())` to allow,
/// `Err(reason)` to reject. Ports `check_fn` on Hermes' `ToolEntry`.
pub type ToolCheck = Arc<dyn Fn() -> std::result::Result<(), String> + Send + Sync>;

#[derive(Clone)]
pub struct ToolEntry {
    pub name: String,
    pub toolset: String,
    pub description: String,
    pub schema: serde_json::Value,
    pub handler: ToolHandler,
    pub check_fn: Option<ToolCheck>,
    pub requires_env: Vec<String>,
    pub is_async: bool,
    pub max_result_size_chars: Option<usize>,
}

impl std::fmt::Debug for ToolEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolEntry")
            .field("name", &self.name)
            .field("toolset", &self.toolset)
            .field("description", &self.description)
            .field("requires_env", &self.requires_env)
            .field("is_async", &self.is_async)
            .field("max_result_size_chars", &self.max_result_size_chars)
            .finish_non_exhaustive()
    }
}

impl ToolEntry {
    pub fn new(
        name: impl Into<String>,
        toolset: impl Into<String>,
        description: impl Into<String>,
        schema: serde_json::Value,
        handler: ToolHandler,
    ) -> Self {
        Self {
            name: name.into(),
            toolset: toolset.into(),
            description: description.into(),
            schema,
            handler,
            check_fn: None,
            requires_env: Vec::new(),
            is_async: false,
            max_result_size_chars: None,
        }
    }

    pub fn with_check(mut self, check: ToolCheck) -> Self {
        self.check_fn = Some(check);
        self
    }

    pub fn requires_env<I, S>(mut self, vars: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.requires_env = vars.into_iter().map(Into::into).collect();
        self
    }

    pub fn with_max_result_size(mut self, n: usize) -> Self {
        self.max_result_size_chars = Some(n);
        self
    }
}

/// Self-registering tool registry. Stable iteration order is guaranteed
/// (BTreeMap), which lets tests assert against the rendered JSON schema
/// catalogue without flakiness.
#[derive(Default)]
pub struct ToolRegistry {
    inner: BTreeMap<String, ToolEntry>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert (or replace) a tool. Returns the previous entry, if any —
    /// mirrors upstream's "last writer wins" semantics for re-registration
    /// (Hermes' `tools.registry.register` overwrites silently).
    pub fn register(&mut self, entry: ToolEntry) -> Option<ToolEntry> {
        self.inner.insert(entry.name.clone(), entry)
    }

    pub fn get(&self, name: &str) -> Option<&ToolEntry> {
        self.inner.get(name)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &ToolEntry> {
        self.inner.values()
    }

    /// Names sorted alphabetically.
    pub fn names(&self) -> Vec<String> {
        self.inner.keys().cloned().collect()
    }

    /// Invoke a tool by name. Resolves the entry, runs `check_fn` (if any)
    /// and env-var gates, then dispatches the handler. Always returns a
    /// `ToolResult` so the LLM gets a structured response; internal errors
    /// get folded into `ok=false`.
    pub fn invoke(&self, name: &str, args: &serde_json::Value) -> crate::error::Result<ToolResult> {
        let entry = self.get(name).ok_or_else(|| HermesError::ToolNotFound {
            name: name.to_string(),
        })?;
        if let Some(check) = &entry.check_fn
            && let Err(reason) = (check)()
        {
            return Err(HermesError::ToolFailed {
                name: entry.name.clone(),
                reason,
            });
        }
        for var in &entry.requires_env {
            if std::env::var(var).is_err() {
                return Err(HermesError::ToolFailed {
                    name: entry.name.clone(),
                    reason: format!("missing env var: {var}"),
                });
            }
        }
        let mut out = (entry.handler)(args)?;
        if let Some(cap) = entry.max_result_size_chars
            && out.output.len() > cap
        {
            out.output.truncate(cap);
            out = out.with_meta("truncated", "true");
        }
        Ok(out)
    }

    /// Render the schema catalogue the LLM sees on the wire. Hermes
    /// emits a JSON array of `{name, description, parameters}` objects;
    /// we follow the same shape so it is drop-in for OpenAI / Anthropic
    /// function-calling adapters.
    pub fn schema_catalogue(&self) -> serde_json::Value {
        let mut arr = Vec::with_capacity(self.inner.len());
        for e in self.inner.values() {
            arr.push(serde_json::json!({
                "name": e.name,
                "description": e.description,
                "parameters": e.schema,
                "toolset": e.toolset,
            }));
        }
        serde_json::Value::Array(arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn echo_entry() -> ToolEntry {
        ToolEntry::new(
            "echo",
            "core",
            "echo the input back",
            serde_json::json!({"type": "object"}),
            Arc::new(|args: &serde_json::Value| Ok(ToolResult::ok(args.to_string()))),
        )
    }

    #[test]
    fn register_and_invoke_roundtrip() {
        let mut r = ToolRegistry::new();
        r.register(echo_entry());
        let out = r.invoke("echo", &serde_json::json!({"x": 1})).unwrap();
        assert!(out.ok);
        assert!(out.output.contains("\"x\":1"));
    }

    #[test]
    fn invoke_missing_tool_returns_not_found() {
        let r = ToolRegistry::new();
        let err = r.invoke("nope", &serde_json::Value::Null).unwrap_err();
        assert!(matches!(err, HermesError::ToolNotFound { .. }));
    }

    #[test]
    fn check_fn_rejection_surfaces() {
        let mut r = ToolRegistry::new();
        r.register(echo_entry().with_check(Arc::new(|| Err("not allowed".into()))));
        let err = r.invoke("echo", &serde_json::Value::Null).unwrap_err();
        match err {
            HermesError::ToolFailed { reason, .. } => assert_eq!(reason, "not allowed"),
            e => panic!("expected ToolFailed, got {e}"),
        }
    }

    #[test]
    fn requires_env_blocks_when_var_absent() {
        let mut r = ToolRegistry::new();
        // Use a sentinel name that nobody else will set.
        r.register(echo_entry().requires_env(["CAVE_HERMES_TEST_NONEXISTENT_XYZ_42"]));
        let err = r.invoke("echo", &serde_json::Value::Null).unwrap_err();
        match err {
            HermesError::ToolFailed { reason, .. } => {
                assert!(reason.contains("missing env var"));
            }
            e => panic!("expected ToolFailed, got {e}"),
        }
    }

    #[test]
    fn max_result_size_truncates_and_marks() {
        let mut r = ToolRegistry::new();
        r.register(
            ToolEntry::new(
                "big",
                "core",
                "emits big payload",
                serde_json::json!({}),
                Arc::new(|_| Ok(ToolResult::ok("x".repeat(1000)))),
            )
            .with_max_result_size(16),
        );
        let out = r.invoke("big", &serde_json::Value::Null).unwrap();
        assert_eq!(out.output.len(), 16);
        assert_eq!(out.meta.get("truncated").map(String::as_str), Some("true"));
    }

    #[test]
    fn re_register_replaces_silently() {
        let mut r = ToolRegistry::new();
        r.register(echo_entry());
        let v2 = ToolEntry::new(
            "echo",
            "core",
            "v2",
            serde_json::json!({}),
            Arc::new(|_| Ok(ToolResult::ok("v2"))),
        );
        let prev = r.register(v2);
        assert!(prev.is_some());
        assert_eq!(
            r.invoke("echo", &serde_json::Value::Null).unwrap().output,
            "v2"
        );
    }

    #[test]
    fn schema_catalogue_lists_all_tools() {
        let mut r = ToolRegistry::new();
        r.register(echo_entry());
        let catalogue = r.schema_catalogue();
        assert!(catalogue.is_array());
        assert_eq!(catalogue.as_array().unwrap().len(), 1);
        let first = &catalogue[0];
        assert_eq!(first["name"], "echo");
        assert_eq!(first["toolset"], "core");
    }

    #[test]
    fn names_are_sorted_alphabetically() {
        let mut r = ToolRegistry::new();
        r.register(ToolEntry::new(
            "zeta",
            "core",
            "",
            serde_json::json!({}),
            Arc::new(|_| Ok(ToolResult::ok("z"))),
        ));
        r.register(ToolEntry::new(
            "alpha",
            "core",
            "",
            serde_json::json!({}),
            Arc::new(|_| Ok(ToolResult::ok("a"))),
        ));
        assert_eq!(r.names(), vec!["alpha".to_string(), "zeta".to_string()]);
    }
}
