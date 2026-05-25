// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Language-specific package scanner.
//!
//! Parses: go.sum, package-lock.json (npm), requirements.txt (pip),
//!         pom.xml (Maven), Cargo.lock (Cargo), composer.lock (PHP).

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Ecosystem {
    Go,
    Npm,
    Pip,
    Maven,
    Cargo,
    Composer,
}

impl std::fmt::Display for Ecosystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Ecosystem::Go => write!(f, "go"),
            Ecosystem::Npm => write!(f, "npm"),
            Ecosystem::Pip => write!(f, "pip"),
            Ecosystem::Maven => write!(f, "maven"),
            Ecosystem::Cargo => write!(f, "cargo"),
            Ecosystem::Composer => write!(f, "composer"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LangPackage {
    pub name: String,
    pub version: String,
    pub ecosystem: Ecosystem,
    pub indirect: bool,
    pub checksum: Option<String>,
    pub file_path: String,
}

// ---------------------------------------------------------------------------
// Go modules (go.sum)
// ---------------------------------------------------------------------------

/// Parse `go.sum` — each line: `module version hash`
pub fn parse_go_sum(content: &str, file_path: &str) -> Vec<LangPackage> {
    let mut pkgs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 3 {
            continue;
        }
        let name = parts[0];
        let ver = parts[1].trim_end_matches("/go.mod");
        if seen.contains(&(name.to_string(), ver.to_string())) {
            continue;
        }
        seen.insert((name.to_string(), ver.to_string()));
        pkgs.push(LangPackage {
            name: name.to_string(),
            version: ver.to_string(),
            ecosystem: Ecosystem::Go,
            indirect: false,
            checksum: Some(parts[2].to_string()),
            file_path: file_path.to_string(),
        });
    }
    pkgs
}

// ---------------------------------------------------------------------------
// npm (package-lock.json v2/v3)
// ---------------------------------------------------------------------------

/// Parse `package-lock.json`.
pub fn parse_package_lock_json(content: &str, file_path: &str) -> Vec<LangPackage> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        return vec![];
    };

    let mut pkgs = Vec::new();

    // lockfileVersion 2 / 3: "packages" key
    if let Some(packages) = v.get("packages").and_then(|p| p.as_object()) {
        for (path, info) in packages {
            if path.is_empty() {
                continue;
            } // root package
            let name = path
                .trim_start_matches("node_modules/")
                .trim_start_matches("../node_modules/");
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if version.is_empty() {
                continue;
            }
            let dev = info.get("dev").and_then(|v| v.as_bool()).unwrap_or(false);
            pkgs.push(LangPackage {
                name: name.to_string(),
                version,
                ecosystem: Ecosystem::Npm,
                indirect: dev,
                checksum: info
                    .get("integrity")
                    .and_then(|v| v.as_str())
                    .map(str::to_string),
                file_path: file_path.to_string(),
            });
        }
    // lockfileVersion 1: "dependencies" key
    } else if let Some(deps) = v.get("dependencies").and_then(|p| p.as_object()) {
        for (name, info) in deps {
            let version = info
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if version.is_empty() {
                continue;
            }
            pkgs.push(LangPackage {
                name: name.to_string(),
                version,
                ecosystem: Ecosystem::Npm,
                indirect: false,
                checksum: None,
                file_path: file_path.to_string(),
            });
        }
    }

    pkgs
}

// ---------------------------------------------------------------------------
// pip (requirements.txt)
// ---------------------------------------------------------------------------

/// Parse `requirements.txt` — supports `==`, `>=`, `~=` specifiers.
pub fn parse_requirements_txt(content: &str, file_path: &str) -> Vec<LangPackage> {
    let mut pkgs = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        // Strip inline comments
        let line = if let Some(pos) = line.find('#') {
            &line[..pos]
        } else {
            line
        };
        let line = line.trim();
        if line.is_empty() || line.starts_with('-') {
            continue;
        }

        // Find version specifier
        let (name, version) = if let Some(pos) = line.find("==") {
            (
                line[..pos].trim().to_string(),
                line[pos + 2..].trim().to_string(),
            )
        } else if let Some(pos) = line.find(">=") {
            (
                line[..pos].trim().to_string(),
                line[pos + 2..].trim().to_string(),
            )
        } else if let Some(pos) = line.find("~=") {
            (
                line[..pos].trim().to_string(),
                line[pos + 2..].trim().to_string(),
            )
        } else {
            (line.to_string(), String::new())
        };

        // Normalize extras: flask[async] → flask
        let name = name.split('[').next().unwrap_or(&name).trim().to_string();
        if name.is_empty() {
            continue;
        }

        pkgs.push(LangPackage {
            name,
            version,
            ecosystem: Ecosystem::Pip,
            indirect: false,
            checksum: None,
            file_path: file_path.to_string(),
        });
    }
    pkgs
}

// ---------------------------------------------------------------------------
// Maven (pom.xml) — simplified dependency extraction
// ---------------------------------------------------------------------------

/// Parse `pom.xml` dependencies (simplified: just groupId:artifactId:version).
pub fn parse_pom_xml(content: &str, file_path: &str) -> Vec<LangPackage> {
    let mut pkgs = Vec::new();

    // Very simplified XML scanning (no full XML parser to avoid extra deps)
    // Looks for <dependency> blocks with <groupId>, <artifactId>, <version>
    let deps: Vec<&str> = content.split("<dependency>").skip(1).collect();
    for dep in deps {
        let end = dep.find("</dependency>").unwrap_or(dep.len());
        let block = &dep[..end];
        let group = extract_xml_tag(block, "groupId").unwrap_or_default();
        let artifact = extract_xml_tag(block, "artifactId").unwrap_or_default();
        let version = extract_xml_tag(block, "version").unwrap_or_default();
        if group.is_empty() || artifact.is_empty() {
            continue;
        }
        let name = format!("{group}:{artifact}");
        pkgs.push(LangPackage {
            name,
            version,
            ecosystem: Ecosystem::Maven,
            indirect: false,
            checksum: None,
            file_path: file_path.to_string(),
        });
    }
    pkgs
}

fn extract_xml_tag<'a>(content: &'a str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = content.find(&open)? + open.len();
    let end = content.find(&close)?;
    if end > start {
        Some(content[start..end].trim().to_string())
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Cargo.lock
// ---------------------------------------------------------------------------

/// Parse `Cargo.lock` (TOML v3 format).
pub fn parse_cargo_lock(content: &str, file_path: &str) -> Vec<LangPackage> {
    let mut pkgs = Vec::new();

    // Split on [[package]] blocks
    for block in content.split("[[package]]").skip(1) {
        let name = extract_toml_value(block, "name").unwrap_or_default();
        let version = extract_toml_value(block, "version").unwrap_or_default();
        let checksum = extract_toml_value(block, "checksum");
        if name.is_empty() || version.is_empty() {
            continue;
        }
        pkgs.push(LangPackage {
            name: name.trim_matches('"').to_string(),
            version: version.trim_matches('"').to_string(),
            ecosystem: Ecosystem::Cargo,
            indirect: false,
            checksum: checksum.map(|s| s.trim_matches('"').to_string()),
            file_path: file_path.to_string(),
        });
    }
    pkgs
}

fn extract_toml_value(block: &str, key: &str) -> Option<String> {
    for line in block.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key) {
            if let Some(rest) = rest.trim_start().strip_prefix('=') {
                return Some(rest.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Composer (composer.lock)
// ---------------------------------------------------------------------------

/// Parse `composer.lock`.
pub fn parse_composer_lock(content: &str, file_path: &str) -> Vec<LangPackage> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(content) else {
        return vec![];
    };

    let mut pkgs = Vec::new();
    for key in &["packages", "packages-dev"] {
        if let Some(arr) = v.get(*key).and_then(|a| a.as_array()) {
            for pkg in arr {
                let name = pkg
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let version = pkg
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                if name.is_empty() {
                    continue;
                }
                pkgs.push(LangPackage {
                    name,
                    version,
                    ecosystem: Ecosystem::Composer,
                    indirect: *key == "packages-dev",
                    checksum: None,
                    file_path: file_path.to_string(),
                });
            }
        }
    }
    pkgs
}

// ---------------------------------------------------------------------------
// Manifest detection helper
// ---------------------------------------------------------------------------

/// Map a file name to the parser that should handle it.
pub fn detect_manifest_type(filename: &str) -> Option<Ecosystem> {
    let name = std::path::Path::new(filename)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or(filename);
    match name {
        "go.sum" => Some(Ecosystem::Go),
        "package-lock.json" | "yarn.lock" | "npm-shrinkwrap.json" => Some(Ecosystem::Npm),
        "requirements.txt" | "Pipfile.lock" => Some(Ecosystem::Pip),
        "pom.xml" => Some(Ecosystem::Maven),
        "Cargo.lock" => Some(Ecosystem::Cargo),
        "composer.lock" => Some(Ecosystem::Composer),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_go_sum_basic() {
        let content = "github.com/pkg/errors v0.9.1 h1:3NjVXXXX=\ngithub.com/pkg/errors v0.9.1/go.mod h1:YYYY=\n";
        let pkgs = parse_go_sum(content, "go.sum");
        // deduplicates go.mod entry
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "github.com/pkg/errors");
        assert_eq!(pkgs[0].version, "v0.9.1");
    }

    #[test]
    fn parse_requirements() {
        let content = "flask==2.3.0\nrequests>=2.28.0\n# comment\ndjango~=4.2.1\n";
        let pkgs = parse_requirements_txt(content, "requirements.txt");
        assert_eq!(pkgs.len(), 3);
        assert_eq!(pkgs[0].version, "2.3.0");
    }

    #[test]
    fn parse_cargo_lock_basic() {
        let content = r#"
[[package]]
name = "serde"
version = "1.0.193"
checksum = "abcdef"

[[package]]
name = "tokio"
version = "1.35.0"
"#;
        let pkgs = parse_cargo_lock(content, "Cargo.lock");
        assert_eq!(pkgs.len(), 2);
        assert_eq!(pkgs[0].name, "serde");
    }

    #[test]
    fn parse_pom_xml_basic() {
        let content = r#"<project>
  <dependencies>
    <dependency>
      <groupId>org.springframework</groupId>
      <artifactId>spring-core</artifactId>
      <version>5.3.18</version>
    </dependency>
  </dependencies>
</project>"#;
        let pkgs = parse_pom_xml(content, "pom.xml");
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "org.springframework:spring-core");
        assert_eq!(pkgs[0].version, "5.3.18");
    }

    #[test]
    fn manifest_detection() {
        assert_eq!(detect_manifest_type("go.sum"), Some(Ecosystem::Go));
        assert_eq!(detect_manifest_type("Cargo.lock"), Some(Ecosystem::Cargo));
        assert_eq!(
            detect_manifest_type("requirements.txt"),
            Some(Ecosystem::Pip)
        );
        assert_eq!(detect_manifest_type("pom.xml"), Some(Ecosystem::Maven));
        assert_eq!(
            detect_manifest_type("package-lock.json"),
            Some(Ecosystem::Npm)
        );
        assert_eq!(
            detect_manifest_type("composer.lock"),
            Some(Ecosystem::Composer)
        );
        assert_eq!(detect_manifest_type("Makefile"), None);
    }
}
