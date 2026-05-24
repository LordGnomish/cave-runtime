// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl bench` CLI surface.
//!
//! Wired post-merge by the orchestrator into crates/cave-cli/src/main.rs.
//! This module only declares the subcommand shape + a synchronous
//! dispatcher so cave-bench is reachable without the cave-cli crate.

use crate::error::Result;
use crate::report::{Format, render};
use crate::runner::{RunMode, ScanInput, run_profile};
use crate::models::Target;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BenchSubcommand {
    /// `cavectl bench scan --profile <id> --host <host>` — run one profile.
    Scan { profile_id: String, host: String, format: Format },
    /// `cavectl bench profiles` — list profiles.
    Profiles,
    /// `cavectl bench checks --framework <fw>` — list checks.
    Checks { framework: String },
    /// `cavectl bench schedules` — list scheduled scans.
    Schedules,
    /// `cavectl bench observability` — print dashboards + alerts.
    Observability,
}

/// Dispatch a CLI subcommand. Returns the textual output to print.
pub fn dispatch(cmd: BenchSubcommand) -> Result<String> {
    match cmd {
        BenchSubcommand::Scan { profile_id, host, format } => {
            let profile = crate::profile::find_profile(&profile_id)?;
            let target = Target::host_files("/etc/kubernetes", host.clone());
            let input = ScanInput::new(host);
            let (findings, summary) = run_profile(&profile, &target, &input, RunMode::Sequential);
            Ok(render(format, &findings, &summary))
        }
        BenchSubcommand::Profiles => {
            let profiles = crate::profile::builtin_profiles();
            let mut out = String::new();
            out.push_str("PROFILE              FRAMEWORK          CHECKS\n");
            for p in profiles {
                out.push_str(&format!("{:<20} {:<18} {}\n", p.id, p.framework.as_str(), p.check_ids.len()));
            }
            Ok(out)
        }
        BenchSubcommand::Checks { framework } => {
            let mut out = String::new();
            match framework.as_str() {
                "cis" => {
                    for (c, _) in crate::runner::cis_pairs() {
                        out.push_str(&format!("{:<14} {:<14} {}\n", c.id, c.node_type.as_str(), c.title));
                    }
                }
                "nsa" => {
                    for c in crate::kubescape_nsa::nsa_controls() {
                        out.push_str(&format!("{:<10} {:<10} {}\n", c.check.id, c.check.severity.as_str(), c.check.title));
                    }
                }
                "mitre" => {
                    for t in crate::kubescape_mitre::mitre_techniques() {
                        out.push_str(&format!("{:<12} {:<25} {}\n", t.id, t.tactic.as_str(), t.check.title));
                    }
                }
                _ => return Err(crate::error::BenchError::Internal(format!("unknown framework '{framework}'"))),
            }
            Ok(out)
        }
        BenchSubcommand::Schedules => Ok("no scheduled scans configured (cave-cli runs in stateless mode)\n".into()),
        BenchSubcommand::Observability => {
            let mut out = String::new();
            out.push_str("# Dashboard panels\n");
            for p in crate::observability::dashboard_panels() {
                out.push_str(&format!("- {} → {}\n", p.title, p.query));
            }
            out.push_str("\n# Alert rules\n");
            out.push_str(&crate::observability::alert_rules_yaml());
            Ok(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dispatch_scan_returns_output() {
        let out = dispatch(BenchSubcommand::Scan {
            profile_id: "cis-1.10".into(),
            host: "test".into(),
            format: Format::Markdown,
        })
        .unwrap();
        assert!(out.contains("cave-bench report"));
    }

    #[test]
    fn test_dispatch_profiles_lists() {
        let out = dispatch(BenchSubcommand::Profiles).unwrap();
        assert!(out.contains("cis-1.10"));
        assert!(out.contains("nsa-2025"));
    }

    #[test]
    fn test_dispatch_checks_cis() {
        let out = dispatch(BenchSubcommand::Checks { framework: "cis".into() }).unwrap();
        assert!(out.contains("cis-1.2.1"));
    }

    #[test]
    fn test_dispatch_checks_nsa() {
        let out = dispatch(BenchSubcommand::Checks { framework: "nsa".into() }).unwrap();
        assert!(out.contains("C-0057"));
    }

    #[test]
    fn test_dispatch_checks_mitre() {
        let out = dispatch(BenchSubcommand::Checks { framework: "mitre".into() }).unwrap();
        assert!(out.contains("T1611"));
    }

    #[test]
    fn test_dispatch_observability_includes_yaml() {
        let out = dispatch(BenchSubcommand::Observability).unwrap();
        assert!(out.contains("alert:"));
    }

    #[test]
    fn test_dispatch_checks_unknown_framework_errors() {
        let r = dispatch(BenchSubcommand::Checks { framework: "soc-bla".into() });
        assert!(r.is_err());
    }

    #[test]
    fn test_dispatch_schedules_returns_message() {
        let out = dispatch(BenchSubcommand::Schedules).unwrap();
        assert!(out.contains("stateless"));
    }
}
