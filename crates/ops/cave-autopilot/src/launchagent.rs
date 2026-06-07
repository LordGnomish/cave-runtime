// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! macOS LaunchAgent plist generation + (un)install.
//!
//! Two instances run as separate LaunchAgents:
//!
//! * `com.gnomish.cave-runtime-autopilot` → port 9101, cave-runtime repo
//! * `com.gnomish.cave-home-autopilot`    → port 9102, cave-home repo
//!
//! Both use `KeepAlive=true` + `RunAtLoad=true`, so launchd restarts the daemon
//! if it dies and starts it at login. plist *rendering* is pure (and tested);
//! [`install`] writes the file under `~/Library/LaunchAgents/` and `launchctl
//! load -w`s it.

use crate::config::AutopilotConfig;
use crate::error::{AutopilotError, Result};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Everything needed to emit a plist.
#[derive(Debug, Clone)]
pub struct PlistSpec {
    pub label: String,
    pub binary_path: String,
    pub instance: String,
    pub config_path: String,
    pub log_dir: String,
    pub claude_token_budget: u64,
}

impl PlistSpec {
    /// Derive a spec from a config + the installed binary + config-file paths.
    pub fn from_config(cfg: &AutopilotConfig, binary_path: &str, config_path: &str) -> Self {
        Self {
            label: cfg.launch_label(),
            binary_path: binary_path.to_string(),
            instance: cfg.instance.clone(),
            config_path: config_path.to_string(),
            log_dir: format!(
                "{}/Library/Logs",
                std::env::var("HOME").unwrap_or_else(|_| "/tmp".into())
            ),
            claude_token_budget: cfg.claude_daily_token_budget,
        }
    }

    /// Render the LaunchAgent property list XML.
    pub fn render(&self) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>{label}</string>
    <key>ProgramArguments</key>
    <array>
        <string>{bin}</string>
        <string>daemon</string>
        <string>--instance</string>
        <string>{instance}</string>
        <string>--config</string>
        <string>{cfg}</string>
    </array>
    <key>EnvironmentVariables</key>
    <dict>
        <key>RUST_LOG</key>
        <string>info</string>
        <key>CAVE_AUTOPILOT_CLAUDE_TOKEN_BUDGET</key>
        <string>{budget}</string>
    </dict>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{logdir}/{label}.log</string>
    <key>StandardErrorPath</key>
    <string>{logdir}/{label}.err.log</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
"#,
            label = self.label,
            bin = self.binary_path,
            instance = self.instance,
            cfg = self.config_path,
            budget = self.claude_token_budget,
            logdir = self.log_dir,
        )
    }
}

/// `~/Library/LaunchAgents/<label>.plist`.
pub fn plist_path(label: &str) -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{label}.plist"))
}

/// Arguments for `launchctl load -w <plist>`.
pub fn load_args(plist: &Path) -> Vec<String> {
    vec!["load".into(), "-w".into(), plist.to_string_lossy().into_owned()]
}

/// Arguments for `launchctl unload -w <plist>`.
pub fn unload_args(plist: &Path) -> Vec<String> {
    vec!["unload".into(), "-w".into(), plist.to_string_lossy().into_owned()]
}

/// Write the plist to `~/Library/LaunchAgents/` and load it via launchctl.
/// Returns the path written.
pub fn install(spec: &PlistSpec) -> Result<PathBuf> {
    let path = plist_path(&spec.label);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&path, spec.render())?;
    // Best-effort load; report a clear error if launchctl is unavailable.
    let out = Command::new("launchctl")
        .args(load_args(&path))
        .output()
        .map_err(|e| AutopilotError::Config(format!("launchctl load: {e}")))?;
    if !out.status.success() {
        return Err(AutopilotError::Config(format!(
            "launchctl load failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )));
    }
    Ok(path)
}

/// Unload + remove the plist.
pub fn uninstall(label: &str) -> Result<()> {
    let path = plist_path(label);
    let _ = Command::new("launchctl").args(unload_args(&path)).output();
    if path.exists() {
        std::fs::remove_file(&path)?;
    }
    Ok(())
}

/// Emit a shell script that pulls the tiered Ollama models, falling back to the
/// resident coding model when the aspirational MoE checkpoints are unavailable.
/// Written rather than executed so Burak controls the (large) downloads.
pub fn ollama_setup_script(cfg: &AutopilotConfig) -> String {
    format!(
        r#"#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# cave-autopilot — Ollama model setup. Generated; review before running.
set -euo pipefail

OLLAMA_URL="{url}"

echo "== Checking Ollama at $OLLAMA_URL =="
if ! curl -sf "$OLLAMA_URL/api/tags" >/dev/null; then
  echo "Ollama not reachable at $OLLAMA_URL — start it with 'ollama serve'." >&2
  exit 1
fi

pull_if_missing() {{
  local model="$1"
  if ollama list | awk '{{print $1}}' | grep -qx "$model"; then
    echo "  [ok] $model already pulled"
  else
    echo "  [pull] $model"
    ollama pull "$model" || echo "  [warn] could not pull $model (will fall back)"
  fi
}}

echo "== L1 router =="
pull_if_missing "{l1}"
echo "== L2 coder =="
pull_if_missing "{l2}"
echo "== Resident fallback (must exist) =="
pull_if_missing "{fallback}"

echo "Done. Autopilot resolves named models, else falls back to {fallback}."
"#,
        url = cfg.ollama_url,
        l1 = cfg.model_l1_router,
        l2 = cfg.model_l2_coder,
        fallback = cfg.model_fallback,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> PlistSpec {
        PlistSpec {
            label: "com.gnomish.cave-runtime-autopilot".into(),
            binary_path: "/Users/gnomish/.local/bin/cave-autopilot".into(),
            instance: "cave-runtime".into(),
            config_path: "/Users/gnomish/.config/cave-autopilot/cave-runtime.toml".into(),
            log_dir: "/Users/gnomish/Library/Logs".into(),
            claude_token_budget: 2_000_000,
        }
    }

    #[test]
    fn plist_has_keepalive_runatload_and_daemon_args() {
        let p = spec().render();
        assert!(p.contains("<key>KeepAlive</key>\n    <true/>"));
        assert!(p.contains("<key>RunAtLoad</key>\n    <true/>"));
        assert!(p.contains("<string>com.gnomish.cave-runtime-autopilot</string>"));
        assert!(p.contains("<string>daemon</string>"));
        assert!(p.contains("<string>--instance</string>"));
        assert!(p.contains("<string>cave-runtime</string>"));
        assert!(p.contains("CAVE_AUTOPILOT_CLAUDE_TOKEN_BUDGET"));
        assert!(p.contains("<string>2000000</string>"));
    }

    #[test]
    fn plist_path_under_launchagents() {
        let p = plist_path("com.gnomish.cave-home-autopilot");
        assert!(p.to_string_lossy().contains("Library/LaunchAgents/com.gnomish.cave-home-autopilot.plist"));
    }

    #[test]
    fn launchctl_arg_builders() {
        let plist = PathBuf::from("/tmp/x.plist");
        assert_eq!(load_args(&plist), vec!["load", "-w", "/tmp/x.plist"]);
        assert_eq!(unload_args(&plist), vec!["unload", "-w", "/tmp/x.plist"]);
    }

    #[test]
    fn from_config_uses_instance_label_and_budget() {
        let cfg = AutopilotConfig::for_instance("cave-home");
        let s = PlistSpec::from_config(&cfg, "/bin/cave-autopilot", "/cfg.toml");
        assert_eq!(s.label, "com.gnomish.cave-home-autopilot");
        assert_eq!(s.instance, "cave-home");
    }

    #[test]
    fn setup_script_references_all_tiers() {
        let cfg = AutopilotConfig::default();
        let sh = ollama_setup_script(&cfg);
        assert!(sh.contains("mellum2:12b-moe"));
        assert!(sh.contains("qwen3-coder-next:80b-moe"));
        assert!(sh.contains("qwen3.6:35b-a3b-coding-mxfp8"));
        assert!(sh.starts_with("#!/usr/bin/env bash"));
    }
}
