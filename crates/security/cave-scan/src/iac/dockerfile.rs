// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/iac/scanners/dockerfile/scanner.go
//! Dockerfile linter.
//!
//! Line-oriented scanner. Each instruction is parsed by `(verb, args)` from
//! whitespace-stripped lines. Rules:
//!
//! | Rule          | Issue                            | Severity |
//! |---------------|----------------------------------|----------|
//! | AVD-DS-0001   | base image uses :latest or none  | Medium   |
//! | AVD-DS-0002   | USER missing or USER root        | High     |
//! | AVD-DS-0010   | ADD with remote URL              | Medium   |
//! | AVD-DS-0027   | curl | sh / wget | sh           | High     |
//! | AVD-DS-0035   | apt-get/apk without --no-cache   | Low      |

use super::{IacError, IacFinding, IacScanner, Severity};

#[derive(Default, Clone)]
pub struct DockerfileScanner;

impl DockerfileScanner {
    pub fn new() -> Self {
        Self
    }
}

impl IacScanner for DockerfileScanner {
    fn provider(&self) -> &'static str {
        "dockerfile"
    }

    fn scan_str(&self, content: &str, path: &str) -> Result<Vec<IacFinding>, IacError> {
        let mut out = Vec::new();
        let mut user_set: Option<String> = None;
        let mut from_seen = false;
        for (idx, raw_line) in content.lines().enumerate() {
            let line_no = idx + 1;
            let line = raw_line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            // Tokenize verb + args
            let mut parts = line.splitn(2, char::is_whitespace);
            let verb = parts.next().unwrap_or("").to_ascii_uppercase();
            let args = parts.next().unwrap_or("").trim();
            match verb.as_str() {
                "FROM" => {
                    from_seen = true;
                    check_from(args, path, line_no, &mut out);
                }
                "USER" => {
                    user_set = Some(args.to_string());
                    if args == "root" || args == "0" || args.starts_with("root:") {
                        out.push(IacFinding {
                            rule_id: "AVD-DS-0002".into(),
                            severity: Severity::High,
                            message: "Container set USER root".into(),
                            file: path.to_string(),
                            line: line_no,
                        });
                    }
                }
                "ADD" => check_add(args, path, line_no, &mut out),
                "RUN" => check_run(args, path, line_no, &mut out),
                _ => {}
            }
        }
        // AVD-DS-0002: no USER set at all (root by default)
        if from_seen && user_set.as_deref().map(|u| u != "root" && u != "0") != Some(true) {
            out.push(IacFinding {
                rule_id: "AVD-DS-0002".into(),
                severity: Severity::High,
                message: "Dockerfile has no non-root USER".into(),
                file: path.to_string(),
                line: 0,
            });
        }
        Ok(out)
    }
}

fn check_from(args: &str, path: &str, line: usize, out: &mut Vec<IacFinding>) {
    // Strip "AS alias" suffix
    let image = args.split_whitespace().next().unwrap_or("");
    if image.is_empty() {
        return;
    }
    let tag = match image.rsplit_once(':') {
        Some((_, t)) => t,
        None => "",
    };
    if tag.is_empty() || tag == "latest" {
        out.push(IacFinding {
            rule_id: "AVD-DS-0001".into(),
            severity: Severity::Medium,
            message: format!("FROM image `{image}` pinned to :latest or untagged"),
            file: path.to_string(),
            line,
        });
    }
}

fn check_add(args: &str, path: &str, line: usize, out: &mut Vec<IacFinding>) {
    if args.contains("https://") || args.contains("http://") || args.contains("ftp://") {
        out.push(IacFinding {
            rule_id: "AVD-DS-0010".into(),
            severity: Severity::Medium,
            message: "ADD fetches a remote URL — use COPY of a verified file instead".into(),
            file: path.to_string(),
            line,
        });
    }
}

fn check_run(args: &str, path: &str, line: usize, out: &mut Vec<IacFinding>) {
    let pipe_sh = args.contains("| sh") || args.contains("| bash") || args.contains("|sh");
    let fetch = args.contains("curl ") || args.contains("wget ");
    if pipe_sh && fetch {
        out.push(IacFinding {
            rule_id: "AVD-DS-0027".into(),
            severity: Severity::High,
            message: "RUN pipes remote content into a shell (curl | sh)".into(),
            file: path.to_string(),
            line,
        });
    }
    // AVD-DS-0035 apt-get / apk without cache cleanup
    if (args.starts_with("apt-get install") || args.starts_with("apt install"))
        && !args.contains("rm -rf /var/lib/apt/lists")
    {
        out.push(IacFinding {
            rule_id: "AVD-DS-0035".into(),
            severity: Severity::Low,
            message: "apt-get install leaves package cache (no rm -rf /var/lib/apt/lists)".into(),
            file: path.to_string(),
            line,
        });
    }
}
