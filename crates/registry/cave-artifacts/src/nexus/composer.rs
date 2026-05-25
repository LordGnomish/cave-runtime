// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: sonatype/nexus-public plugins/nexus-repository-composer
//
//! PHP Composer adapter — parse `composer.json` package metadata and the
//! `provider-includes` /  `package` index files Composer's repository
//! protocol uses (<https://getcomposer.org/doc/05-repositories.md>).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposerPackage {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub package_type: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub authors: Vec<ComposerAuthor>,
    #[serde(default)]
    pub require: Vec<ComposerDep>,
    #[serde(default)]
    pub require_dev: Vec<ComposerDep>,
    #[serde(default)]
    pub license: Vec<String>,
    pub dist: Option<ComposerDist>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposerAuthor {
    pub name: String,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposerDep {
    pub name: String,
    pub constraint: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComposerDist {
    #[serde(rename = "type")]
    pub dist_type: String,
    pub url: String,
    pub reference: Option<String>,
    pub shasum: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ComposerError {
    #[error("invalid JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("missing field `{0}`")]
    MissingField(&'static str),
    #[error("invalid package name `{0}` (expected `vendor/name`)")]
    InvalidName(String),
}

/// Parse a single `composer.json` document.
pub fn parse_composer_json(s: &str) -> Result<ComposerPackage, ComposerError> {
    let v: Value = serde_json::from_str(s)?;
    let name = v
        .get("name")
        .and_then(Value::as_str)
        .ok_or(ComposerError::MissingField("name"))?
        .to_string();
    if !validate_package_name(&name) {
        return Err(ComposerError::InvalidName(name));
    }
    let version = v
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or("dev-master")
        .to_string();
    let package_type = v.get("type").and_then(Value::as_str).map(String::from);
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .map(String::from);
    let authors = v
        .get("authors")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    let name = x.get("name").and_then(Value::as_str)?.to_string();
                    let email = x.get("email").and_then(Value::as_str).map(String::from);
                    Some(ComposerAuthor { name, email })
                })
                .collect()
        })
        .unwrap_or_default();
    let require = parse_deps(v.get("require"));
    let require_dev = parse_deps(v.get("require-dev"));
    let license = parse_license(v.get("license"));
    let dist = v.get("dist").and_then(parse_dist);
    Ok(ComposerPackage {
        name,
        version,
        package_type,
        description,
        authors,
        require,
        require_dev,
        license,
        dist,
    })
}

fn parse_deps(v: Option<&Value>) -> Vec<ComposerDep> {
    let Some(obj) = v.and_then(Value::as_object) else {
        return Vec::new();
    };
    obj.iter()
        .filter_map(|(name, c)| {
            c.as_str().map(|c| ComposerDep {
                name: name.clone(),
                constraint: c.to_string(),
            })
        })
        .collect()
}

fn parse_license(v: Option<&Value>) -> Vec<String> {
    match v {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|x| x.as_str().map(String::from))
            .collect(),
        _ => Vec::new(),
    }
}

fn parse_dist(v: &Value) -> Option<ComposerDist> {
    let dist_type = v.get("type")?.as_str()?.to_string();
    let url = v.get("url")?.as_str()?.to_string();
    let reference = v.get("reference").and_then(Value::as_str).map(String::from);
    let shasum = v.get("shasum").and_then(Value::as_str).map(String::from);
    Some(ComposerDist {
        dist_type,
        url,
        reference,
        shasum,
    })
}

/// Composer enforces `vendor/name` package naming. Both parts are
/// `[a-z0-9_.-]+` and the slash is mandatory (`vendor/name`).
pub fn validate_package_name(name: &str) -> bool {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 2 {
        return false;
    }
    for p in &parts {
        if p.is_empty() {
            return false;
        }
        if !p
            .bytes()
            .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.' | b'-'))
        {
            return false;
        }
    }
    true
}

/// Repository request paths that this adapter recognises. The Composer
/// protocol layers two cache-friendly index types and an actual download
/// of an archived distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ComposerPath {
    /// `packages.json`
    PackagesJson,
    /// `p2/<vendor>/<name>.json`
    PackageMetadata { vendor: String, name: String },
    /// `dists/<vendor>/<name>/<version>.<format>`
    Dist {
        vendor: String,
        name: String,
        version: String,
        format: String,
    },
}

pub fn parse_path(path: &str) -> Option<ComposerPath> {
    let trimmed = path.trim_start_matches('/');
    if trimmed == "packages.json" {
        return Some(ComposerPath::PackagesJson);
    }
    if let Some(rest) = trimmed.strip_prefix("p2/") {
        let rest = rest.strip_suffix(".json")?;
        let (vendor, name) = rest.split_once('/')?;
        return Some(ComposerPath::PackageMetadata {
            vendor: vendor.to_string(),
            name: name.to_string(),
        });
    }
    if let Some(rest) = trimmed.strip_prefix("dists/") {
        let parts: Vec<&str> = rest.split('/').collect();
        if parts.len() == 3 {
            let last = parts[2];
            let (version, format) = last.rsplit_once('.')?;
            return Some(ComposerPath::Dist {
                vendor: parts[0].to_string(),
                name: parts[1].to_string(),
                version: version.to_string(),
                format: format.to_string(),
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_composer_json() {
        let s = r#"{
            "name": "acme/widget",
            "version": "1.2.3",
            "type": "library",
            "description": "A widget",
            "license": "MIT",
            "require": {"php": ">=8.0", "monolog/monolog": "^2.0"},
            "authors": [{"name": "Alice", "email": "a@example.com"}]
        }"#;
        let p = parse_composer_json(s).unwrap();
        assert_eq!(p.name, "acme/widget");
        assert_eq!(p.version, "1.2.3");
        assert_eq!(p.package_type.as_deref(), Some("library"));
        assert_eq!(p.license, vec!["MIT"]);
        assert_eq!(p.require.len(), 2);
        assert_eq!(p.authors[0].email.as_deref(), Some("a@example.com"));
    }

    #[test]
    fn parse_multi_license_array() {
        let s = r#"{"name":"x/y","license":["MIT","Apache-2.0"]}"#;
        let p = parse_composer_json(s).unwrap();
        assert_eq!(p.license, vec!["MIT", "Apache-2.0"]);
    }

    #[test]
    fn parse_dist_block() {
        let s = r#"{
            "name": "x/y",
            "dist": {
                "type": "zip",
                "url": "https://example/dist.zip",
                "reference": "abc123",
                "shasum": "deadbeef"
            }
        }"#;
        let p = parse_composer_json(s).unwrap();
        let d = p.dist.unwrap();
        assert_eq!(d.dist_type, "zip");
        assert_eq!(d.url, "https://example/dist.zip");
        assert_eq!(d.shasum.as_deref(), Some("deadbeef"));
    }

    #[test]
    fn parse_missing_name_errors() {
        let s = r#"{"version":"1.0"}"#;
        assert!(matches!(
            parse_composer_json(s).unwrap_err(),
            ComposerError::MissingField("name")
        ));
    }

    #[test]
    fn parse_invalid_name_errors() {
        assert!(parse_composer_json(r#"{"name":"single-token"}"#).is_err());
        assert!(parse_composer_json(r#"{"name":"a/b/c"}"#).is_err());
        assert!(parse_composer_json(r#"{"name":"BAD/case"}"#).is_err());
    }

    #[test]
    fn version_defaults_to_dev_master() {
        let p = parse_composer_json(r#"{"name":"x/y"}"#).unwrap();
        assert_eq!(p.version, "dev-master");
    }

    #[test]
    fn validate_name_accepts_normal() {
        assert!(validate_package_name("vendor/name"));
        assert!(validate_package_name("a/b-c.d_e"));
    }

    #[test]
    fn validate_name_rejects_garbage() {
        assert!(!validate_package_name("nocolon"));
        assert!(!validate_package_name("UPPER/case"));
        assert!(!validate_package_name("/"));
        assert!(!validate_package_name("a/"));
        assert!(!validate_package_name("/b"));
    }

    #[test]
    fn parse_path_packages_json() {
        assert_eq!(parse_path("packages.json"), Some(ComposerPath::PackagesJson));
        assert_eq!(parse_path("/packages.json"), Some(ComposerPath::PackagesJson));
    }

    #[test]
    fn parse_path_p2_metadata() {
        let p = parse_path("/p2/acme/widget.json").unwrap();
        assert_eq!(
            p,
            ComposerPath::PackageMetadata {
                vendor: "acme".into(),
                name: "widget".into(),
            }
        );
    }

    #[test]
    fn parse_path_dist() {
        let p = parse_path("/dists/acme/widget/1.2.3.zip").unwrap();
        assert_eq!(
            p,
            ComposerPath::Dist {
                vendor: "acme".into(),
                name: "widget".into(),
                version: "1.2.3".into(),
                format: "zip".into(),
            }
        );
    }

    #[test]
    fn parse_path_unknown() {
        assert!(parse_path("/foo/bar").is_none());
        assert!(parse_path("").is_none());
    }

    #[test]
    fn require_dev_separated_from_require() {
        let s = r#"{
            "name": "x/y",
            "require": {"a/b": "^1.0"},
            "require-dev": {"phpunit/phpunit": "^9.0"}
        }"#;
        let p = parse_composer_json(s).unwrap();
        assert_eq!(p.require.len(), 1);
        assert_eq!(p.require_dev.len(), 1);
        assert_eq!(p.require_dev[0].name, "phpunit/phpunit");
    }
}
