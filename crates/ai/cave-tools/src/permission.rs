// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Per-tool and per-user authorization.
//!
//! MCP delegates tool authorization to the host ("Hosts must obtain explicit
//! user consent before invoking any tool"); this module is that host-side
//! policy engine. A [`PermissionPolicy`] is an ordered list of allow/deny
//! [`Rule`]s evaluated with **deny-overrides** semantics on top of a
//! configurable default:
//!
//! 1. if any matching `Deny` rule exists → denied;
//! 2. else if any matching `Allow` rule exists → allowed;
//! 3. else → the policy default.
//!
//! Targets are matched per-tool (`file_read`), per-toolset (`fs/*`), or
//! globally (`*`); principals are matched per-user or as "any user".

use crate::error::{Result, ToolError};
use crate::tool::ToolSpec;

/// What a rule applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A specific tool by name.
    Tool(String),
    /// Every tool in a toolset.
    Toolset(String),
    /// Every tool.
    Any,
}

impl Target {
    pub fn tool(name: impl Into<String>) -> Self {
        Target::Tool(name.into())
    }
    pub fn toolset(name: impl Into<String>) -> Self {
        Target::Toolset(name.into())
    }

    fn matches(&self, tool: &str, toolset: &str) -> bool {
        match self {
            Target::Tool(t) => t == tool,
            Target::Toolset(ts) => ts == toolset,
            Target::Any => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Effect {
    Allow,
    Deny,
}

#[derive(Debug, Clone)]
struct Rule {
    /// `None` means "any user".
    user: Option<String>,
    target: Target,
    effect: Effect,
}

impl Rule {
    fn applies(&self, user: &str, tool: &str, toolset: &str) -> bool {
        let user_ok = self.user.as_deref().map(|u| u == user).unwrap_or(true);
        user_ok && self.target.matches(tool, toolset)
    }
}

/// An ordered allow/deny policy with a default effect.
#[derive(Debug, Clone)]
pub struct PermissionPolicy {
    default_allow: bool,
    rules: Vec<Rule>,
}

impl PermissionPolicy {
    /// A policy that allows everything unless an explicit deny matches.
    pub fn default_allow() -> Self {
        Self {
            default_allow: true,
            rules: Vec::new(),
        }
    }

    /// A policy that denies everything unless an explicit allow matches.
    pub fn default_deny() -> Self {
        Self {
            default_allow: false,
            rules: Vec::new(),
        }
    }

    /// Allow `user` to use `target`.
    pub fn allow(mut self, user: impl Into<String>, target: Target) -> Self {
        self.rules.push(Rule {
            user: Some(user.into()),
            target,
            effect: Effect::Allow,
        });
        self
    }

    /// Deny `user` from using `target`.
    pub fn deny(mut self, user: impl Into<String>, target: Target) -> Self {
        self.rules.push(Rule {
            user: Some(user.into()),
            target,
            effect: Effect::Deny,
        });
        self
    }

    /// Allow `target` for every user.
    pub fn allow_any_user(mut self, target: Target) -> Self {
        self.rules.push(Rule {
            user: None,
            target,
            effect: Effect::Allow,
        });
        self
    }

    /// Deny `target` for every user.
    pub fn deny_any_user(mut self, target: Target) -> Self {
        self.rules.push(Rule {
            user: None,
            target,
            effect: Effect::Deny,
        });
        self
    }

    /// Decide whether `user` may invoke `tool` (a member of `toolset`).
    pub fn is_allowed(&self, user: &str, tool: &str, toolset: &str) -> bool {
        let mut allowed = false;
        // Deny-overrides: a single matching deny is decisive.
        for r in &self.rules {
            if r.applies(user, tool, toolset) {
                match r.effect {
                    Effect::Deny => return false,
                    Effect::Allow => allowed = true,
                }
            }
        }
        allowed || self.default_allow
    }

    /// Like [`is_allowed`](Self::is_allowed) but returns a
    /// [`ToolError::PermissionDenied`] on rejection.
    pub fn check(&self, user: &str, tool: &str, toolset: &str) -> Result<()> {
        if self.is_allowed(user, tool, toolset) {
            Ok(())
        } else {
            Err(ToolError::PermissionDenied {
                tool: tool.to_string(),
                reason: format!("user `{user}` is not authorized"),
            })
        }
    }

    /// Filter a list of tool descriptors down to those `user` may invoke —
    /// the per-user view used to populate `tools/list`.
    pub fn filter_visible(&self, user: &str, specs: &[ToolSpec]) -> Vec<ToolSpec> {
        specs
            .iter()
            .filter(|s| self.is_allowed(user, &s.name, &s.toolset))
            .cloned()
            .collect()
    }
}

impl Default for PermissionPolicy {
    fn default() -> Self {
        Self::default_deny()
    }
}
