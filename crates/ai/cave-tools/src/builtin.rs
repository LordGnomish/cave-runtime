// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Sandboxed built-in tools.
//!
//! Each tool is constructed from a *capability handle* the host owns and
//! passes in, so the sandbox boundary is explicit and testable:
//!
//! * file ops are jailed to a [`FileSandbox`] root (path traversal and
//!   out-of-root absolute paths are rejected);
//! * web search delegates to an injected [`WebSearchProvider`] — the
//!   framework never opens a socket itself;
//! * code execution is a pure arithmetic evaluator ([`eval_arithmetic`]),
//!   with no system access (general subprocess/WASM execution is a
//!   documented scope-cut);
//! * calendar and email write to in-memory stores ([`Calendar`],
//!   [`Mailbox`]) that never touch the network — the outbox is inspectable.

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::error::ToolError;
use crate::tool::{FnTool, ToolRegistry, ToolResult};

// ════════════════════════════════════════════════════════════════════════
// File sandbox
// ════════════════════════════════════════════════════════════════════════

/// A directory jail. Every resolved path must stay within `root`.
pub struct FileSandbox {
    root: PathBuf,
}

impl FileSandbox {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Resolve a caller-supplied relative path against the jail root,
    /// rejecting absolute paths and any `..` that would escape the root.
    fn resolve(&self, requested: &str) -> std::result::Result<PathBuf, String> {
        let p = Path::new(requested);
        if p.is_absolute() {
            return Err(format!("absolute paths are not allowed: {requested}"));
        }
        let mut out = self.root.clone();
        for comp in p.components() {
            match comp {
                Component::Normal(seg) => out.push(seg),
                Component::CurDir => {}
                Component::ParentDir => {
                    // Pop, but never above the root.
                    if !out.pop() || !out.starts_with(&self.root) {
                        return Err(format!("path escapes sandbox root: {requested}"));
                    }
                }
                Component::RootDir | Component::Prefix(_) => {
                    return Err(format!("invalid path component in: {requested}"));
                }
            }
        }
        if !out.starts_with(&self.root) {
            return Err(format!("path escapes sandbox root: {requested}"));
        }
        Ok(out)
    }
}

fn sandbox_err(tool: &str, reason: String) -> ToolError {
    ToolError::Sandbox {
        tool: tool.to_string(),
        reason,
    }
}

fn exec_err(tool: &str, reason: impl Into<String>) -> ToolError {
    ToolError::Execution {
        tool: tool.to_string(),
        reason: reason.into(),
    }
}

/// `file_read` — read a UTF-8 file from within the sandbox.
pub fn file_read_tool(sb: Arc<FileSandbox>) -> FnTool {
    FnTool::new(
        "file_read",
        "Read a UTF-8 text file relative to the sandbox root",
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": ["path"]
        }),
        move |args| {
            let path = args["path"].as_str().unwrap_or("");
            let resolved = sb.resolve(path).map_err(|e| sandbox_err("file_read", e))?;
            let body = std::fs::read_to_string(&resolved)
                .map_err(|e| exec_err("file_read", format!("read {path}: {e}")))?;
            Ok(ToolResult::text(body))
        },
    )
    .with_toolset("fs")
}

/// `file_write` — write a UTF-8 file within the sandbox (creating parents).
pub fn file_write_tool(sb: Arc<FileSandbox>) -> FnTool {
    FnTool::new(
        "file_write",
        "Write a UTF-8 text file relative to the sandbox root (creates parent dirs)",
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}, "content": {"type": "string"}},
            "required": ["path", "content"]
        }),
        move |args| {
            let path = args["path"].as_str().unwrap_or("");
            let content = args["content"].as_str().unwrap_or("");
            let resolved = sb.resolve(path).map_err(|e| sandbox_err("file_write", e))?;
            if let Some(parent) = resolved.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| exec_err("file_write", format!("mkdir: {e}")))?;
            }
            std::fs::write(&resolved, content)
                .map_err(|e| exec_err("file_write", format!("write {path}: {e}")))?;
            Ok(ToolResult::text(format!("wrote {} bytes", content.len()))
                .with_structured(json!({"path": path, "bytes": content.len()})))
        },
    )
    .with_toolset("fs")
}

/// `file_list` — list immediate entries of a directory within the sandbox.
pub fn file_list_tool(sb: Arc<FileSandbox>) -> FnTool {
    FnTool::new(
        "file_list",
        "List entries of a directory relative to the sandbox root",
        json!({
            "type": "object",
            "properties": {"path": {"type": "string"}},
            "required": []
        }),
        move |args| {
            let path = args.get("path").and_then(Value::as_str).unwrap_or(".");
            let resolved = sb.resolve(path).map_err(|e| sandbox_err("file_list", e))?;
            let mut names = Vec::new();
            let rd = std::fs::read_dir(&resolved)
                .map_err(|e| exec_err("file_list", format!("readdir {path}: {e}")))?;
            for entry in rd.flatten() {
                names.push(entry.file_name().to_string_lossy().to_string());
            }
            names.sort();
            Ok(ToolResult::text(names.join("\n")).with_structured(json!({"entries": names})))
        },
    )
    .with_toolset("fs")
}

// ════════════════════════════════════════════════════════════════════════
// Web search (injected provider)
// ════════════════════════════════════════════════════════════════════════

/// A single web-search hit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Host-supplied search backend. The framework opens no sockets itself; the
/// host injects a provider (a real HTTP client, a cached index, a mock, …).
pub trait WebSearchProvider: Send + Sync {
    fn search(&self, query: &str, limit: usize) -> Vec<WebResult>;
}

/// `web_search` — query the injected provider.
pub fn web_search_tool(provider: Arc<dyn WebSearchProvider>) -> FnTool {
    FnTool::new(
        "web_search",
        "Search the web via the host-configured search provider",
        json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "limit": {"type": "integer", "minimum": 1, "maximum": 50}
            },
            "required": ["query"]
        }),
        move |args| {
            let query = args["query"].as_str().unwrap_or("");
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(5) as usize;
            let results = provider.search(query, limit);
            let summary = results
                .iter()
                .map(|r| format!("{} — {}", r.title, r.url))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ToolResult::text(summary).with_structured(json!({"results": results})))
        },
    )
    .with_toolset("net")
}

// ════════════════════════════════════════════════════════════════════════
// Sandboxed code execution — arithmetic evaluator
// ════════════════════════════════════════════════════════════════════════

/// `code_exec` — evaluate an arithmetic expression in a no-IO sandbox.
pub fn code_exec_tool() -> FnTool {
    FnTool::new(
        "code_exec",
        "Evaluate a sandboxed arithmetic expression (+ - * / %, parentheses, unary minus). No system access.",
        json!({
            "type": "object",
            "properties": {"expression": {"type": "string"}},
            "required": ["expression"]
        }),
        move |args| {
            let expr = args["expression"].as_str().unwrap_or("");
            let n = eval_arithmetic(expr).map_err(|e| exec_err("code_exec", e))?;
            // Render integers without a trailing `.0`.
            let rendered = if n.fract() == 0.0 && n.abs() < 1e15 {
                (n as i64).to_string()
            } else {
                n.to_string()
            };
            Ok(ToolResult::text(rendered).with_structured(json!({"value": n})))
        },
    )
    .with_toolset("code")
}

/// Evaluate a pure arithmetic expression. Supports `+ - * / %`, parentheses,
/// unary minus, and decimal literals. Any other token is rejected — this is
/// the sandbox boundary for the `code_exec` tool.
pub fn eval_arithmetic(input: &str) -> std::result::Result<f64, String> {
    let tokens = lex(input)?;
    let mut p = Parser { tokens, pos: 0 };
    let v = p.expr()?;
    if p.pos != p.tokens.len() {
        return Err(format!("unexpected trailing token at {}", p.pos));
    }
    Ok(v)
}

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Num(f64),
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    LParen,
    RParen,
}

fn lex(s: &str) -> std::result::Result<Vec<Tok>, String> {
    let mut toks = Vec::new();
    let chars: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\n' | '\r' => i += 1,
            '+' => {
                toks.push(Tok::Plus);
                i += 1;
            }
            '-' => {
                toks.push(Tok::Minus);
                i += 1;
            }
            '*' => {
                toks.push(Tok::Star);
                i += 1;
            }
            '/' => {
                toks.push(Tok::Slash);
                i += 1;
            }
            '%' => {
                toks.push(Tok::Percent);
                i += 1;
            }
            '(' => {
                toks.push(Tok::LParen);
                i += 1;
            }
            ')' => {
                toks.push(Tok::RParen);
                i += 1;
            }
            '0'..='9' | '.' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                let lit: String = chars[start..i].iter().collect();
                let n: f64 = lit
                    .parse()
                    .map_err(|_| format!("invalid number literal `{lit}`"))?;
                toks.push(Tok::Num(n));
            }
            other => {
                return Err(format!(
                    "illegal character `{other}` — only arithmetic is allowed in the sandbox"
                ));
            }
        }
    }
    Ok(toks)
}

struct Parser {
    tokens: Vec<Tok>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.tokens.get(self.pos)
    }

    // expr := term (('+' | '-') term)*
    fn expr(&mut self) -> std::result::Result<f64, String> {
        let mut acc = self.term()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Tok::Plus => {
                    self.pos += 1;
                    acc += self.term()?;
                }
                Tok::Minus => {
                    self.pos += 1;
                    acc -= self.term()?;
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    // term := factor (('*' | '/' | '%') factor)*
    fn term(&mut self) -> std::result::Result<f64, String> {
        let mut acc = self.factor()?;
        while let Some(op) = self.peek().cloned() {
            match op {
                Tok::Star => {
                    self.pos += 1;
                    acc *= self.factor()?;
                }
                Tok::Slash => {
                    self.pos += 1;
                    let d = self.factor()?;
                    if d == 0.0 {
                        return Err("division by zero".into());
                    }
                    acc /= d;
                }
                Tok::Percent => {
                    self.pos += 1;
                    let d = self.factor()?;
                    if d == 0.0 {
                        return Err("modulo by zero".into());
                    }
                    acc %= d;
                }
                _ => break,
            }
        }
        Ok(acc)
    }

    // factor := '-' factor | '(' expr ')' | NUM
    fn factor(&mut self) -> std::result::Result<f64, String> {
        match self.peek().cloned() {
            Some(Tok::Minus) => {
                self.pos += 1;
                Ok(-self.factor()?)
            }
            Some(Tok::LParen) => {
                self.pos += 1;
                let v = self.expr()?;
                match self.peek() {
                    Some(Tok::RParen) => {
                        self.pos += 1;
                        Ok(v)
                    }
                    _ => Err("expected `)`".into()),
                }
            }
            Some(Tok::Num(n)) => {
                self.pos += 1;
                Ok(n)
            }
            other => Err(format!("expected a number or `(`, found {other:?}")),
        }
    }
}

// ════════════════════════════════════════════════════════════════════════
// Calendar (in-memory store)
// ════════════════════════════════════════════════════════════════════════

/// One calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub title: String,
    pub start: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<String>,
}

/// An in-memory calendar. Never touches a real calendar service.
#[derive(Default)]
pub struct Calendar {
    events: Mutex<Vec<CalendarEvent>>,
}

impl Calendar {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn events(&self) -> Vec<CalendarEvent> {
        self.events.lock().unwrap().clone()
    }
}

/// `calendar_add` — add an event to the in-memory calendar.
pub fn calendar_add_tool(cal: Arc<Calendar>) -> FnTool {
    FnTool::new(
        "calendar_add",
        "Add an event to the sandboxed calendar",
        json!({
            "type": "object",
            "properties": {
                "title": {"type": "string"},
                "start": {"type": "string"},
                "end": {"type": "string"}
            },
            "required": ["title", "start"]
        }),
        move |args| {
            let ev = CalendarEvent {
                title: args["title"].as_str().unwrap_or("").to_string(),
                start: args["start"].as_str().unwrap_or("").to_string(),
                end: args.get("end").and_then(Value::as_str).map(String::from),
            };
            cal.events.lock().unwrap().push(ev);
            Ok(ToolResult::text("event added"))
        },
    )
    .with_toolset("calendar")
}

/// `calendar_list` — list events from the in-memory calendar.
pub fn calendar_list_tool(cal: Arc<Calendar>) -> FnTool {
    FnTool::new(
        "calendar_list",
        "List events in the sandboxed calendar",
        json!({"type": "object", "properties": {}, "required": []}),
        move |_args| {
            let events = cal.events();
            let summary = events
                .iter()
                .map(|e| format!("{} @ {}", e.title, e.start))
                .collect::<Vec<_>>()
                .join("\n");
            Ok(ToolResult::text(summary).with_structured(json!({"events": events})))
        },
    )
    .with_toolset("calendar")
}

// ════════════════════════════════════════════════════════════════════════
// Email (in-memory outbox)
// ════════════════════════════════════════════════════════════════════════

/// A queued email.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

/// An in-memory outbox. `email_send` queues here; nothing leaves the process.
#[derive(Default)]
pub struct Mailbox {
    outbox: Mutex<Vec<Email>>,
}

impl Mailbox {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn outbox(&self) -> Vec<Email> {
        self.outbox.lock().unwrap().clone()
    }
}

/// `email_send` — queue an email to the sandboxed outbox.
pub fn email_send_tool(mb: Arc<Mailbox>) -> FnTool {
    FnTool::new(
        "email_send",
        "Queue an email to the sandboxed outbox (no network send)",
        json!({
            "type": "object",
            "properties": {
                "to": {"type": "string"},
                "subject": {"type": "string"},
                "body": {"type": "string"}
            },
            "required": ["to", "subject", "body"]
        }),
        move |args| {
            let email = Email {
                to: args["to"].as_str().unwrap_or("").to_string(),
                subject: args["subject"].as_str().unwrap_or("").to_string(),
                body: args["body"].as_str().unwrap_or("").to_string(),
            };
            mb.outbox.lock().unwrap().push(email);
            Ok(ToolResult::text("queued to outbox"))
        },
    )
    .with_toolset("email")
}

// ════════════════════════════════════════════════════════════════════════
// Default registration
// ════════════════════════════════════════════════════════════════════════

/// Capability handles for the full built-in set.
pub struct BuiltinConfig {
    pub file_sandbox: Arc<FileSandbox>,
    pub web: Arc<dyn WebSearchProvider>,
    pub calendar: Arc<Calendar>,
    pub mailbox: Arc<Mailbox>,
}

/// Register every built-in tool into `reg`.
pub fn register_builtins(reg: &mut ToolRegistry, cfg: &BuiltinConfig) {
    reg.register(file_read_tool(cfg.file_sandbox.clone()));
    reg.register(file_write_tool(cfg.file_sandbox.clone()));
    reg.register(file_list_tool(cfg.file_sandbox.clone()));
    reg.register(web_search_tool(cfg.web.clone()));
    reg.register(code_exec_tool());
    reg.register(calendar_add_tool(cfg.calendar.clone()));
    reg.register(calendar_list_tool(cfg.calendar.clone()));
    reg.register(email_send_tool(cfg.mailbox.clone()));
}
