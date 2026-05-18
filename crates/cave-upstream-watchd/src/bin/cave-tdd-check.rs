// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cave-tdd-check` — Charter v2 TDD-strict gate CLI.
//!
//! Standalone driver around the [`cave_upstream_watchd::tdd`] analyzer.
//! Walks a branch's commits, classifies each, and reports the four TDD
//! signals (test_first / red / green / no_ignore). Exits 0 on pass, 1 on
//! any TDD failure, 2 on internal error.
//!
//! CI usage (see `.github/workflows/parity-tdd.yml`):
//!
//!   cave-tdd-check --base origin/main --branch HEAD --tests-pass=true --json
//!
//! Local pre-push usage:
//!
//!   cave-tdd-check
//!
//! The composite Charter v2 gate (parity_ratio_delta + cargo +
//! workspace-stub count) lives in
//! [`cave_upstream_watchd::auto_port_gate::CharterV2Gate`]; this CLI is
//! the lighter-weight TDD-only signal that runs on every PR without
//! requiring a baseline snapshot.

use std::path::PathBuf;
use std::process::ExitCode;

use clap::Parser;
use serde::Serialize;

use cave_upstream_watchd::tdd::{
    analyze_tdd_compliance, scan_stubs, CommitKind, ShellGitInspector,
};

#[derive(Parser, Debug)]
#[command(
    name = "cave-tdd-check",
    about = "Charter v2 TDD-strict gate. Exits 0 on pass, 1 on fail, 2 on error."
)]
struct Args {
    /// Repository root (default: current dir).
    #[arg(long, default_value = ".")]
    repo: PathBuf,

    /// Base ref (main / origin/main / SHA).
    #[arg(long, default_value = "main")]
    base: String,

    /// Branch ref (default: HEAD).
    #[arg(long, default_value = "HEAD")]
    branch: String,

    /// External signal: did `cargo test` pass on branch tip?
    /// CI fills this in after running the test suite. Default true so the
    /// CLI can be run as a lint-only check.
    #[arg(long, default_value_t = true)]
    tests_pass: bool,

    /// Emit JSON report to stdout instead of human text.
    #[arg(long)]
    json: bool,

    /// Also scan for `todo!()` / `unimplemented!()` / `unreachable!()`
    /// stubs in impl files touched by the branch. A non-zero count fails
    /// the gate even when TDD compliance is otherwise green.
    #[arg(long)]
    check_stubs: bool,
}

#[derive(Serialize)]
struct Report<'a> {
    pass: bool,
    tests_pass: bool,
    stub_count: u32,
    tdd: TddJson<'a>,
}

#[derive(Serialize)]
struct TddJson<'a> {
    test_first: bool,
    red_proof: bool,
    green_proof: bool,
    no_skip_attribute: bool,
    violation_count: usize,
    commits: Vec<CommitJson<'a>>,
}

#[derive(Serialize)]
struct CommitJson<'a> {
    sha: &'a str,
    subject: &'a str,
    kind: &'a str,
    modules: &'a [String],
}

fn main() -> ExitCode {
    let args = Args::parse();
    let inspector = ShellGitInspector::new(&args.repo);

    let tdd =
        match analyze_tdd_compliance(&inspector, &args.base, &args.branch, args.tests_pass) {
            Ok(t) => t,
            Err(e) => {
                eprintln!("cave-tdd-check: tdd analyzer error: {e}");
                return ExitCode::from(2);
            }
        };

    let stub_count = if args.check_stubs {
        match scan_stubs(&inspector, &args.base, &args.branch) {
            Ok(s) => s.len() as u32,
            Err(e) => {
                eprintln!("cave-tdd-check: stub scan error: {e}");
                return ExitCode::from(2);
            }
        }
    } else {
        0
    };

    let pass = tdd.is_pass() && stub_count == 0;

    if args.json {
        let commits: Vec<CommitJson> = tdd
            .details
            .commits
            .iter()
            .map(|c| CommitJson {
                sha: &c.sha,
                subject: &c.subject,
                kind: kind_label(c.kind),
                modules: &c.touched_modules,
            })
            .collect();
        let report = Report {
            pass,
            tests_pass: tdd.green_proof,
            stub_count,
            tdd: TddJson {
                test_first: tdd.test_first,
                red_proof: tdd.red_proof,
                green_proof: tdd.green_proof,
                no_skip_attribute: tdd.no_skip_attribute,
                violation_count: tdd.details.violations.len(),
                commits,
            },
        };
        println!("{}", serde_json::to_string_pretty(&report).unwrap());
    } else {
        eprintln!("Charter v2 TDD gate — {}..{}", args.base, args.branch);
        eprintln!("  {}", tdd.summary());
        if args.check_stubs {
            eprintln!(
                "  stubs in changed impl files: {}{}",
                stub_count,
                if stub_count == 0 { "" } else { " (FAIL)" }
            );
        }
        for v in &tdd.details.violations {
            eprintln!("  ! {v:?}");
        }
        eprintln!("  verdict: {}", if pass { "PASS" } else { "FAIL" });
    }

    if pass {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn kind_label(k: CommitKind) -> &'static str {
    match k {
        CommitKind::TestOnly => "test_only",
        CommitKind::ImplOnly => "impl_only",
        CommitKind::Mixed => "mixed",
        CommitKind::NonCode => "non_code",
    }
}
