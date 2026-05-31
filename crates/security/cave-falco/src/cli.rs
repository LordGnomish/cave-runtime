// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! `cavectl falco` CLI surface — synchronous in-process dispatcher.
//! Wired by the eksik-sweep ray (pending follow-up: cave-cli/Cargo.toml +
//! Commands::Falco variant; not yet attached so the crate can land
//! independent of cave-cli changes).

use crate::error::Result;
use crate::observability;
use crate::{engine, falcoctl, rule_loader};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FalcoSubcommand {
    /// `cavectl falco rules-parse --path <pack.yaml>` — parse, print rule count.
    RulesParse { path: String },
    /// `cavectl falco rules-list-builtin` — built-in starter pack count.
    RulesListBuiltin,
    /// `cavectl falco observability` — print panels + alert YAML.
    Observability,
    /// `cavectl falco operators` — list the supported filter operators.
    Operators,
    /// `cavectl falco artifact-resolve --index <index.yaml> --ref <name>` —
    /// resolve a falcoctl artifact reference against an index.
    ArtifactResolve { index: String, reference: String },
    /// `cavectl falco version` — print crate + upstream version.
    Version,
}

pub fn dispatch(cmd: FalcoSubcommand) -> Result<String> {
    match cmd {
        FalcoSubcommand::RulesParse { path } => {
            let body = std::fs::read_to_string(&path)
                .map_err(|e| crate::error::FalcoError::Internal(format!("read {path}: {e}")))?;
            let pack = rule_loader::parse(&body)?;
            Ok(format!(
                "parsed: {} rule(s) / {} macro(s) / {} list(s)\n",
                pack.rules.len(), pack.macros.len(), pack.lists.len()
            ))
        }
        FalcoSubcommand::RulesListBuiltin => Ok("cave-falco ships no built-in rules; pack-loaded at runtime\n".into()),
        FalcoSubcommand::Observability => {
            let mut out = String::new();
            out.push_str("# Dashboard panels\n");
            for p in observability::dashboard_panels() {
                out.push_str(&format!("- {} → {}\n", p.title, p.query));
            }
            out.push_str("\n# Alert rules\n");
            out.push_str(&observability::alert_rules_yaml());
            Ok(out)
        }
        FalcoSubcommand::Operators => {
            Ok(format!("supported filter operators: {}\n", engine::supported_operators().join(" ")))
        }
        FalcoSubcommand::ArtifactResolve { index, reference } => {
            let body = std::fs::read_to_string(&index)
                .map_err(|e| crate::error::FalcoError::Internal(format!("read {index}: {e}")))?;
            let idx = falcoctl::Index::from_yaml("cli", &body)?;
            let resolved = idx.resolve_reference(&reference)?;
            Ok(format!("{resolved}\n"))
        }
        FalcoSubcommand::Version => Ok("cave-falco upstream falcosecurity/falco@0.43.1\n".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_list_builtin_returns_message() {
        let out = dispatch(FalcoSubcommand::RulesListBuiltin).unwrap();
        assert!(out.contains("no built-in"));
    }

    #[test]
    fn version_includes_upstream_version() {
        let out = dispatch(FalcoSubcommand::Version).unwrap();
        assert!(out.contains("0.43.1"));
    }

    #[test]
    fn observability_includes_alert_yaml() {
        let out = dispatch(FalcoSubcommand::Observability).unwrap();
        assert!(out.contains("FalcoCriticalAlertSurge"));
        assert!(out.contains("alert:"));
    }

    #[test]
    fn rules_parse_unreadable_path_errors() {
        let r = dispatch(FalcoSubcommand::RulesParse { path: "/no/such/path-cave-falco-test".into() });
        assert!(r.is_err());
    }

    #[test]
    fn operators_lists_filter_ops() {
        let out = dispatch(FalcoSubcommand::Operators).unwrap();
        assert!(out.contains("regex"));
        assert!(out.contains("pmatch"));
        assert!(out.contains("intersects"));
    }

    #[test]
    fn artifact_resolve_reads_index_and_resolves() {
        let mut f = std::env::temp_dir();
        f.push("cave-falco-cli-index.yaml");
        std::fs::write(&f, "- name: cloudtrail\n  type: plugin\n  registry: ghcr.io\n  repository: falcosecurity/plugins/cloudtrail\n").unwrap();
        let out = dispatch(FalcoSubcommand::ArtifactResolve {
            index: f.to_string_lossy().into_owned(),
            reference: "cloudtrail:0.5.1".into(),
        }).unwrap();
        assert_eq!(out.trim(), "ghcr.io/falcosecurity/plugins/cloudtrail:0.5.1");
        let _ = std::fs::remove_file(&f);
    }

    #[test]
    fn artifact_resolve_missing_index_errors() {
        let r = dispatch(FalcoSubcommand::ArtifactResolve {
            index: "/no/such/index-cave-falco.yaml".into(),
            reference: "x".into(),
        });
        assert!(r.is_err());
    }
}
