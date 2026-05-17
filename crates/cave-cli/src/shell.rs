// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Shell completion script generation.
//!
//! Per ADR-RUNTIME-CLI-CONSOLIDATION-001 M6: ship completions for
//! bash, zsh, fish, and PowerShell from a single source. We generate
//! the scripts in-house (rather than pulling `clap_complete`) so the
//! verb list stays a pure data structure that's trivially testable.

use anyhow::{bail, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shell {
    Bash,
    Zsh,
    Fish,
    PowerShell,
}

impl Shell {
    pub fn parse(s: &str) -> Result<Self> {
        match s.to_lowercase().as_str() {
            "bash" => Ok(Shell::Bash),
            "zsh" => Ok(Shell::Zsh),
            "fish" => Ok(Shell::Fish),
            "powershell" | "pwsh" | "ps" => Ok(Shell::PowerShell),
            _ => bail!("unknown shell `{}`", s),
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Shell::Bash => ".bash",
            Shell::Zsh => ".zsh",
            Shell::Fish => ".fish",
            Shell::PowerShell => ".ps1",
        }
    }
}

/// Top-level subcommand verbs that completion offers. Keep alphabetical;
/// shells that paginate (zsh) benefit, and tests can do an exact-list
/// snapshot.
pub const TOP_VERBS: &[&str] = &[
    "argocd", "chaos", "completion", "deploy", "describe", "events", "flag", "get", "harbor",
    "helm", "kubectl", "logs", "secrets", "tenant", "topology", "tui", "vault",
];

/// Generate the completion script for `shell` and the binary `name`.
pub fn generate(shell: Shell, name: &str) -> Result<String> {
    if name.is_empty() {
        bail!("binary name required");
    }
    if name.contains(char::is_whitespace) {
        bail!("binary name may not contain whitespace");
    }
    Ok(match shell {
        Shell::Bash => bash(name),
        Shell::Zsh => zsh(name),
        Shell::Fish => fish(name),
        Shell::PowerShell => pwsh(name),
    })
}

fn verbs_space() -> String {
    TOP_VERBS.join(" ")
}

fn bash(name: &str) -> String {
    format!(
        "_{name}_complete() {{\n\
         \tlocal cur prev opts\n\
         \tCOMPREPLY=()\n\
         \tcur=\"${{COMP_WORDS[COMP_CWORD]}}\"\n\
         \topts=\"{verbs}\"\n\
         \tif [[ ${{COMP_CWORD}} -eq 1 ]]; then\n\
         \t\tCOMPREPLY=( $(compgen -W \"${{opts}}\" -- \"${{cur}}\") )\n\
         \t\treturn 0\n\
         \tfi\n\
         }}\n\
         complete -F _{name}_complete {name}\n",
        name = name,
        verbs = verbs_space(),
    )
}

fn zsh(name: &str) -> String {
    let body: String = TOP_VERBS
        .iter()
        .map(|v| format!("\t\t\t'{}:cavectl {} subcommand'\\\n", v, v))
        .collect();
    format!(
        "#compdef {name}\n\
         _{name}() {{\n\
         \tlocal -a verbs\n\
         \tverbs=(\n\
         {body}\
         \t)\n\
         \t_describe 'verb' verbs\n\
         }}\n\
         compdef _{name} {name}\n",
        name = name,
        body = body,
    )
}

fn fish(name: &str) -> String {
    let mut out = String::new();
    for v in TOP_VERBS {
        out.push_str(&format!(
            "complete -c {name} -n '__fish_use_subcommand' -a '{v}' -d 'cavectl {v} subcommand'\n",
            name = name,
            v = v,
        ));
    }
    out
}

fn pwsh(name: &str) -> String {
    let body: String = TOP_VERBS
        .iter()
        .map(|v| format!("        '{}'", v))
        .collect::<Vec<_>>()
        .join(",\n");
    format!(
        "Register-ArgumentCompleter -CommandName {name} -ScriptBlock {{\n\
         \tparam($wordToComplete, $commandAst, $cursorPosition)\n\
         \t@(\n\
         {body}\n\
         \t) | Where-Object {{ $_ -like \"$wordToComplete*\" }}\n\
         }}\n",
        name = name,
        body = body,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_known_shells() {
        assert_eq!(Shell::parse("bash").unwrap(), Shell::Bash);
        assert_eq!(Shell::parse("zsh").unwrap(), Shell::Zsh);
        assert_eq!(Shell::parse("fish").unwrap(), Shell::Fish);
        assert_eq!(Shell::parse("powershell").unwrap(), Shell::PowerShell);
        assert_eq!(Shell::parse("pwsh").unwrap(), Shell::PowerShell);
    }

    #[test]
    fn parse_case_insensitive() {
        assert_eq!(Shell::parse("BASH").unwrap(), Shell::Bash);
    }

    #[test]
    fn parse_rejects_unknown() {
        assert!(Shell::parse("tcsh").is_err());
    }

    #[test]
    fn extensions() {
        assert_eq!(Shell::Bash.extension(), ".bash");
        assert_eq!(Shell::Zsh.extension(), ".zsh");
        assert_eq!(Shell::Fish.extension(), ".fish");
        assert_eq!(Shell::PowerShell.extension(), ".ps1");
    }

    #[test]
    fn bash_contains_function_name() {
        let s = generate(Shell::Bash, "cavectl").unwrap();
        assert!(s.contains("_cavectl_complete"));
        assert!(s.contains("complete -F _cavectl_complete cavectl"));
    }

    #[test]
    fn bash_contains_all_verbs() {
        let s = generate(Shell::Bash, "cavectl").unwrap();
        for v in TOP_VERBS {
            assert!(s.contains(v), "bash script should contain `{}`", v);
        }
    }

    #[test]
    fn zsh_has_compdef() {
        let s = generate(Shell::Zsh, "cavectl").unwrap();
        assert!(s.starts_with("#compdef cavectl"));
        assert!(s.contains("compdef _cavectl cavectl"));
    }

    #[test]
    fn zsh_lists_all_verbs() {
        let s = generate(Shell::Zsh, "cavectl").unwrap();
        for v in TOP_VERBS {
            assert!(s.contains(v));
        }
    }

    #[test]
    fn fish_uses_complete_per_verb() {
        let s = generate(Shell::Fish, "cavectl").unwrap();
        for v in TOP_VERBS {
            let line = format!(
                "complete -c cavectl -n '__fish_use_subcommand' -a '{}'",
                v
            );
            assert!(s.contains(&line), "fish line missing for {}", v);
        }
    }

    #[test]
    fn pwsh_register_argument_completer() {
        let s = generate(Shell::PowerShell, "cavectl").unwrap();
        assert!(s.contains("Register-ArgumentCompleter"));
        assert!(s.contains("-CommandName cavectl"));
    }

    #[test]
    fn pwsh_includes_each_verb() {
        let s = generate(Shell::PowerShell, "cavectl").unwrap();
        for v in TOP_VERBS {
            assert!(s.contains(v));
        }
    }

    #[test]
    fn generate_rejects_empty_name() {
        assert!(generate(Shell::Bash, "").is_err());
    }

    #[test]
    fn generate_rejects_whitespace_name() {
        assert!(generate(Shell::Bash, "cave ctl").is_err());
    }

    #[test]
    fn top_verbs_alphabetical() {
        let mut sorted = TOP_VERBS.to_vec();
        sorted.sort();
        let original: Vec<&'static str> = TOP_VERBS.to_vec();
        assert_eq!(sorted, original);
    }

    #[test]
    fn top_verbs_no_duplicates() {
        let mut s = std::collections::HashSet::new();
        for v in TOP_VERBS {
            assert!(s.insert(*v), "duplicate verb {}", v);
        }
    }

    #[test]
    fn top_verbs_includes_native_and_compat() {
        let v: std::collections::HashSet<&'static str> = TOP_VERBS.iter().copied().collect();
        // Native
        assert!(v.contains("deploy"));
        assert!(v.contains("get"));
        assert!(v.contains("logs"));
        // Compat
        assert!(v.contains("kubectl"));
        assert!(v.contains("helm"));
        assert!(v.contains("argocd"));
        assert!(v.contains("vault"));
        assert!(v.contains("harbor"));
        // M5/M6
        assert!(v.contains("tui"));
        assert!(v.contains("completion"));
    }

    #[test]
    fn parse_pwsh_alias() {
        assert_eq!(Shell::parse("ps").unwrap(), Shell::PowerShell);
    }

    #[test]
    fn bash_uses_compreply() {
        let s = generate(Shell::Bash, "cavectl").unwrap();
        assert!(s.contains("COMPREPLY"));
    }

    #[test]
    fn fish_includes_descriptions() {
        let s = generate(Shell::Fish, "cavectl").unwrap();
        assert!(s.contains("subcommand"));
    }

    #[test]
    fn shell_equality() {
        assert_eq!(Shell::Bash, Shell::Bash);
        assert_ne!(Shell::Bash, Shell::Zsh);
    }
}
