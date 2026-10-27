// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `/admin/hermes` — Hermes agent-orchestration admin view.
//!
//! A read-only overview of the cave-hermes Backend surface: the built-in tool
//! catalogue, the agent-loop / orchestrator endpoints the runtime mounts under
//! `/api/hermes`, and the subsystems the crate ports. The live data is served
//! by the runtime; this page documents the surface and links the operator to
//! the JSON endpoints.

use crate::admin::permission::{AuthError, Permission, RequestCtx};
use crate::admin::render::{escape, page_shell_full};

/// The four MVP built-in tools cave-hermes registers by default.
const BUILTIN_TOOLS: [(&str, &str); 4] = [
    ("bash", "Run a shell command via /bin/sh"),
    ("file_read", "Read a file from disk"),
    ("file_write", "Write a file to disk"),
    ("web_fetch", "Fetch the body of a URL"),
];

/// The HTTP endpoints the runtime exposes under `/api/hermes`.
const ENDPOINTS: [(&str, &str, &str); 4] = [
    ("GET", "/api/hermes/health", "liveness + upstream pin"),
    ("GET", "/api/hermes/tools", "JSON-Schema tool catalogue"),
    ("POST", "/api/hermes/agent/run", "run the agent loop on a goal"),
    (
        "POST",
        "/api/hermes/orchestrate",
        "fan a subtask graph out across worker agents",
    ),
];

pub fn render(ctx: &RequestCtx) -> Result<String, AuthError> {
    ctx.authorise(Permission::HermesRead)?;

    let mut tool_rows = String::new();
    for (name, desc) in BUILTIN_TOOLS {
        tool_rows.push_str(&format!(
            r#"<tr><td class="font-mono">{n}</td><td class="text-gray-600">{d}</td></tr>"#,
            n = escape(name),
            d = escape(desc),
        ));
    }

    let mut endpoint_rows = String::new();
    for (verb, path, desc) in ENDPOINTS {
        endpoint_rows.push_str(&format!(
            r#"<tr><td class="font-mono text-xs">{v}</td><td class="font-mono">{p}</td><td class="text-gray-600">{d}</td></tr>"#,
            v = escape(verb),
            p = escape(path),
            d = escape(desc),
        ));
    }

    let body = format!(
        r#"<section class="grid grid-cols-3 gap-3 mb-4">
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">built-in tools</div><div class="text-2xl font-bold">{tools}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">endpoints</div><div class="text-2xl font-bold">{eps}</div></div>
  <div class="bg-white rounded shadow p-3"><div class="text-xs text-gray-500">upstream</div><div class="text-lg font-bold">Hermes Agent</div></div>
</section>
<h2 class="text-lg font-semibold mb-2">Tool catalogue</h2>
<table class="w-full mb-5 text-sm bg-white rounded shadow">
  <thead><tr class="text-left text-gray-500"><th class="p-2">tool</th><th class="p-2">description</th></tr></thead>
  <tbody>{tool_rows}</tbody>
</table>
<h2 class="text-lg font-semibold mb-2">HTTP surface</h2>
<table class="w-full text-sm bg-white rounded shadow">
  <thead><tr class="text-left text-gray-500"><th class="p-2">method</th><th class="p-2">path</th><th class="p-2">purpose</th></tr></thead>
  <tbody>{endpoint_rows}</tbody>
</table>"#,
        tools = BUILTIN_TOOLS.len(),
        eps = ENDPOINTS.len(),
        tool_rows = tool_rows,
        endpoint_rows = endpoint_rows,
    );

    Ok(page_shell_full(
        ctx,
        "/admin/hermes",
        &format!("hermes · {}", escape(ctx.tenant.as_str())),
        &body,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn render_refuses_without_perm() {
        assert!(render(&ctx(&[])).is_err());
    }

    #[test]
    fn render_lists_tools_and_endpoints() {
        let html = render(&ctx(&[Permission::HermesRead])).unwrap();
        for tool in ["bash", "file_read", "file_write", "web_fetch"] {
            assert!(html.contains(tool), "missing tool {tool}");
        }
        assert!(html.contains("/api/hermes/agent/run"));
        assert!(html.contains("/api/hermes/orchestrate"));
    }
}
