// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_python — Python Package Index (PyPI) content plugin.
//!
//! Mirrors `pulp_python`:
//! - PEP 427 wheel filename parser: `name-version[-build]-pythontag-abitag-platformtag.whl`.
//! - PEP 625 sdist filename parser: `name-version.tar.gz` (canonical) +
//!   legacy `.zip` / `.tar.bz2`.
//! - PEP 503 normalised project names (lowercase, hyphens for any
//!   sequence of `-_.`).
//! - PEP 503 simple HTML index renderer.
//! - PEP 691 JSON simple-API renderer (`api-version: "1.0"`).
//! - `PythonPackageType` classification (sdist vs bdist_wheel).

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct PythonPlugin;

/// Parsed PEP 427 wheel filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WheelFilename {
    pub distribution: String,
    pub version: String,
    pub build_tag: Option<String>,
    pub python_tag: String,
    pub abi_tag: String,
    pub platform_tag: String,
}

/// Source distribution filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdistFilename {
    pub distribution: String,
    pub version: String,
    pub extension: String,
}

/// Result of distribution-filename parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DistFilename {
    Wheel(WheelFilename),
    Sdist(SdistFilename),
}

impl DistFilename {
    pub fn name(&self) -> &str {
        match self {
            Self::Wheel(w) => &w.distribution,
            Self::Sdist(s) => &s.distribution,
        }
    }

    pub fn version(&self) -> &str {
        match self {
            Self::Wheel(w) => &w.version,
            Self::Sdist(s) => &s.version,
        }
    }

    pub fn is_wheel(&self) -> bool {
        matches!(self, Self::Wheel(_))
    }
}

impl ArtifactsPlugin for PythonPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Python
    }

    fn name(&self) -> &str {
        "pulp_python"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["python.python"]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let parsed = parse_distribution_filename(filename).ok_or_else(|| {
            ArtifactsError::InvalidRequest(format!("not a distribution filename: {filename}"))
        })?;
        let sha256 = hex::encode(Sha256::digest(data));

        let mut meta = serde_json::json!({
            "name": parsed.name(),
            "name_normalised": pep503_normalise(parsed.name()),
            "version": parsed.version(),
            "filename": filename,
            "packagetype": if parsed.is_wheel() { "bdist_wheel" } else { "sdist" },
            "sha256_digest": sha256,
        });
        if let DistFilename::Wheel(ref w) = parsed {
            meta["python_version"] = serde_json::json!(w.python_tag);
            meta["abi_tag"] = serde_json::json!(w.abi_tag);
            meta["platform_tag"] = serde_json::json!(w.platform_tag);
            if let Some(ref b) = w.build_tag {
                meta["build_tag"] = serde_json::json!(b);
            }
        }

        let mut unit = ContentUnit::new(PluginType::Python, meta);
        unit.relative_path = Some(relative_path.to_string());
        unit.sha256 = Some(sha256);
        unit.size = Some(data.len() as u64);
        Ok(unit)
    }

    fn generate_metadata(
        &self,
        _repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value {
        // Group packages by PEP 503 normalised project name.
        let mut by_project: std::collections::BTreeMap<String, Vec<&ContentUnit>> =
            Default::default();
        for u in units {
            let name = u
                .metadata
                .get("name_normalised")
                .or_else(|| u.metadata.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            by_project.entry(name).or_default().push(u);
        }

        let projects: serde_json::Map<String, serde_json::Value> = by_project
            .iter()
            .map(|(name, units)| {
                (
                    name.clone(),
                    serde_json::json!({
                        "simple_html": render_simple_html(name, units),
                        "simple_json": render_simple_json(name, units),
                    }),
                )
            })
            .collect();

        serde_json::json!({
            "projects": serde_json::Value::Object(projects),
            "project_count": by_project.len(),
        })
    }
}

/// PEP 503 normalisation: lowercase, replace runs of `-`, `_`, `.` with `-`.
pub fn pep503_normalise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut prev_sep = false;
    for c in name.chars() {
        if c == '-' || c == '_' || c == '.' {
            if !prev_sep && !out.is_empty() {
                out.push('-');
                prev_sep = true;
            }
        } else {
            out.push(c.to_ascii_lowercase());
            prev_sep = false;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

/// Parse a wheel or sdist filename. Returns `None` if the suffix is
/// unrecognised or the wheel-form has the wrong number of `-` segments.
pub fn parse_distribution_filename(filename: &str) -> Option<DistFilename> {
    if let Some(stem) = filename.strip_suffix(".whl") {
        return parse_wheel_stem(stem).map(DistFilename::Wheel);
    }
    for (ext, name) in &[(".tar.gz", "tar.gz"), (".tar.bz2", "tar.bz2"), (".zip", "zip")] {
        if let Some(stem) = filename.strip_suffix(ext) {
            let (dist, version) = stem.rsplit_once('-')?;
            return Some(DistFilename::Sdist(SdistFilename {
                distribution: dist.to_string(),
                version: version.to_string(),
                extension: name.to_string(),
            }));
        }
    }
    None
}

fn parse_wheel_stem(stem: &str) -> Option<WheelFilename> {
    let parts: Vec<&str> = stem.split('-').collect();
    match parts.len() {
        5 => Some(WheelFilename {
            distribution: parts[0].to_string(),
            version: parts[1].to_string(),
            build_tag: None,
            python_tag: parts[2].to_string(),
            abi_tag: parts[3].to_string(),
            platform_tag: parts[4].to_string(),
        }),
        6 => Some(WheelFilename {
            distribution: parts[0].to_string(),
            version: parts[1].to_string(),
            build_tag: Some(parts[2].to_string()),
            python_tag: parts[3].to_string(),
            abi_tag: parts[4].to_string(),
            platform_tag: parts[5].to_string(),
        }),
        _ => None,
    }
}

fn render_simple_html(project: &str, units: &[&ContentUnit]) -> String {
    let mut html = format!(
        "<!DOCTYPE html>\n<html>\n<head><meta name=\"pypi:repository-version\" content=\"1.0\"><title>Links for {project}</title></head>\n<body>\n<h1>Links for {project}</h1>\n",
    );
    for u in units {
        let filename = u.metadata["filename"].as_str().unwrap_or("");
        let sha = u.sha256.as_deref().unwrap_or("");
        let req_py = u
            .metadata
            .get("requires_python")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let req_py_attr = if req_py.is_empty() {
            String::new()
        } else {
            format!(" data-requires-python=\"{req_py}\"")
        };
        html.push_str(&format!(
            "<a href=\"../../packages/{filename}#sha256={sha}\"{req_py_attr}>{filename}</a><br/>\n",
        ));
    }
    html.push_str("</body>\n</html>");
    html
}

fn render_simple_json(project: &str, units: &[&ContentUnit]) -> serde_json::Value {
    let files: Vec<serde_json::Value> = units
        .iter()
        .map(|u| {
            let mut entry = serde_json::json!({
                "filename": u.metadata["filename"],
                "url": format!("../../packages/{}", u.metadata["filename"].as_str().unwrap_or("")),
                "hashes": { "sha256": u.sha256.clone().unwrap_or_default() },
            });
            if let Some(req_py) = u.metadata.get("requires_python") {
                entry["requires-python"] = req_py.clone();
            }
            entry
        })
        .collect();

    serde_json::json!({
        "meta": { "api-version": "1.0" },
        "name": project,
        "files": files,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pep503_normalise_examples() {
        assert_eq!(pep503_normalise("friendly_bard"), "friendly-bard");
        assert_eq!(pep503_normalise("Friendly.Bard"), "friendly-bard");
        assert_eq!(pep503_normalise("FRIENDLY-..-bard"), "friendly-bard");
        assert_eq!(pep503_normalise("requests"), "requests");
    }

    #[test]
    fn parse_wheel_no_build_tag() {
        let DistFilename::Wheel(w) =
            parse_distribution_filename("requests-2.31.0-py3-none-any.whl").unwrap()
        else {
            panic!("expected wheel");
        };
        assert_eq!(w.distribution, "requests");
        assert_eq!(w.version, "2.31.0");
        assert_eq!(w.build_tag, None);
        assert_eq!(w.python_tag, "py3");
        assert_eq!(w.abi_tag, "none");
        assert_eq!(w.platform_tag, "any");
    }

    #[test]
    fn parse_wheel_with_build_tag() {
        let DistFilename::Wheel(w) = parse_distribution_filename(
            "numpy-1.26.0-1-cp311-cp311-manylinux_2_17_x86_64.whl",
        )
        .unwrap()
        else {
            panic!("expected wheel");
        };
        assert_eq!(w.distribution, "numpy");
        assert_eq!(w.version, "1.26.0");
        assert_eq!(w.build_tag.as_deref(), Some("1"));
        assert_eq!(w.python_tag, "cp311");
        assert_eq!(w.abi_tag, "cp311");
        assert_eq!(w.platform_tag, "manylinux_2_17_x86_64");
    }

    #[test]
    fn parse_sdist_tar_gz() {
        let DistFilename::Sdist(s) =
            parse_distribution_filename("django-5.0.0.tar.gz").unwrap()
        else {
            panic!("expected sdist");
        };
        assert_eq!(s.distribution, "django");
        assert_eq!(s.version, "5.0.0");
        assert_eq!(s.extension, "tar.gz");
    }

    #[test]
    fn parse_sdist_zip_and_tar_bz2() {
        let DistFilename::Sdist(z) =
            parse_distribution_filename("legacy-0.1.zip").unwrap()
        else {
            panic!("expected sdist");
        };
        assert_eq!(z.extension, "zip");

        let DistFilename::Sdist(b) =
            parse_distribution_filename("legacy-0.1.tar.bz2").unwrap()
        else {
            panic!("expected sdist");
        };
        assert_eq!(b.extension, "tar.bz2");
    }

    #[test]
    fn parse_rejects_unknown_extension() {
        assert!(parse_distribution_filename("not-a-pkg.foo").is_none());
    }

    #[test]
    fn parse_content_records_wheel_tags() {
        let plugin = PythonPlugin;
        let unit = plugin
            .parse_content(b"x", "packages/requests-2.31.0-py3-none-any.whl")
            .unwrap();
        assert_eq!(unit.metadata["packagetype"], "bdist_wheel");
        assert_eq!(unit.metadata["python_version"], "py3");
        assert_eq!(unit.metadata["abi_tag"], "none");
        assert_eq!(unit.metadata["platform_tag"], "any");
        assert_eq!(unit.metadata["name_normalised"], "requests");
    }

    #[test]
    fn simple_html_index_pep503() {
        let plugin = PythonPlugin;
        let u = plugin
            .parse_content(b"x", "requests-2.31.0-py3-none-any.whl")
            .unwrap();
        let html = render_simple_html("requests", &[&u]);
        assert!(html.contains("Links for requests"));
        assert!(html.contains("pypi:repository-version"));
        assert!(html.contains("#sha256="));
    }

    #[test]
    fn simple_json_pep691_shape() {
        let plugin = PythonPlugin;
        let u = plugin
            .parse_content(b"x", "django-5.0.0.tar.gz")
            .unwrap();
        let v = render_simple_json("django", &[&u]);
        assert_eq!(v["meta"]["api-version"], "1.0");
        assert_eq!(v["name"], "django");
        let files = v["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0]["url"].as_str().unwrap().contains("django-5.0.0.tar.gz"));
        assert!(files[0]["hashes"]["sha256"].as_str().unwrap().len() == 64);
    }

    #[test]
    fn generate_metadata_groups_normalised() {
        let plugin = PythonPlugin;
        let a = plugin
            .parse_content(b"x", "Friendly_Bard-1.0-py3-none-any.whl")
            .unwrap();
        let b = plugin
            .parse_content(b"y", "friendly-bard-1.0.tar.gz")
            .unwrap();
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/python/python/x/", 1);
        let meta = plugin.generate_metadata(&ver, &[a, b]);
        assert_eq!(meta["project_count"], 1);
        let projects = meta["projects"].as_object().unwrap();
        assert!(projects.contains_key("friendly-bard"));
    }
}
