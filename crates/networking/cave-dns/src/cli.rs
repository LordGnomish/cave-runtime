// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl dns` subcommand dispatcher — provided as a library so the cave-cli
//! binary can wire it without forcing cross-crate edits.
//!
//! The dispatcher is intentionally framework-agnostic: it accepts the raw
//! argv tail and returns a typed `DnsCommand` that callers map to runtime
//! actions. This keeps the test surface tight (parser-only) and avoids
//! pulling clap into the public API.

use serde::{Deserialize, Serialize};

/// Parsed `cavectl dns ...` invocation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DnsCommand {
    /// `cavectl dns query <name> [type]`
    Query { name: String, qtype: String },
    /// `cavectl dns zone {list|show|reload} [zone]`
    Zone { action: ZoneAction, zone: Option<String> },
    /// `cavectl dns plugin {list|describe} [name]`
    Plugin { action: PluginAction, name: Option<String> },
    /// `cavectl dns cache {stats|flush}`
    Cache { action: CacheAction },
    /// `cavectl dns reload`
    Reload,
    /// `cavectl dns corefile {validate|show} <path>`
    Corefile { action: CorefileAction, path: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CorefileAction {
    /// Parse the Corefile and report errors (exit non-zero on failure).
    Validate,
    /// Parse the Corefile and print the resolved server blocks.
    Show,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ZoneAction {
    List,
    Show,
    Reload,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginAction {
    List,
    Describe,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CacheAction {
    Stats,
    Flush,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CliError {
    #[error("dns: missing subcommand (one of: query, zone, plugin, cache, reload, corefile)")]
    MissingSubcommand,
    #[error("dns: unknown subcommand: {0}")]
    UnknownSubcommand(String),
    #[error("dns: {0} requires {1}")]
    MissingArgument(&'static str, &'static str),
    #[error("dns: {0} action {1:?} not recognised")]
    UnknownAction(&'static str, String),
    #[error("dns: unexpected positional argument: {0}")]
    UnexpectedArgument(String),
}

/// Parse the argv tail of `cavectl dns ...` into a typed command.
///
/// `argv` must NOT include the leading `dns` token — callers strip that
/// when they dispatch to this function.
pub fn parse(argv: &[&str]) -> Result<DnsCommand, CliError> {
    let mut iter = argv.iter().copied();
    let sub = iter.next().ok_or(CliError::MissingSubcommand)?;
    match sub {
        "query" => {
            let name = iter
                .next()
                .ok_or(CliError::MissingArgument("query", "<name>"))?
                .to_string();
            let qtype = iter.next().unwrap_or("A").to_string();
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.to_string()));
            }
            Ok(DnsCommand::Query { name, qtype })
        }
        "zone" => {
            let action_str = iter
                .next()
                .ok_or(CliError::MissingArgument("zone", "<action>"))?;
            let action = match action_str {
                "list" => ZoneAction::List,
                "show" => ZoneAction::Show,
                "reload" => ZoneAction::Reload,
                other => return Err(CliError::UnknownAction("zone", other.to_string())),
            };
            let zone = iter.next().map(str::to_string);
            if matches!(action, ZoneAction::Show | ZoneAction::Reload) && zone.is_none() {
                return Err(CliError::MissingArgument("zone show|reload", "<zone>"));
            }
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.to_string()));
            }
            Ok(DnsCommand::Zone { action, zone })
        }
        "plugin" => {
            let action_str = iter
                .next()
                .ok_or(CliError::MissingArgument("plugin", "<action>"))?;
            let action = match action_str {
                "list" => PluginAction::List,
                "describe" => PluginAction::Describe,
                other => return Err(CliError::UnknownAction("plugin", other.to_string())),
            };
            let name = iter.next().map(str::to_string);
            if matches!(action, PluginAction::Describe) && name.is_none() {
                return Err(CliError::MissingArgument("plugin describe", "<name>"));
            }
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.to_string()));
            }
            Ok(DnsCommand::Plugin { action, name })
        }
        "cache" => {
            let action_str = iter
                .next()
                .ok_or(CliError::MissingArgument("cache", "<action>"))?;
            let action = match action_str {
                "stats" => CacheAction::Stats,
                "flush" => CacheAction::Flush,
                other => return Err(CliError::UnknownAction("cache", other.to_string())),
            };
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.to_string()));
            }
            Ok(DnsCommand::Cache { action })
        }
        "reload" => {
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.to_string()));
            }
            Ok(DnsCommand::Reload)
        }
        "corefile" => {
            let action_str = iter
                .next()
                .ok_or(CliError::MissingArgument("corefile", "<action>"))?;
            let action = match action_str {
                "validate" => CorefileAction::Validate,
                "show" => CorefileAction::Show,
                other => return Err(CliError::UnknownAction("corefile", other.to_string())),
            };
            let path = iter
                .next()
                .ok_or(CliError::MissingArgument("corefile validate|show", "<path>"))?
                .to_string();
            if let Some(extra) = iter.next() {
                return Err(CliError::UnexpectedArgument(extra.to_string()));
            }
            Ok(DnsCommand::Corefile { action, path })
        }
        other => Err(CliError::UnknownSubcommand(other.to_string())),
    }
}

/// Render a human-readable help string for `cavectl dns ...`.
pub fn help() -> &'static str {
    "cavectl dns <subcommand> [...]\n\
     \n\
     Subcommands:\n\
     \tquery   <name> [type]            issue a one-shot DNS query\n\
     \tzone    list|show|reload [zone]  inspect / reload zones\n\
     \tplugin  list|describe [name]     list plugins / describe one\n\
     \tcache   stats|flush              cache introspection\n\
     \treload                           reload the running config\n\
     \tcorefile validate|show <path>   parse / validate a Corefile\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_query_with_default_type() {
        let cmd = parse(&["query", "example.com"]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Query {
                name: "example.com".into(),
                qtype: "A".into(),
            }
        );
    }

    #[test]
    fn parse_query_with_explicit_type() {
        let cmd = parse(&["query", "example.com", "AAAA"]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Query {
                name: "example.com".into(),
                qtype: "AAAA".into(),
            }
        );
    }

    #[test]
    fn parse_zone_list_without_zone() {
        let cmd = parse(&["zone", "list"]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Zone {
                action: ZoneAction::List,
                zone: None,
            }
        );
    }

    #[test]
    fn parse_zone_show_requires_zone() {
        let err = parse(&["zone", "show"]).unwrap_err();
        assert_eq!(err, CliError::MissingArgument("zone show|reload", "<zone>"));
    }

    #[test]
    fn parse_zone_reload_with_zone() {
        let cmd = parse(&["zone", "reload", "example.com."]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Zone {
                action: ZoneAction::Reload,
                zone: Some("example.com.".into()),
            }
        );
    }

    #[test]
    fn parse_plugin_describe_requires_name() {
        let err = parse(&["plugin", "describe"]).unwrap_err();
        assert_eq!(err, CliError::MissingArgument("plugin describe", "<name>"));
    }

    #[test]
    fn parse_plugin_list_ok() {
        let cmd = parse(&["plugin", "list"]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Plugin {
                action: PluginAction::List,
                name: None,
            }
        );
    }

    #[test]
    fn parse_cache_stats() {
        let cmd = parse(&["cache", "stats"]).unwrap();
        assert_eq!(cmd, DnsCommand::Cache { action: CacheAction::Stats });
    }

    #[test]
    fn parse_cache_flush() {
        let cmd = parse(&["cache", "flush"]).unwrap();
        assert_eq!(cmd, DnsCommand::Cache { action: CacheAction::Flush });
    }

    #[test]
    fn parse_reload_takes_no_args() {
        let cmd = parse(&["reload"]).unwrap();
        assert_eq!(cmd, DnsCommand::Reload);
        let err = parse(&["reload", "now"]).unwrap_err();
        assert_eq!(err, CliError::UnexpectedArgument("now".into()));
    }

    #[test]
    fn parse_unknown_subcommand_errors() {
        let err = parse(&["dump"]).unwrap_err();
        assert_eq!(err, CliError::UnknownSubcommand("dump".into()));
    }

    #[test]
    fn parse_empty_argv_errors() {
        assert_eq!(parse(&[]).unwrap_err(), CliError::MissingSubcommand);
    }

    #[test]
    fn parse_unknown_zone_action_errors() {
        let err = parse(&["zone", "destroy"]).unwrap_err();
        assert_eq!(err, CliError::UnknownAction("zone", "destroy".into()));
    }

    #[test]
    fn parse_unknown_cache_action_errors() {
        let err = parse(&["cache", "warm"]).unwrap_err();
        assert_eq!(err, CliError::UnknownAction("cache", "warm".into()));
    }

    #[test]
    fn parse_rejects_unexpected_query_args() {
        let err = parse(&["query", "example.com", "A", "more"]).unwrap_err();
        assert_eq!(err, CliError::UnexpectedArgument("more".into()));
    }

    #[test]
    fn help_mentions_all_subcommands() {
        let h = help();
        for sub in ["query", "zone", "plugin", "cache", "reload", "corefile"] {
            assert!(h.contains(sub), "help missing subcommand {sub}");
        }
    }

    // ── Cycle 6: corefile subcommand ───────────────────────────────────────

    #[test]
    fn parse_corefile_validate_with_path() {
        let cmd = parse(&["corefile", "validate", "/etc/cave/Corefile"]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Corefile {
                action: CorefileAction::Validate,
                path: "/etc/cave/Corefile".into(),
            }
        );
    }

    #[test]
    fn parse_corefile_show_with_path() {
        let cmd = parse(&["corefile", "show", "Corefile"]).unwrap();
        assert_eq!(
            cmd,
            DnsCommand::Corefile {
                action: CorefileAction::Show,
                path: "Corefile".into(),
            }
        );
    }

    #[test]
    fn parse_corefile_requires_path() {
        let err = parse(&["corefile", "validate"]).unwrap_err();
        assert_eq!(err, CliError::MissingArgument("corefile validate|show", "<path>"));
    }

    #[test]
    fn parse_corefile_unknown_action_errors() {
        let err = parse(&["corefile", "lint", "Corefile"]).unwrap_err();
        assert_eq!(err, CliError::UnknownAction("corefile", "lint".into()));
    }
}
