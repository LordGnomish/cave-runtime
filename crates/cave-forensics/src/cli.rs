// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl forensics ...` command surface. Returns a process exit code
//! and (in tests) the textual stdout from the matched command.
//!
//! Wired into `cave-cli/src/main.rs` separately — this module only
//! provides the dispatch function so multiple parallel agents can land
//! the CLI binding without touching the same file at the same time.

use crate::observability::{alert_rules, dashboard_panels};
use crate::tracing_policy::TracingPolicy;

/// Top-level dispatch. Subcommands:
///   - `policy {validate <path> | list}` — validate or list policies
///   - `events {kinds}` — enumerate event variants
///   - `filter {ops}` — enumerate supported filter operators
///   - `enforce {actions}` — enumerate enforcement actions
///   - `case {list | new <title>}` — case-store stubs (in-process)
///   - `observability {panels | alerts}` — dump observability artefacts
pub fn dispatch(args: &[String]) -> Result<i32, String> {
    let (code, _out) = dispatch_with_output(args)?;
    Ok(code)
}

/// Same as [`dispatch`] but returns the textual stdout output so tests
/// can assert on what would be printed.
pub fn dispatch_with_output(args: &[String]) -> Result<(i32, String), String> {
    let (cmd, rest) = args
        .split_first()
        .map(|(h, t)| (h.as_str(), t))
        .unwrap_or(("help", &[][..]));
    match cmd {
        "policy" => policy(rest),
        "events" => events(rest),
        "filter" => filter(rest),
        "enforce" => enforce(rest),
        "case" => case(rest),
        "observability" => observability(rest),
        "help" => Ok((0, HELP.to_string())),
        other => Err(format!("unknown subcommand: {other} — try `help`")),
    }
}

const HELP: &str = "cavectl forensics — runtime forensics (Tetragon)
Subcommands:
  policy {validate <json-path> | list}
  events kinds
  filter ops
  enforce actions
  case {list | new <title>}
  observability {panels | alerts}
";

fn policy(rest: &[String]) -> Result<(i32, String), String> {
    match rest.split_first() {
        Some((s, _)) if s == "list" => Ok((0, "(empty — no in-process policy store wired)\n".into())),
        Some((s, tail)) if s == "validate" => {
            let path = tail.first().ok_or("`validate` requires <json-path>")?;
            let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
            let p = TracingPolicy::parse_json(&text).map_err(|e| e.to_string())?;
            Ok((0, format!("policy {} valid\n", p.metadata.name)))
        }
        _ => Err("policy expects `validate <path>` or `list`".into()),
    }
}

fn events(rest: &[String]) -> Result<(i32, String), String> {
    match rest.first().map(|s| s.as_str()) {
        Some("kinds") => Ok((
            0,
            "process_exec process_exit file_op network capability bpf_load kprobe uprobe\n".into(),
        )),
        _ => Err("events expects `kinds`".into()),
    }
}

fn filter(rest: &[String]) -> Result<(i32, String), String> {
    match rest.first().map(|s| s.as_str()) {
        Some("ops") => Ok((
            0,
            "In NotIn Equal NotEqual Prefix Postfix Mask\n".into(),
        )),
        _ => Err("filter expects `ops`".into()),
    }
}

fn enforce(rest: &[String]) -> Result<(i32, String), String> {
    match rest.first().map(|s| s.as_str()) {
        Some("actions") => Ok((
            0,
            "Post Override Sigkill FollowFd Signal NoPost UnfollowFd GetUrl DnsLookup NotifyEnforcer\n".into(),
        )),
        _ => Err("enforce expects `actions`".into()),
    }
}

fn case(rest: &[String]) -> Result<(i32, String), String> {
    match rest.split_first() {
        Some((s, _)) if s == "list" => Ok((0, "(no cases — in-process store)\n".into())),
        Some((s, tail)) if s == "new" => {
            let title = tail.first().ok_or("`new` requires <title>")?;
            Ok((0, format!("case {} created (in-process)\n", title)))
        }
        _ => Err("case expects `list` or `new <title>`".into()),
    }
}

fn observability(rest: &[String]) -> Result<(i32, String), String> {
    match rest.first().map(|s| s.as_str()) {
        Some("panels") => {
            let mut out = String::new();
            for p in dashboard_panels() {
                out.push_str(&format!("{}\t{}\n", p.metric, p.title));
            }
            Ok((0, out))
        }
        Some("alerts") => {
            let mut out = String::new();
            for r in alert_rules() {
                out.push_str(&format!("{}\t{}\n", r.severity, r.name));
            }
            Ok((0, out))
        }
        _ => Err("observability expects `panels` or `alerts`".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn test_help_returns_zero() {
        let (code, out) = dispatch_with_output(&s(&["help"])).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("Subcommands:"));
    }

    #[test]
    fn test_no_args_defaults_to_help() {
        let (code, out) = dispatch_with_output(&[]).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("forensics"));
    }

    #[test]
    fn test_unknown_subcommand_errors() {
        assert!(dispatch(&s(&["nope"])).is_err());
    }

    #[test]
    fn test_events_kinds_lists_all_variants() {
        let (code, out) = dispatch_with_output(&s(&["events", "kinds"])).unwrap();
        assert_eq!(code, 0);
        for k in [
            "process_exec",
            "process_exit",
            "file_op",
            "network",
            "capability",
            "bpf_load",
            "kprobe",
            "uprobe",
        ] {
            assert!(out.contains(k), "{k} missing from output: {out}");
        }
    }

    #[test]
    fn test_filter_ops_lists_all_ops() {
        let (_code, out) = dispatch_with_output(&s(&["filter", "ops"])).unwrap();
        for o in ["In", "NotIn", "Equal", "NotEqual", "Prefix", "Postfix", "Mask"] {
            assert!(out.contains(o));
        }
    }

    #[test]
    fn test_enforce_actions_lists_all_actions() {
        let (_code, out) = dispatch_with_output(&s(&["enforce", "actions"])).unwrap();
        for a in ["Post", "Override", "Sigkill", "FollowFd", "Signal"] {
            assert!(out.contains(a));
        }
    }

    #[test]
    fn test_observability_panels_emits_8_lines() {
        let (_code, out) = dispatch_with_output(&s(&["observability", "panels"])).unwrap();
        assert_eq!(out.lines().count(), 8);
    }

    #[test]
    fn test_observability_alerts_emits_5_lines() {
        let (_code, out) = dispatch_with_output(&s(&["observability", "alerts"])).unwrap();
        assert_eq!(out.lines().count(), 5);
    }

    #[test]
    fn test_case_new_requires_title() {
        assert!(dispatch_with_output(&s(&["case", "new"])).is_err());
        let (_c, out) = dispatch_with_output(&s(&["case", "new", "title-1"])).unwrap();
        assert!(out.contains("title-1"));
    }

    #[test]
    fn test_policy_validate_via_tempfile() {
        let mut tmp = std::env::temp_dir();
        tmp.push("cave-forensics-cli-policy.json");
        let mut f = std::fs::File::create(&tmp).unwrap();
        write!(
            f,
            r#"{{
              "api_version":"cilium.io/v1alpha1",
              "kind":"TracingPolicy",
              "metadata":{{"name":"p1"}},
              "spec":{{"kprobes":[{{"call":"sys_open","syscall":true,"return_":false}}]}}
            }}"#
        )
        .unwrap();
        let path = tmp.to_string_lossy().to_string();
        let (code, out) = dispatch_with_output(&s(&["policy", "validate", &path])).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("p1 valid"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_policy_list_returns_empty_message() {
        let (code, out) = dispatch_with_output(&s(&["policy", "list"])).unwrap();
        assert_eq!(code, 0);
        assert!(out.contains("empty"));
    }
}
