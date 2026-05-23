// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Language-package detection.
//!
//! Mirrors trivy's `pkg/fanal/analyzer/language/*` for the subset cave-trivy
//! MVP supports: `package-lock.json` (npm), `yarn.lock` (yarn), `pnpm-lock.yaml`
//! (pnpm), `requirements.txt` (pip), `Pipfile.lock` (pipenv),
//! `Gemfile.lock` (rubygems), `go.mod` + `go.sum` (golang), `Cargo.lock`
//! (cargo), `composer.lock` (composer), `pom.xml` (maven, minimal), and
//! `pubspec.lock` (pub).

use crate::models::Package;

/// Detect by file basename → parser dispatch.
pub fn parse_lockfile(basename: &str, text: &str) -> Vec<Package> {
    match basename {
        "package-lock.json" => parse_package_lock(text),
        "yarn.lock" => parse_yarn_lock(text),
        "pnpm-lock.yaml" => parse_pnpm_lock(text),
        "requirements.txt" => parse_requirements_txt(text),
        "Pipfile.lock" => parse_pipfile_lock(text),
        "Gemfile.lock" => parse_gemfile_lock(text),
        "go.mod" => parse_go_mod(text),
        "go.sum" => parse_go_sum(text),
        "Cargo.lock" => parse_cargo_lock(text),
        "composer.lock" => parse_composer_lock(text),
        "pom.xml" => parse_pom_xml(text),
        "pubspec.lock" => parse_pubspec_lock(text),
        _ => Vec::new(),
    }
}

pub fn parse_package_lock(text: &str) -> Vec<Package> {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    if let Some(packages) = v.get("packages").and_then(|p| p.as_object()) {
        for (path, info) in packages {
            if path.is_empty() {
                continue;
            }
            let name = path
                .rsplit("node_modules/")
                .next()
                .unwrap_or(path)
                .to_string();
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !name.is_empty() && !version.is_empty() {
                out.push(Package::new(&name, &version, "npm"));
            }
        }
    } else if let Some(deps) = v.get("dependencies").and_then(|p| p.as_object()) {
        for (name, info) in deps {
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if !version.is_empty() {
                out.push(Package::new(name, &version, "npm"));
            }
        }
    }
    out
}

pub fn parse_yarn_lock(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut header: Option<String> = None;
    for line in text.lines() {
        let stripped = line.trim_end();
        if stripped.is_empty() || stripped.starts_with('#') {
            continue;
        }
        if !stripped.starts_with(' ') && stripped.ends_with(':') {
            let h = stripped.trim_end_matches(':').trim_matches('"').to_string();
            header = Some(h.split(',').next().unwrap_or(&h).to_string());
        } else if let Some(rest) = stripped.strip_prefix("  version ") {
            let version = rest.trim().trim_matches('"').to_string();
            if let Some(ref h) = header {
                let name = h.rsplit_once('@').map(|t| t.0).unwrap_or(h);
                out.push(Package::new(name, &version, "npm"));
            }
        }
    }
    out
}

pub fn parse_pnpm_lock(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("/") {
            if let Some(slash) = rest.find('/') {
                let name = &rest[..slash];
                let ver = &rest[slash + 1..];
                let v = ver.trim_end_matches(':').trim_matches('\'').to_string();
                out.push(Package::new(name, &v, "npm"));
            }
        }
    }
    out
}

pub fn parse_requirements_txt(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    for line in text.lines() {
        let l = line.split('#').next().unwrap_or("").trim();
        if l.is_empty() || l.starts_with("-r ") || l.starts_with("--") {
            continue;
        }
        if let Some((n, v)) = l.split_once("==") {
            out.push(Package::new(n.trim(), v.trim(), "pypi"));
        }
    }
    out
}

pub fn parse_pipfile_lock(text: &str) -> Vec<Package> {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for section in ["default", "develop"] {
        if let Some(obj) = v.get(section).and_then(|o| o.as_object()) {
            for (name, info) in obj {
                if let Some(ver) = info
                    .get("version")
                    .and_then(|v| v.as_str())
                    .and_then(|s| s.strip_prefix("=="))
                {
                    out.push(Package::new(name, ver, "pypi"));
                }
            }
        }
    }
    out
}

pub fn parse_gemfile_lock(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut in_specs = false;
    for line in text.lines() {
        let l = line.trim_end();
        if l.trim_start() == "specs:" {
            in_specs = true;
            continue;
        }
        if !in_specs {
            continue;
        }
        if !l.starts_with("    ") {
            in_specs = false;
            continue;
        }
        let body = l.trim_start();
        if !body.starts_with(char::is_alphanumeric) {
            continue;
        }
        if let Some((name, ver)) = body.split_once(' ') {
            let v = ver.trim_start_matches('(').trim_end_matches(')').to_string();
            out.push(Package::new(name, &v, "gem"));
        }
    }
    out
}

pub fn parse_go_mod(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut in_require = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("require (") {
            in_require = true;
            continue;
        }
        if l == ")" {
            in_require = false;
            continue;
        }
        let line_to_use = l.strip_prefix("require ").unwrap_or(l);
        if !in_require && !l.starts_with("require ") {
            continue;
        }
        if let Some((name, ver)) = line_to_use.split_once(' ') {
            let v = ver
                .split("//")
                .next()
                .unwrap_or(ver)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_string();
            if !v.is_empty() {
                out.push(Package::new(name, &v, "go"));
            }
        }
    }
    out
}

pub fn parse_go_sum(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        if let (Some(name), Some(ver)) = (it.next(), it.next()) {
            let v = ver.trim_end_matches("/go.mod").to_string();
            out.push(Package::new(name, &v, "go"));
        }
    }
    out
}

pub fn parse_cargo_lock(text: &str) -> Vec<Package> {
    let v: toml::Value = match toml::from_str(text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    if let Some(arr) = v.get("package").and_then(|p| p.as_array()) {
        for p in arr {
            let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let ver = p.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if !name.is_empty() && !ver.is_empty() {
                out.push(Package::new(name, ver, "cargo"));
            }
        }
    }
    out
}

pub fn parse_composer_lock(text: &str) -> Vec<Package> {
    let v: serde_json::Value = match serde_json::from_str(text) {
        Ok(v) => v,
        Err(_) => return vec![],
    };
    let mut out = Vec::new();
    for section in ["packages", "packages-dev"] {
        if let Some(arr) = v.get(section).and_then(|a| a.as_array()) {
            for p in arr {
                let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("");
                let ver = p.get("version").and_then(|v| v.as_str()).unwrap_or("");
                if !name.is_empty() && !ver.is_empty() {
                    out.push(Package::new(name, ver, "composer"));
                }
            }
        }
    }
    out
}

pub fn parse_pom_xml(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut group = String::new();
    let mut artifact = String::new();
    let mut version = String::new();
    let mut in_dep = false;
    for line in text.lines() {
        let l = line.trim();
        if l == "<dependency>" {
            in_dep = true;
            group.clear();
            artifact.clear();
            version.clear();
            continue;
        }
        if l == "</dependency>" {
            in_dep = false;
            if !artifact.is_empty() && !version.is_empty() {
                let name = if !group.is_empty() {
                    format!("{}:{}", group, artifact)
                } else {
                    artifact.clone()
                };
                out.push(Package::new(&name, &version, "maven"));
            }
            continue;
        }
        if !in_dep {
            continue;
        }
        let inner = |open: &str, close: &str| {
            if let Some(s) = l.strip_prefix(open) {
                if let Some(rest) = s.strip_suffix(close) {
                    return Some(rest.to_string());
                }
            }
            None
        };
        if let Some(v) = inner("<groupId>", "</groupId>") {
            group = v;
        } else if let Some(v) = inner("<artifactId>", "</artifactId>") {
            artifact = v;
        } else if let Some(v) = inner("<version>", "</version>") {
            version = v;
        }
    }
    out
}

pub fn parse_pubspec_lock(text: &str) -> Vec<Package> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        let trimmed = line.trim_end();
        if trimmed.starts_with("    version: ") {
            let v = trimmed.trim_start_matches("    version: ").trim_matches('"');
            if let Some(n) = &name {
                if !v.is_empty() {
                    out.push(Package::new(n, v, "pub"));
                }
            }
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("  ") {
            if !rest.starts_with(' ')
                && rest.ends_with(':')
                && !rest.contains(' ')
            {
                name = Some(rest.trim_end_matches(':').to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn npm_package_lock_v2() {
        let p = parse_package_lock(
            r#"{"packages":{"":{"name":"app"},"node_modules/lodash":{"version":"4.17.20"}}}"#,
        );
        assert_eq!(p.len(), 1);
        assert_eq!(p[0].name, "lodash");
        assert_eq!(p[0].version, "4.17.20");
    }

    #[test]
    fn npm_package_lock_v1() {
        let p = parse_package_lock(r#"{"dependencies":{"lodash":{"version":"4.17.20"}}}"#);
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn yarn_lock_simple() {
        let txt = "\"lodash@^4.0.0\":\n  version \"4.17.21\"\n  resolved \"x\"\n";
        let p = parse_yarn_lock(txt);
        assert_eq!(p[0].name, "lodash");
        assert_eq!(p[0].version, "4.17.21");
    }

    #[test]
    fn pnpm_lock() {
        let txt = "packages:\n  /lodash/4.17.21:\n    dev: false\n";
        let p = parse_pnpm_lock(txt);
        assert_eq!(p[0].name, "lodash");
        assert_eq!(p[0].version, "4.17.21");
    }

    #[test]
    fn pip_requirements() {
        let txt = "requests==2.31.0\n# comment\nflask==3.0.0\n--index https://example\n";
        let p = parse_requirements_txt(txt);
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].name, "flask");
    }

    #[test]
    fn pipfile_lock() {
        let p = parse_pipfile_lock(
            r#"{"default":{"requests":{"version":"==2.31.0"}}}"#,
        );
        assert_eq!(p[0].version, "2.31.0");
    }

    #[test]
    fn gemfile_lock_specs() {
        let txt = "GEM\n  specs:\n    actionpack (7.1.0)\n    nokogiri (1.16.0)\nBUNDLED WITH\n";
        let p = parse_gemfile_lock(txt);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "actionpack");
        assert_eq!(p[1].version, "1.16.0");
    }

    #[test]
    fn go_mod_block() {
        let txt = "module a\n\nrequire (\n\tgithub.com/x/y v1.2.3\n\tgithub.com/a/b v0.0.1 // indirect\n)\n";
        let p = parse_go_mod(txt);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].name, "github.com/x/y");
    }

    #[test]
    fn go_sum_dedup() {
        let txt = "github.com/x/y v1.2.3 h1:abc\ngithub.com/x/y v1.2.3/go.mod h1:def\n";
        let p = parse_go_sum(txt);
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].version, "v1.2.3");
        assert_eq!(p[1].version, "v1.2.3");
    }

    #[test]
    fn cargo_lock_packages() {
        let txt = r#"[[package]]
name = "serde"
version = "1.0.200"

[[package]]
name = "tokio"
version = "1.40.0"
"#;
        let p = parse_cargo_lock(txt);
        assert_eq!(p.len(), 2);
        assert_eq!(p[1].name, "tokio");
    }

    #[test]
    fn composer_lock() {
        let p = parse_composer_lock(
            r#"{"packages":[{"name":"symfony/http","version":"6.4.0"}]}"#,
        );
        assert_eq!(p[0].name, "symfony/http");
    }

    #[test]
    fn pom_xml_dep() {
        let txt = "<dependency>\n<groupId>org.spring</groupId>\n<artifactId>spring-core</artifactId>\n<version>6.1.5</version>\n</dependency>";
        let p = parse_pom_xml(txt);
        assert_eq!(p[0].name, "org.spring:spring-core");
        assert_eq!(p[0].version, "6.1.5");
    }

    #[test]
    fn pubspec_lock_basic() {
        let txt = "packages:\n  flutter:\n    version: \"3.16.0\"\n";
        let p = parse_pubspec_lock(txt);
        assert_eq!(p[0].name, "flutter");
        assert_eq!(p[0].version, "3.16.0");
    }

    #[test]
    fn dispatch_by_filename() {
        let p = parse_lockfile("Cargo.lock", r#"[[package]]
name = "x"
version = "1"
"#);
        assert_eq!(p[0].name, "x");
        let n = parse_lockfile("unknown.lock", "");
        assert!(n.is_empty());
    }
}
