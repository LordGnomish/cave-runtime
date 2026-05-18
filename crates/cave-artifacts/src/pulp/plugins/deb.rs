// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_deb — Debian package content plugin.
//!
//! Mirrors `pulp_deb`:
//! - `{name}_{version}_{arch}.deb` filename parser.
//! - deb822 (RFC 822 / `control` file) parser and renderer — used for
//!   both `Packages` index entries and the top-level `Release` file.
//! - `Release` and `Packages` index renderers covering the
//!   suite × component × architecture matrix.
//! - `pool/{section}/{first_letter}/{name}/{filename}` placement helper.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub struct DebPlugin;

/// Parsed Debian filename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DebFilename {
    pub package: String,
    pub version: String,
    pub architecture: String,
}

impl DebFilename {
    pub fn parse(filename: &str) -> Option<Self> {
        let stem = filename.strip_suffix(".deb")?;
        let parts: Vec<&str> = stem.splitn(3, '_').collect();
        if parts.len() != 3 {
            return None;
        }
        Some(Self {
            package: parts[0].to_string(),
            version: parts[1].to_string(),
            architecture: parts[2].to_string(),
        })
    }

    /// `pool/{section}/{first_letter}/{name}/{name}_{version}_{arch}.deb`.
    /// Libraries get a `lib` bucket (so `libfoo` → `pool/main/libf/libfoo/...`).
    pub fn pool_path(&self, section: &str) -> String {
        let bucket = if self.package.starts_with("lib") && self.package.len() >= 4 {
            &self.package[0..4]
        } else {
            &self.package[0..1]
        };
        format!(
            "pool/{section}/{bucket}/{}/{}_{}_{}.deb",
            self.package, self.package, self.version, self.architecture
        )
    }
}

impl ArtifactsPlugin for DebPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Deb
    }

    fn name(&self) -> &str {
        "pulp_deb"
    }

    fn content_types(&self) -> Vec<&str> {
        vec![
            "deb.package",
            "deb.installer_package",
            "deb.package_index",
            "deb.installer_file_index",
            "deb.release",
            "deb.release_architecture",
            "deb.release_component",
            "deb.release_file",
            "deb.source_package",
        ]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let parsed = DebFilename::parse(filename).ok_or_else(|| {
            ArtifactsError::InvalidRequest(format!("not a .deb filename: {filename}"))
        })?;
        let sha256 = hex::encode(Sha256::digest(data));

        let mut unit = ContentUnit::new(
            PluginType::Deb,
            serde_json::json!({
                "package": parsed.package,
                "version": parsed.version,
                "architecture": parsed.architecture,
                "filename": filename,
                "sha256": sha256,
                "section": "main",
                "pool_path": parsed.pool_path("main"),
            }),
        );
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
        let mut sections: BTreeSet<String> = BTreeSet::new();
        let mut arches: BTreeSet<String> = BTreeSet::new();
        let mut groups: std::collections::BTreeMap<(String, String), Vec<&ContentUnit>> =
            std::collections::BTreeMap::new();
        for u in units {
            let section = u
                .metadata
                .get("section")
                .and_then(|v| v.as_str())
                .unwrap_or("main")
                .to_string();
            let arch = u
                .metadata
                .get("architecture")
                .and_then(|v| v.as_str())
                .unwrap_or("amd64")
                .to_string();
            sections.insert(section.clone());
            arches.insert(arch.clone());
            groups.entry((section, arch)).or_default().push(u);
        }

        let suite = "stable";
        let packages: serde_json::Map<String, serde_json::Value> = groups
            .iter()
            .map(|((sec, arch), pkgs)| {
                (
                    format!("{sec}/binary-{arch}/Packages"),
                    serde_json::json!(render_packages_index(pkgs)),
                )
            })
            .collect();

        let packages_pairs: Vec<(String, String)> = packages
            .iter()
            .map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string()))
            .collect();

        serde_json::json!({
            "suite": suite,
            "components": sections.iter().collect::<Vec<_>>(),
            "architectures": arches.iter().collect::<Vec<_>>(),
            "release": render_release(
                suite,
                sections.iter().cloned().collect(),
                arches.iter().cloned().collect(),
                &packages_pairs,
            ),
            "packages_files": serde_json::Value::Object(packages),
        })
    }
}

/// Render a single deb822 stanza (RFC 822 single record).
pub fn render_deb822_stanza(fields: &[(&str, &str)]) -> String {
    let mut out = String::new();
    for (k, v) in fields {
        if v.contains('\n') {
            out.push_str(&format!("{k}:\n"));
            for line in v.lines() {
                let line = if line.trim().is_empty() { "." } else { line };
                out.push_str(&format!(" {line}\n"));
            }
        } else {
            out.push_str(&format!("{k}: {v}\n"));
        }
    }
    out
}

/// Parse a sequence of deb822 stanzas (separated by blank lines).
pub fn parse_deb822(body: &str) -> Vec<Vec<(String, String)>> {
    let mut stanzas = Vec::new();
    let mut current: Vec<(String, String)> = Vec::new();
    let mut last_key: Option<String> = None;
    for line in body.lines() {
        if line.is_empty() {
            if !current.is_empty() {
                stanzas.push(std::mem::take(&mut current));
                last_key = None;
            }
            continue;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(ref key) = last_key {
                let trimmed = line.trim_start();
                let appended = if trimmed == "." {
                    String::new()
                } else {
                    trimmed.to_string()
                };
                if let Some((_, v)) = current.iter_mut().find(|(k, _)| k == key) {
                    v.push('\n');
                    v.push_str(&appended);
                }
            }
            continue;
        }
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();
            current.push((key.clone(), value));
            last_key = Some(key);
        }
    }
    if !current.is_empty() {
        stanzas.push(current);
    }
    stanzas
}

/// Render a `Packages` index — a deb822 file with one stanza per package.
pub fn render_packages_index(units: &[&ContentUnit]) -> String {
    let mut out = String::new();
    for u in units {
        let m = &u.metadata;
        let fields: Vec<(&str, String)> = vec![
            ("Package", m["package"].as_str().unwrap_or("").to_string()),
            ("Version", m["version"].as_str().unwrap_or("").to_string()),
            ("Architecture", m["architecture"].as_str().unwrap_or("").to_string()),
            (
                "Filename",
                m["pool_path"].as_str().unwrap_or("").to_string(),
            ),
            ("Size", u.size.unwrap_or(0).to_string()),
            ("SHA256", u.sha256.clone().unwrap_or_default()),
        ];
        let mut refs: Vec<(&str, &str)> = fields.iter().map(|(k, v)| (*k, v.as_str())).collect();
        if let Some(maint) = m.get("maintainer").and_then(|v| v.as_str()) {
            refs.push(("Maintainer", maint));
        }
        if let Some(desc) = m.get("description").and_then(|v| v.as_str()) {
            refs.push(("Description", desc));
        }
        if let Some(dep) = m.get("depends").and_then(|v| v.as_str()) {
            refs.push(("Depends", dep));
        }
        out.push_str(&render_deb822_stanza(&refs));
        out.push('\n');
    }
    out
}

/// Render a `Release` file describing all components × architectures and
/// listing every per-arch `Packages` index by SHA256 + size.
pub fn render_release(
    suite: &str,
    components: Vec<String>,
    architectures: Vec<String>,
    packages_files: &[(String, String)],
) -> String {
    let date = chrono::Utc::now().to_rfc2822();
    let mut out = format!(
        "Origin: Cave\nLabel: Cave\nSuite: {suite}\nCodename: {suite}\nDate: {date}\nComponents: {components}\nArchitectures: {arches}\nSHA256:\n",
        components = components.join(" "),
        arches = architectures.join(" "),
    );
    for (path, body) in packages_files {
        let sha = hex::encode(Sha256::digest(body.as_bytes()));
        out.push_str(&format!(" {} {} {}\n", sha, body.len(), path));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_deb_filename_basic() {
        let f = DebFilename::parse("libc6_2.35-0ubuntu3_amd64.deb").unwrap();
        assert_eq!(f.package, "libc6");
        assert_eq!(f.version, "2.35-0ubuntu3");
        assert_eq!(f.architecture, "amd64");
    }

    #[test]
    fn parse_deb_filename_rejects_non_deb() {
        assert!(DebFilename::parse("not-a-deb.rpm").is_none());
    }

    #[test]
    fn pool_path_uses_lib_bucket() {
        let f = DebFilename::parse("libc6_2.35_amd64.deb").unwrap();
        assert_eq!(f.pool_path("main"), "pool/main/libc/libc6/libc6_2.35_amd64.deb");
    }

    #[test]
    fn pool_path_uses_first_letter_for_non_libs() {
        let f = DebFilename::parse("curl_8.5.0_amd64.deb").unwrap();
        assert_eq!(f.pool_path("main"), "pool/main/c/curl/curl_8.5.0_amd64.deb");
    }

    #[test]
    fn deb822_render_simple_stanza() {
        let s = render_deb822_stanza(&[
            ("Package", "curl"),
            ("Version", "8.5.0"),
            ("Architecture", "amd64"),
        ]);
        assert_eq!(s, "Package: curl\nVersion: 8.5.0\nArchitecture: amd64\n");
    }

    #[test]
    fn deb822_render_multiline_description() {
        let s = render_deb822_stanza(&[(
            "Description",
            "short summary\nlonger detail",
        )]);
        assert!(s.starts_with("Description:\n short summary\n longer detail\n"));
    }

    #[test]
    fn deb822_parse_round_trip() {
        let body = "Package: curl\nVersion: 8.5.0\nArchitecture: amd64\n\nPackage: wget\nVersion: 1.21\nArchitecture: amd64\n";
        let stanzas = parse_deb822(body);
        assert_eq!(stanzas.len(), 2);
        assert_eq!(stanzas[0][0], ("Package".to_string(), "curl".to_string()));
        assert_eq!(stanzas[1][0], ("Package".to_string(), "wget".to_string()));
    }

    #[test]
    fn deb822_parse_multiline_field() {
        let body = "Package: curl\nDescription: short\n longer detail\n .\n footer\n";
        let stanzas = parse_deb822(body);
        let desc = &stanzas[0]
            .iter()
            .find(|(k, _)| k == "Description")
            .unwrap()
            .1;
        assert!(desc.contains("longer detail"));
        assert!(desc.contains("footer"));
        assert!(desc.contains('\n'));
    }

    fn make_pkg(name: &str, ver: &str, arch: &str) -> ContentUnit {
        let plugin = DebPlugin;
        plugin
            .parse_content(b"x", &format!("pool/main/{}/{name}/{name}_{ver}_{arch}.deb", &name[..1]))
            .unwrap()
    }

    #[test]
    fn packages_index_contains_required_fields() {
        let units = vec![make_pkg("curl", "8.5.0", "amd64")];
        let refs: Vec<&ContentUnit> = units.iter().collect();
        let idx = render_packages_index(&refs);
        assert!(idx.contains("Package: curl"));
        assert!(idx.contains("Version: 8.5.0"));
        assert!(idx.contains("Architecture: amd64"));
        assert!(idx.contains("Filename: pool/main/c/curl/curl_8.5.0_amd64.deb"));
        assert!(idx.contains("Size:"));
        assert!(idx.contains("SHA256:"));
    }

    #[test]
    fn release_file_describes_components_and_architectures() {
        let pkg_body = "Package: curl\n";
        let release = render_release(
            "stable",
            vec!["main".into(), "contrib".into()],
            vec!["amd64".into(), "arm64".into()],
            &[("main/binary-amd64/Packages".into(), pkg_body.into())],
        );
        assert!(release.contains("Suite: stable"));
        assert!(release.contains("Components: main contrib"));
        assert!(release.contains("Architectures: amd64 arm64"));
        assert!(release.contains("SHA256:"));
        assert!(release.contains("main/binary-amd64/Packages"));
    }

    #[test]
    fn generate_metadata_emits_full_matrix() {
        let plugin = DebPlugin;
        let units = vec![
            make_pkg("curl", "8.5.0", "amd64"),
            make_pkg("curl", "8.5.0", "arm64"),
        ];
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/deb/apt/x/", 1);
        let meta = plugin.generate_metadata(&ver, &units);
        let arches = meta["architectures"].as_array().unwrap();
        assert_eq!(arches.len(), 2);
        let pkg_files = meta["packages_files"].as_object().unwrap();
        assert!(pkg_files.contains_key("main/binary-amd64/Packages"));
        assert!(pkg_files.contains_key("main/binary-arm64/Packages"));
        assert!(meta["release"].as_str().unwrap().contains("Suite:"));
    }

    #[test]
    fn content_types_match_pulp_deb_models() {
        let plugin = DebPlugin;
        let kinds = plugin.content_types();
        for required in [
            "deb.package",
            "deb.installer_package",
            "deb.release",
            "deb.release_file",
            "deb.source_package",
        ] {
            assert!(kinds.contains(&required), "missing kind {required}");
        }
    }
}
