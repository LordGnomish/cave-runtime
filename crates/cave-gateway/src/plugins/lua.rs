//! lua plugin — config layer for Kong-compatible Lua scripts.
//!
//! Real Lua execution lands in a separate runtime (mlua / piccolo) — this
//! module ships the *scaffolding*: script source storage, pre/post-hook phase
//! attachment, syntactic sanity checks, the small DSL the script can use to
//! drive header / status / body changes, and the apply step that interprets
//! those DSL records against a request/response context.
//!
//! Today's gateway operators write tiny scripts; we model that by accepting
//! either:
//! - a free-form Lua source body (kept as a string for the embedder), or
//! - an inline `LuaAction` sequence — `set_header`, `remove_header`,
//!   `set_status`, `replace_body`, `log` — that the gateway executes
//!   directly without a Lua VM.
//!
//! The DSL is the parity surface; future work plugs in mlua to evaluate the
//! script source and emit `LuaAction`s.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LuaPhase {
    /// Before authorization / routing.
    Access,
    /// Modify request headers en route to upstream.
    RewriteRequest,
    /// Modify response headers / status.
    HeaderFilter,
    /// Modify response body.
    BodyFilter,
    /// After response, fire-and-forget (logging).
    Log,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LuaAction {
    SetRequestHeader { name: String, value: String, append: bool },
    RemoveRequestHeader { name: String },
    SetResponseHeader { name: String, value: String, append: bool },
    RemoveResponseHeader { name: String },
    SetStatus { status: u16 },
    ReplaceBody { body: String },
    Log { level: LogLevel, message: String },
    SetVariable { name: String, value: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LuaConfig {
    pub phase: LuaPhase,
    /// Inline DSL (preferred when mlua isn't linked).
    #[serde(default)]
    pub actions: Vec<LuaAction>,
    /// Free-form Lua source (kept for the embedder).
    #[serde(default)]
    pub source: Option<String>,
    /// If true, errors in the script abort the request.
    #[serde(default)]
    pub fail_closed: bool,
    /// Limit for the inline action list (defense against runaway DSL).
    #[serde(default = "default_max_actions")]
    pub max_actions: u32,
}

fn default_max_actions() -> u32 {
    256
}

impl Default for LuaConfig {
    fn default() -> Self {
        Self {
            phase: LuaPhase::Access,
            actions: vec![],
            source: None,
            fail_closed: false,
            max_actions: 256,
        }
    }
}

impl LuaConfig {
    pub fn validate(&self) -> Result<(), String> {
        if (self.actions.len() as u32) > self.max_actions {
            return Err(format!(
                "actions ({}) exceed max_actions ({})",
                self.actions.len(),
                self.max_actions
            ));
        }
        if let Some(src) = &self.source {
            sanity_check_source(src)?;
        }
        for a in &self.actions {
            validate_action(a)?;
        }
        // Cross-phase rule: response actions only meaningful in
        // header_filter / body_filter / log.
        let touches_response = self.actions.iter().any(|a| match a {
            LuaAction::SetResponseHeader { .. }
            | LuaAction::RemoveResponseHeader { .. }
            | LuaAction::SetStatus { .. }
            | LuaAction::ReplaceBody { .. } => true,
            _ => false,
        });
        if touches_response
            && !matches!(self.phase, LuaPhase::HeaderFilter | LuaPhase::BodyFilter | LuaPhase::Log)
        {
            return Err(
                "response-side actions only allowed in header_filter / body_filter / log phases".into(),
            );
        }
        Ok(())
    }
}

fn validate_action(a: &LuaAction) -> Result<(), String> {
    match a {
        LuaAction::SetRequestHeader { name, .. }
        | LuaAction::RemoveRequestHeader { name }
        | LuaAction::SetResponseHeader { name, .. }
        | LuaAction::RemoveResponseHeader { name } => {
            if name.is_empty() {
                return Err("header name is empty".into());
            }
            if name.contains(' ') || name.contains(':') {
                return Err(format!("invalid header name '{}'", name));
            }
        }
        LuaAction::SetStatus { status } => {
            if !(100..=599).contains(status) {
                return Err(format!("invalid status {}", status));
            }
        }
        LuaAction::ReplaceBody { body } => {
            if body.len() > 16 * 1024 * 1024 {
                return Err("replacement body exceeds 16MB".into());
            }
        }
        LuaAction::Log { message, .. } => {
            if message.is_empty() {
                return Err("log message is empty".into());
            }
        }
        LuaAction::SetVariable { name, .. } => {
            if name.is_empty() {
                return Err("variable name is empty".into());
            }
        }
    }
    Ok(())
}

/// Cheap sanity check on the Lua source — block obvious sandbox escapes
/// before we even hand the script to a real interpreter.
fn sanity_check_source(src: &str) -> Result<(), String> {
    const FORBIDDEN: &[&str] = &[
        "io.open",
        "io.popen",
        "os.execute",
        "os.exit",
        "os.remove",
        "os.rename",
        "loadfile",
        "dofile",
        "package.loadlib",
        "debug.sethook",
        "debug.setupvalue",
        "require(\"ffi\")",
        "ffi.load",
    ];
    for f in FORBIDDEN {
        if src.contains(f) {
            return Err(format!("forbidden Lua call: {}", f));
        }
    }
    if src.len() > 256 * 1024 {
        return Err("Lua source > 256KB".into());
    }
    Ok(())
}

// ── Mutable request/response context the DSL actions operate on ──────────────

#[derive(Debug, Default)]
pub struct ScriptContext {
    pub request_headers: HashMap<String, String>,
    pub response_headers: HashMap<String, String>,
    pub response_status: Option<u16>,
    pub response_body: Option<Vec<u8>>,
    pub variables: HashMap<String, String>,
    pub logs: Vec<(LogLevel, String)>,
}

impl ScriptContext {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LuaOutcome {
    Continue,
    Halt,
}

/// Apply a list of [`LuaAction`]s against the script context.
///
/// Returns `Halt` when an action sets a 4xx/5xx status — the caller should
/// short-circuit and emit the response immediately.
pub fn apply(
    cfg: &LuaConfig,
    ctx: &mut ScriptContext,
    actions: &[LuaAction],
) -> Result<LuaOutcome, String> {
    let mut outcome = LuaOutcome::Continue;
    for action in actions {
        match action {
            LuaAction::SetRequestHeader { name, value, append } => {
                let key = name.to_ascii_lowercase();
                if *append {
                    if let Some(existing) = ctx.request_headers.get_mut(&key) {
                        existing.push(',');
                        existing.push_str(value);
                    } else {
                        ctx.request_headers.insert(key, value.clone());
                    }
                } else {
                    ctx.request_headers.insert(key, value.clone());
                }
            }
            LuaAction::RemoveRequestHeader { name } => {
                ctx.request_headers.remove(&name.to_ascii_lowercase());
            }
            LuaAction::SetResponseHeader { name, value, append } => {
                let key = name.to_ascii_lowercase();
                if *append {
                    if let Some(existing) = ctx.response_headers.get_mut(&key) {
                        existing.push(',');
                        existing.push_str(value);
                    } else {
                        ctx.response_headers.insert(key, value.clone());
                    }
                } else {
                    ctx.response_headers.insert(key, value.clone());
                }
            }
            LuaAction::RemoveResponseHeader { name } => {
                ctx.response_headers.remove(&name.to_ascii_lowercase());
            }
            LuaAction::SetStatus { status } => {
                ctx.response_status = Some(*status);
                if (400..=599).contains(status) {
                    outcome = LuaOutcome::Halt;
                }
            }
            LuaAction::ReplaceBody { body } => {
                ctx.response_body = Some(body.as_bytes().to_vec());
            }
            LuaAction::Log { level, message } => {
                ctx.logs.push((*level, message.clone()));
            }
            LuaAction::SetVariable { name, value } => {
                ctx.variables.insert(name.clone(), value.clone());
            }
        }
    }
    if cfg.fail_closed && !ctx.logs.is_empty() {
        // fail_closed semantics: any log.error during apply forces deny. We
        // surface that here so the caller can act.
        if ctx.logs.iter().any(|(l, _)| matches!(l, LogLevel::Error)) {
            return Err("fail_closed: error logged during script run".into());
        }
    }
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(phase: LuaPhase, actions: Vec<LuaAction>) -> LuaConfig {
        LuaConfig {
            phase,
            actions,
            source: None,
            fail_closed: false,
            max_actions: 256,
        }
    }

    // ── Source sanity ──

    #[test]
    fn source_blocks_io_open() {
        assert!(sanity_check_source("local f = io.open('/etc/passwd')").is_err());
    }

    #[test]
    fn source_blocks_os_execute() {
        assert!(sanity_check_source("os.execute('rm -rf /')").is_err());
    }

    #[test]
    fn source_blocks_loadfile() {
        assert!(sanity_check_source("loadfile('/tmp/x')").is_err());
    }

    #[test]
    fn source_blocks_ffi_load() {
        assert!(sanity_check_source("ffi.load('/tmp/x.so')").is_err());
    }

    #[test]
    fn source_allows_simple_script() {
        sanity_check_source("ngx.req.set_header('X-User', 'alice')").unwrap();
    }

    #[test]
    fn source_rejects_too_large() {
        let big = "x".repeat(300 * 1024);
        assert!(sanity_check_source(&big).is_err());
    }

    // ── Action validation ──

    #[test]
    fn action_set_header_rejects_empty_name() {
        assert!(validate_action(&LuaAction::SetRequestHeader {
            name: "".into(),
            value: "v".into(),
            append: false,
        })
        .is_err());
    }

    #[test]
    fn action_set_header_rejects_invalid_chars() {
        assert!(validate_action(&LuaAction::SetRequestHeader {
            name: "X: Bad".into(),
            value: "v".into(),
            append: false,
        })
        .is_err());
    }

    #[test]
    fn action_set_status_out_of_range_rejects() {
        assert!(validate_action(&LuaAction::SetStatus { status: 99 }).is_err());
        assert!(validate_action(&LuaAction::SetStatus { status: 600 }).is_err());
    }

    #[test]
    fn action_replace_body_too_large_rejects() {
        let body = "x".repeat(20 * 1024 * 1024);
        assert!(validate_action(&LuaAction::ReplaceBody { body }).is_err());
    }

    #[test]
    fn action_log_empty_rejects() {
        assert!(validate_action(&LuaAction::Log {
            level: LogLevel::Info,
            message: "".into()
        })
        .is_err());
    }

    #[test]
    fn action_set_variable_empty_name_rejects() {
        assert!(validate_action(&LuaAction::SetVariable {
            name: "".into(),
            value: "v".into()
        })
        .is_err());
    }

    // ── Config validation ──

    #[test]
    fn config_default_validates() {
        LuaConfig::default().validate().unwrap();
    }

    #[test]
    fn config_too_many_actions_rejects() {
        let mut c = LuaConfig::default();
        c.max_actions = 2;
        c.actions = vec![
            LuaAction::Log { level: LogLevel::Info, message: "a".into() },
            LuaAction::Log { level: LogLevel::Info, message: "b".into() },
            LuaAction::Log { level: LogLevel::Info, message: "c".into() },
        ];
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_response_action_in_access_phase_rejects() {
        let c = cfg(
            LuaPhase::Access,
            vec![LuaAction::SetStatus { status: 200 }],
        );
        assert!(c.validate().is_err());
    }

    #[test]
    fn config_response_action_in_header_filter_phase_passes() {
        let c = cfg(
            LuaPhase::HeaderFilter,
            vec![LuaAction::SetStatus { status: 200 }],
        );
        c.validate().unwrap();
    }

    #[test]
    fn config_request_action_in_access_phase_passes() {
        let c = cfg(
            LuaPhase::Access,
            vec![LuaAction::SetRequestHeader {
                name: "X-User".into(),
                value: "alice".into(),
                append: false,
            }],
        );
        c.validate().unwrap();
    }

    // ── apply ──

    #[test]
    fn apply_set_request_header_overwrites() {
        let mut ctx = ScriptContext::new();
        ctx.request_headers.insert("x-user".into(), "old".into());
        let actions = vec![LuaAction::SetRequestHeader {
            name: "X-User".into(),
            value: "new".into(),
            append: false,
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(ctx.request_headers.get("x-user"), Some(&"new".to_string()));
    }

    #[test]
    fn apply_set_request_header_appends() {
        let mut ctx = ScriptContext::new();
        ctx.request_headers.insert("x-trace".into(), "first".into());
        let actions = vec![LuaAction::SetRequestHeader {
            name: "X-Trace".into(),
            value: "second".into(),
            append: true,
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(
            ctx.request_headers.get("x-trace"),
            Some(&"first,second".to_string())
        );
    }

    #[test]
    fn apply_remove_request_header() {
        let mut ctx = ScriptContext::new();
        ctx.request_headers.insert("x-internal".into(), "secret".into());
        let actions = vec![LuaAction::RemoveRequestHeader {
            name: "X-Internal".into(),
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert!(ctx.request_headers.is_empty());
    }

    #[test]
    fn apply_set_response_header() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::SetResponseHeader {
            name: "Cache-Control".into(),
            value: "no-store".into(),
            append: false,
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(
            ctx.response_headers.get("cache-control"),
            Some(&"no-store".to_string())
        );
    }

    #[test]
    fn apply_set_status_4xx_halts() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::SetStatus { status: 401 }];
        let outcome = apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(outcome, LuaOutcome::Halt);
        assert_eq!(ctx.response_status, Some(401));
    }

    #[test]
    fn apply_set_status_2xx_continues() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::SetStatus { status: 200 }];
        let outcome = apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(outcome, LuaOutcome::Continue);
    }

    #[test]
    fn apply_replace_body() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::ReplaceBody {
            body: "hello".into(),
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(ctx.response_body.as_deref(), Some(b"hello".as_ref()));
    }

    #[test]
    fn apply_log_collects() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::Log {
            level: LogLevel::Info,
            message: "hello".into(),
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(ctx.logs.len(), 1);
    }

    #[test]
    fn apply_set_variable_persists() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::SetVariable {
            name: "tenant".into(),
            value: "acme".into(),
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(ctx.variables.get("tenant"), Some(&"acme".to_string()));
    }

    #[test]
    fn apply_fail_closed_propagates_error_log() {
        let mut ctx = ScriptContext::new();
        let mut c = LuaConfig::default();
        c.fail_closed = true;
        let actions = vec![LuaAction::Log {
            level: LogLevel::Error,
            message: "kaboom".into(),
        }];
        assert!(apply(&c, &mut ctx, &actions).is_err());
    }

    #[test]
    fn apply_fail_open_swallows_error_log() {
        let mut ctx = ScriptContext::new();
        let actions = vec![LuaAction::Log {
            level: LogLevel::Error,
            message: "kaboom".into(),
        }];
        apply(&LuaConfig::default(), &mut ctx, &actions).unwrap();
        assert_eq!(ctx.logs.len(), 1);
    }

    #[test]
    fn lua_config_json_roundtrip() {
        let c = cfg(
            LuaPhase::HeaderFilter,
            vec![
                LuaAction::SetResponseHeader {
                    name: "X-Cave".into(),
                    value: "1".into(),
                    append: false,
                },
                LuaAction::Log {
                    level: LogLevel::Info,
                    message: "ok".into(),
                },
            ],
        );
        let s = serde_json::to_string(&c).unwrap();
        let c2: LuaConfig = serde_json::from_str(&s).unwrap();
        assert_eq!(c, c2);
    }
}
