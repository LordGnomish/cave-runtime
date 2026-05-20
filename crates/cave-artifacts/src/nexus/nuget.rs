// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: sonatype/nexus-public plugins/nexus-repository-nuget
//
//! .NET NuGet adapter — parse `.nuspec` metadata and recognise NuGet V3
//! repository paths.
//!
//! NuGet V3 protocol exposes `index.json` as the entry point and addresses
//! packages as `<id>/<version>/<id>.<version>.nupkg`. We expose the path
//! parser plus a lightweight XML reader for the `<metadata>` block of
//! `.nuspec` files (XML in the package manifest).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NuGetPackage {
    pub id: String,
    pub version: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub authors: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub license: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub require_license_acceptance: bool,
    #[serde(default)]
    pub dependencies: Vec<NuGetDependency>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NuGetDependency {
    pub id: String,
    pub version_range: String,
    #[serde(default)]
    pub target_framework: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum NuGetError {
    #[error("invalid nuspec: {0}")]
    InvalidNuspec(String),
    #[error("missing element `{0}`")]
    MissingElement(&'static str),
    #[error("invalid path: {0}")]
    InvalidPath(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NuGetPath {
    /// `/index.json` — service discovery document.
    Index,
    /// `/v3-flatcontainer/<id>/index.json` — package metadata index.
    PackageIndex { id: String },
    /// `/v3-flatcontainer/<id>/<version>/<id>.<version>.nupkg` — content fetch.
    Package { id: String, version: String },
    /// `/v3-flatcontainer/<id>/<version>/<id>.nuspec` — manifest fetch.
    Nuspec { id: String, version: String },
}

pub fn parse_path(path: &str) -> Result<NuGetPath, NuGetError> {
    let trimmed = path.trim_start_matches('/');
    if trimmed == "index.json" {
        return Ok(NuGetPath::Index);
    }
    let rest = trimmed
        .strip_prefix("v3-flatcontainer/")
        .ok_or_else(|| NuGetError::InvalidPath(path.to_string()))?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        [id, "index.json"] => Ok(NuGetPath::PackageIndex {
            id: id.to_ascii_lowercase(),
        }),
        [id, version, filename] => {
            let id_lower = id.to_ascii_lowercase();
            let expected_pkg = format!("{}.{}.nupkg", id_lower, version);
            let expected_nuspec = format!("{}.nuspec", id_lower);
            if filename.eq_ignore_ascii_case(&expected_pkg) {
                Ok(NuGetPath::Package {
                    id: id_lower,
                    version: version.to_string(),
                })
            } else if filename.eq_ignore_ascii_case(&expected_nuspec) {
                Ok(NuGetPath::Nuspec {
                    id: id_lower,
                    version: version.to_string(),
                })
            } else {
                Err(NuGetError::InvalidPath(path.to_string()))
            }
        }
        _ => Err(NuGetError::InvalidPath(path.to_string())),
    }
}

/// Lightweight `.nuspec` reader. Looks for the elements NuGet's reference
/// reader treats as required (`id`, `version`) plus the common optional
/// metadata. Uses a hand-written scanner so we don't pull a full XML
/// dependency for what's effectively grep-by-tag.
pub fn parse_nuspec(xml: &str) -> Result<NuGetPackage, NuGetError> {
    let id = first_tag(xml, "id").ok_or(NuGetError::MissingElement("id"))?;
    let version = first_tag(xml, "version").ok_or(NuGetError::MissingElement("version"))?;
    if !validate_package_id(&id) {
        return Err(NuGetError::InvalidNuspec(format!("invalid id `{}`", id)));
    }
    let title = first_tag(xml, "title");
    let description = first_tag(xml, "description");
    let project_url = first_tag(xml, "projectUrl");
    let license = first_tag(xml, "license").or_else(|| first_tag(xml, "licenseUrl"));
    let authors = first_tag(xml, "authors")
        .map(|s| {
            s.split(',')
                .map(|p| p.trim().to_string())
                .filter(|p| !p.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let tags = first_tag(xml, "tags")
        .map(|s| {
            s.split_whitespace().map(String::from).collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let require_license_acceptance = first_tag(xml, "requireLicenseAcceptance")
        .map(|s| s.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let dependencies = parse_dependencies(xml);
    Ok(NuGetPackage {
        id,
        version,
        title,
        authors,
        description,
        project_url,
        license,
        tags,
        require_license_acceptance,
        dependencies,
    })
}

fn first_tag(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{}>", tag);
    let close = format!("</{}>", tag);
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml[start..end].trim().to_string())
}

fn parse_dependencies(xml: &str) -> Vec<NuGetDependency> {
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(idx) = rest.find("<dependency ") {
        let tail = &rest[idx..];
        let end = match tail.find("/>") {
            Some(e) => e,
            None => break,
        };
        let attr_slice = &tail[..end];
        let id = attr_value(attr_slice, "id");
        let version = attr_value(attr_slice, "version");
        let target_framework = attr_value(attr_slice, "targetFramework");
        if let (Some(id), Some(version)) = (id, version) {
            out.push(NuGetDependency {
                id,
                version_range: version,
                target_framework,
            });
        }
        rest = &tail[end + 2..];
    }
    out
}

fn attr_value(s: &str, name: &str) -> Option<String> {
    let needle = format!("{}=\"", name);
    let start = s.find(&needle)? + needle.len();
    let end = s[start..].find('"')? + start;
    Some(s[start..end].to_string())
}

/// NuGet IDs are case-insensitive, start with letter/digit, and contain only
/// `[A-Za-z0-9._-]`. Length is enforced ≤ 100 chars by upstream.
pub fn validate_package_id(id: &str) -> bool {
    if id.is_empty() || id.len() > 100 {
        return false;
    }
    let first = id.bytes().next().unwrap();
    if !first.is_ascii_alphanumeric() {
        return false;
    }
    id.bytes()
        .all(|b| matches!(b, b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_path_index() {
        assert_eq!(parse_path("/index.json").unwrap(), NuGetPath::Index);
    }

    #[test]
    fn parse_path_package_index() {
        let p = parse_path("/v3-flatcontainer/Newtonsoft.Json/index.json").unwrap();
        assert_eq!(
            p,
            NuGetPath::PackageIndex {
                id: "newtonsoft.json".into()
            }
        );
    }

    #[test]
    fn parse_path_nupkg() {
        let p = parse_path("/v3-flatcontainer/Newtonsoft.Json/13.0.1/Newtonsoft.Json.13.0.1.nupkg")
            .unwrap();
        assert_eq!(
            p,
            NuGetPath::Package {
                id: "newtonsoft.json".into(),
                version: "13.0.1".into(),
            }
        );
    }

    #[test]
    fn parse_path_nuspec() {
        let p = parse_path("/v3-flatcontainer/Newtonsoft.Json/13.0.1/Newtonsoft.Json.nuspec")
            .unwrap();
        assert_eq!(
            p,
            NuGetPath::Nuspec {
                id: "newtonsoft.json".into(),
                version: "13.0.1".into(),
            }
        );
    }

    #[test]
    fn parse_path_invalid() {
        assert!(parse_path("/foo").is_err());
        assert!(parse_path("/v3-flatcontainer/id/13/wrongfile.nupkg").is_err());
    }

    #[test]
    fn validate_id_basic() {
        assert!(validate_package_id("Newtonsoft.Json"));
        assert!(validate_package_id("a"));
        assert!(validate_package_id("1abc"));
    }

    #[test]
    fn validate_id_rejects_garbage() {
        assert!(!validate_package_id(""));
        assert!(!validate_package_id(".start-with-dot"));
        assert!(!validate_package_id("has space"));
    }

    #[test]
    fn parse_minimal_nuspec() {
        let xml = r#"<?xml version="1.0"?>
            <package>
              <metadata>
                <id>Newtonsoft.Json</id>
                <version>13.0.1</version>
              </metadata>
            </package>
        "#;
        let p = parse_nuspec(xml).unwrap();
        assert_eq!(p.id, "Newtonsoft.Json");
        assert_eq!(p.version, "13.0.1");
        assert!(p.dependencies.is_empty());
    }

    #[test]
    fn parse_full_nuspec() {
        let xml = r#"<?xml version="1.0"?>
            <package>
              <metadata>
                <id>Acme.Lib</id>
                <version>1.0.0</version>
                <title>Acme Lib</title>
                <authors>Alice, Bob</authors>
                <description>A useful library</description>
                <projectUrl>https://acme.example/</projectUrl>
                <license>MIT</license>
                <tags>util acme lib</tags>
                <requireLicenseAcceptance>true</requireLicenseAcceptance>
                <dependencies>
                  <dependency id="Newtonsoft.Json" version="[13.0,)" />
                  <dependency id="Serilog" version="2.10.0" targetFramework="netstandard2.0" />
                </dependencies>
              </metadata>
            </package>
        "#;
        let p = parse_nuspec(xml).unwrap();
        assert_eq!(p.title.as_deref(), Some("Acme Lib"));
        assert_eq!(p.authors, vec!["Alice", "Bob"]);
        assert_eq!(p.tags, vec!["util", "acme", "lib"]);
        assert!(p.require_license_acceptance);
        assert_eq!(p.dependencies.len(), 2);
        assert_eq!(p.dependencies[0].id, "Newtonsoft.Json");
        assert_eq!(p.dependencies[1].target_framework.as_deref(), Some("netstandard2.0"));
    }

    #[test]
    fn parse_nuspec_missing_id_errors() {
        let xml = "<package><metadata><version>1</version></metadata></package>";
        assert!(matches!(
            parse_nuspec(xml).unwrap_err(),
            NuGetError::MissingElement("id")
        ));
    }

    #[test]
    fn parse_nuspec_missing_version_errors() {
        let xml = "<package><metadata><id>x</id></metadata></package>";
        assert!(matches!(
            parse_nuspec(xml).unwrap_err(),
            NuGetError::MissingElement("version")
        ));
    }

    #[test]
    fn parse_nuspec_invalid_id_errors() {
        let xml = "<package><metadata><id>.bad</id><version>1</version></metadata></package>";
        assert!(matches!(
            parse_nuspec(xml).unwrap_err(),
            NuGetError::InvalidNuspec(_)
        ));
    }

    #[test]
    fn license_falls_back_to_license_url() {
        let xml = "<package><metadata><id>x</id><version>1</version><licenseUrl>https://example/L</licenseUrl></metadata></package>";
        let p = parse_nuspec(xml).unwrap();
        assert_eq!(p.license.as_deref(), Some("https://example/L"));
    }
}
