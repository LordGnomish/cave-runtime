// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! `cavectl alloy …` command surface for the Alloy engine.
//!
//! In-process, upstream-agnostic verbs over the Alloy/River config syntax
//! ([`crate::alloy`]). Mirrors the local-dispatch pattern used by
//! `cave-bench` / `cave-falco` in `cave-cli`: a plain subcommand enum plus a
//! [`dispatch`] returning the rendered string, which keeps the logic testable
//! without a running daemon.

use crate::alloy::parser::parse_file;
use crate::alloy::scanner::Scanner;
use crate::alloy::token::Token;

/// A `cavectl alloy` subcommand.
#[derive(Debug, Clone)]
pub enum AlloySubcommand {
    /// Parse an Alloy config file and report any diagnostics.
    Validate {
        /// Path to the `.alloy` config file.
        path: String,
    },
    /// Parse an Alloy config file and summarise its block/attribute structure.
    Parse {
        /// Path to the `.alloy` config file.
        path: String,
    },
    /// Tokenize an Alloy config file and list the token stream.
    Tokens {
        /// Path to the `.alloy` config file.
        path: String,
    },
    /// Print the tracked upstream version.
    Version,
}

/// Dispatches an [`AlloySubcommand`], returning the rendered output or an error
/// message. File-reading errors are surfaced as `Err`.
pub fn dispatch(sub: AlloySubcommand) -> Result<String, String> {
    match sub {
        AlloySubcommand::Validate { path } => {
            let src = read(&path)?;
            Ok(validate_source(&path, &src))
        }
        AlloySubcommand::Parse { path } => {
            let src = read(&path)?;
            Ok(parse_summary(&path, &src))
        }
        AlloySubcommand::Tokens { path } => {
            let src = read(&path)?;
            Ok(token_stream(&src))
        }
        AlloySubcommand::Version => Ok(version()),
    }
}

fn read(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {path:?}: {e}"))
}

/// Tracked upstream version banner.
pub fn version() -> String {
    "grafana/alloy v1.5.0 (cave-tracing in-process engine)\n".to_string()
}

/// Validates `src`, returning a human-readable report of diagnostics.
pub fn validate_source(name: &str, src: &str) -> String {
    let (file, diags) = parse_file(name, src);
    if diags.is_empty() {
        format!("ok: {} parsed cleanly ({} top-level statements)\n", name, file.body.len())
    } else {
        let mut out = format!("{}: {} diagnostic(s)\n", name, diags.len());
        for d in &diags {
            out.push_str(&format!(
                "  {}:{}:{}: {}\n",
                name, d.pos.line, d.pos.column, d.message
            ));
        }
        out
    }
}

/// Summarises the parsed structure: counts of blocks vs attributes at the top
/// level, and each block's `.`-delimited name + optional label.
pub fn parse_summary(name: &str, src: &str) -> String {
    use crate::alloy::ast::Stmt;
    let (file, _diags) = parse_file(name, src);
    let mut blocks = 0usize;
    let mut attrs = 0usize;
    let mut lines = String::new();
    for stmt in &file.body {
        match stmt {
            Stmt::Block(b) => {
                blocks += 1;
                if b.label.is_empty() {
                    lines.push_str(&format!("  block {}\n", b.block_name()));
                } else {
                    lines.push_str(&format!("  block {} {:?}\n", b.block_name(), b.label));
                }
            }
            Stmt::Attribute(a) => {
                attrs += 1;
                lines.push_str(&format!("  attr  {}\n", a.name.name));
            }
        }
    }
    format!("{name}: {blocks} block(s), {attrs} attribute(s)\n{lines}")
}

/// Renders the token stream of `src`, one `TOKEN literal` per line.
pub fn token_stream(src: &str) -> String {
    let mut s = Scanner::new(src);
    let mut out = String::new();
    loop {
        let (_pos, tok, lit) = s.scan();
        if tok == Token::Eof {
            out.push_str("EOF\n");
            break;
        }
        if lit.is_empty() || tok == Token::Terminator {
            out.push_str(&format!("{tok}\n"));
        } else {
            out.push_str(&format!("{tok} {lit:?}\n"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_reports_clean_parse() {
        let out = validate_source("ok.alloy", "prometheus.scrape \"d\" {\n  targets = []\n}\n");
        assert!(out.contains("ok:"), "{out}");
        assert!(out.contains("1 top-level"), "{out}");
    }

    #[test]
    fn validate_reports_diagnostics() {
        let out = validate_source("bad.alloy", "x =\n");
        assert!(out.contains("diagnostic"), "{out}");
    }

    #[test]
    fn parse_summary_counts_blocks_and_attrs() {
        let out = parse_summary("c.alloy", "a = 1\nprometheus.scrape \"d\" {\n  targets = []\n}\n");
        assert!(out.contains("1 block(s), 1 attribute(s)"), "{out}");
        assert!(out.contains("block prometheus.scrape \"d\""), "{out}");
    }

    #[test]
    fn token_stream_lists_tokens() {
        let out = token_stream("x = 1");
        assert!(out.contains("IDENT \"x\""), "{out}");
        assert!(out.contains("="), "{out}");
        assert!(out.ends_with("EOF\n"), "{out}");
    }

    #[test]
    fn version_mentions_alloy() {
        assert!(version().contains("grafana/alloy v1.5.0"));
    }
}
