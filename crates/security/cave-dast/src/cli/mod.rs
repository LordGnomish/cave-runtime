// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: Galkan/zap-cli@v0.10.0  (Apache-2.0 community CLI for ZAP)
//
//! `zap-cli` compatibility surface. Parses the subcommand syntax of the
//! community ZAP CLI so the same Bash automation can drive cave-dast.
//! Supported subcommands:
//!
//! * `cave-dast quick-scan <url> [--spider] [--ajax] [--active]`
//! * `cave-dast baseline -t <url> [--minutes N] [--report file.html]`
//! * `cave-dast report -o file.html`
//! * `cave-dast status`

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CliCommand {
    QuickScan(QuickScanArgs),
    Baseline(BaselineArgs),
    Report(ReportArgs),
    Status,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct QuickScanArgs {
    pub target: String,
    pub spider: bool,
    pub ajax: bool,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BaselineArgs {
    pub target: String,
    pub minutes: u32,
    pub report: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ReportArgs {
    pub output: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("no subcommand provided")]
    Empty,
    #[error("unknown subcommand: {0}")]
    Unknown(String),
    #[error("missing required argument: {0}")]
    MissingArg(&'static str),
    #[error("invalid value for {0}: {1}")]
    BadValue(&'static str, String),
}

pub fn parse(args: &[&str]) -> Result<CliCommand, CliError> {
    let (sub, rest) = args.split_first().ok_or(CliError::Empty)?;
    match *sub {
        "quick-scan" => parse_quick_scan(rest).map(CliCommand::QuickScan),
        "baseline" => parse_baseline(rest).map(CliCommand::Baseline),
        "report" => parse_report(rest).map(CliCommand::Report),
        "status" => Ok(CliCommand::Status),
        other => Err(CliError::Unknown(other.to_string())),
    }
}

fn parse_quick_scan(rest: &[&str]) -> Result<QuickScanArgs, CliError> {
    let mut q = QuickScanArgs::default();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "--spider" => q.spider = true,
            "--ajax" => q.ajax = true,
            "--active" => q.active = true,
            flag if flag.starts_with("--") => {
                return Err(CliError::Unknown(flag.to_string()));
            }
            target if q.target.is_empty() => q.target = target.to_string(),
            unexpected => return Err(CliError::Unknown(unexpected.to_string())),
        }
        i += 1;
    }
    if q.target.is_empty() {
        return Err(CliError::MissingArg("target"));
    }
    Ok(q)
}

fn parse_baseline(rest: &[&str]) -> Result<BaselineArgs, CliError> {
    let mut b = BaselineArgs::default();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "-t" | "--target" => {
                i += 1;
                let v = rest.get(i).ok_or(CliError::MissingArg("target"))?;
                b.target = (*v).to_string();
            }
            "--minutes" => {
                i += 1;
                let v = rest.get(i).ok_or(CliError::MissingArg("minutes"))?;
                b.minutes = v
                    .parse()
                    .map_err(|_| CliError::BadValue("minutes", v.to_string()))?;
            }
            "--report" => {
                i += 1;
                let v = rest.get(i).ok_or(CliError::MissingArg("report"))?;
                b.report = Some((*v).to_string());
            }
            other => return Err(CliError::Unknown(other.to_string())),
        }
        i += 1;
    }
    if b.target.is_empty() {
        return Err(CliError::MissingArg("target"));
    }
    Ok(b)
}

fn parse_report(rest: &[&str]) -> Result<ReportArgs, CliError> {
    let mut r = ReportArgs::default();
    let mut i = 0;
    while i < rest.len() {
        match rest[i] {
            "-o" | "--output" => {
                i += 1;
                let v = rest.get(i).ok_or(CliError::MissingArg("output"))?;
                r.output = (*v).to_string();
            }
            other => return Err(CliError::Unknown(other.to_string())),
        }
        i += 1;
    }
    if r.output.is_empty() {
        return Err(CliError::MissingArg("output"));
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quick_scan_minimal() {
        let cmd = parse(&["quick-scan", "http://x.test/"]).unwrap();
        match cmd {
            CliCommand::QuickScan(q) => {
                assert_eq!(q.target, "http://x.test/");
                assert!(!q.spider && !q.ajax && !q.active);
            }
            _ => panic!("expected QuickScan"),
        }
    }

    #[test]
    fn quick_scan_all_flags() {
        let cmd = parse(&["quick-scan", "http://x/", "--spider", "--ajax", "--active"]).unwrap();
        match cmd {
            CliCommand::QuickScan(q) => assert!(q.spider && q.ajax && q.active),
            _ => panic!(),
        }
    }

    #[test]
    fn quick_scan_missing_target() {
        let err = parse(&["quick-scan", "--spider"]).unwrap_err();
        assert!(matches!(err, CliError::MissingArg("target")));
    }

    #[test]
    fn baseline_full_args() {
        let cmd = parse(&[
            "baseline",
            "-t",
            "http://x/",
            "--minutes",
            "5",
            "--report",
            "out.html",
        ])
        .unwrap();
        match cmd {
            CliCommand::Baseline(b) => {
                assert_eq!(b.target, "http://x/");
                assert_eq!(b.minutes, 5);
                assert_eq!(b.report.as_deref(), Some("out.html"));
            }
            _ => panic!(),
        }
    }

    #[test]
    fn baseline_bad_minutes() {
        let err = parse(&["baseline", "-t", "http://x/", "--minutes", "ten"]).unwrap_err();
        assert!(matches!(err, CliError::BadValue("minutes", _)));
    }

    #[test]
    fn report_requires_output() {
        let err = parse(&["report"]).unwrap_err();
        assert!(matches!(err, CliError::MissingArg("output")));
    }

    #[test]
    fn report_dash_o() {
        let cmd = parse(&["report", "-o", "r.html"]).unwrap();
        match cmd {
            CliCommand::Report(r) => assert_eq!(r.output, "r.html"),
            _ => panic!(),
        }
    }

    #[test]
    fn status_no_args() {
        assert_eq!(parse(&["status"]).unwrap(), CliCommand::Status);
    }

    #[test]
    fn empty_args_errors() {
        assert!(matches!(parse(&[]), Err(CliError::Empty)));
    }

    #[test]
    fn unknown_subcommand() {
        assert!(matches!(parse(&["fly"]), Err(CliError::Unknown(_))));
    }
}
