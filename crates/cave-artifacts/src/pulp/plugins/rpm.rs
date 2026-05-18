// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_rpm — RPM package content plugin.
//!
//! Mirrors `pulp_rpm` upstream:
//! - NEVRA (Name-Epoch-Version-Release.Arch) parsing including explicit
//!   `<epoch>:` and `epoch-`-prefixed forms.
//! - createrepo_c-equivalent metadata bundles: `repomd.xml`, `primary.xml`,
//!   `filelists.xml`, `other.xml`, optional `updateinfo.xml` advisories and
//!   `modules.yaml` (modulemd_v2).
//! - `RpmAdvisory` and `RpmModulemd` content kinds threaded through
//!   `generate_metadata` so a publication can emit them alongside packages.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct RpmPlugin;

/// Parsed NEVRA coordinates (Name-Epoch:Version-Release.Arch).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nevra {
    pub name: String,
    pub epoch: String,
    pub version: String,
    pub release: String,
    pub arch: String,
}

impl Nevra {
    /// Canonical filename form (epoch is omitted when zero, matching
    /// `createrepo_c` / `rpm -q` behavior).
    pub fn filename(&self) -> String {
        format!("{}-{}-{}.{}.rpm", self.name, self.version, self.release, self.arch)
    }

    /// `name-epoch:version-release.arch` — the full RPM identifier used in
    /// `repomd.xml` and `updateinfo.xml`.
    pub fn nevra(&self) -> String {
        format!(
            "{}-{}:{}-{}.{}",
            self.name, self.epoch, self.version, self.release, self.arch
        )
    }
}

impl ArtifactsPlugin for RpmPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Rpm
    }

    fn name(&self) -> &str {
        "pulp_rpm"
    }

    fn content_types(&self) -> Vec<&str> {
        vec![
            "rpm.package",
            "rpm.advisory",
            "rpm.modulemd",
            "rpm.modulemd_defaults",
            "rpm.modulemd_obsoletes",
            "rpm.packagegroup",
            "rpm.packagecategory",
            "rpm.packageenvironment",
            "rpm.packagelangpacks",
            "rpm.repo_metadata_file",
        ]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let nevra = parse_nevra(filename)
            .ok_or_else(|| ArtifactsError::InvalidRequest(format!("not an rpm filename: {filename}")))?;
        let sha256 = hex::encode(Sha256::digest(data));

        let mut unit = ContentUnit::new(
            PluginType::Rpm,
            serde_json::json!({
                "name": nevra.name,
                "epoch": nevra.epoch,
                "version": nevra.version,
                "release": nevra.release,
                "arch": nevra.arch,
                "filename": filename,
                "checksum_type": "sha256",
                "pkgId": sha256,
                "nevra": nevra.nevra(),
                "location_href": format!("Packages/{}/{}", first_letter(&nevra.name), filename),
            }),
        );
        unit.relative_path = Some(relative_path.to_string());
        unit.sha256 = Some(sha256);
        unit.size = Some(data.len() as u64);
        Ok(unit)
    }

    fn generate_metadata(
        &self,
        repo_version: &RepositoryVersion,
        units: &[ContentUnit],
    ) -> serde_json::Value {
        let packages: Vec<&ContentUnit> = units
            .iter()
            .filter(|u| {
                u.metadata
                    .get("nevra")
                    .and_then(|v| v.as_str())
                    .is_some()
            })
            .collect();

        let advisories: Vec<&ContentUnit> = units
            .iter()
            .filter(|u| u.metadata.get("advisory_id").is_some())
            .collect();

        let modulemds: Vec<&ContentUnit> = units
            .iter()
            .filter(|u| u.metadata.get("module_stream").is_some())
            .collect();

        serde_json::json!({
            "repomd": render_repomd_xml(repo_version, packages.len(), !advisories.is_empty(), !modulemds.is_empty()),
            "primary": render_primary_xml(&packages),
            "filelists": render_filelists_xml(&packages),
            "other": render_other_xml(&packages),
            "updateinfo": (!advisories.is_empty()).then(|| render_updateinfo_xml(&advisories)),
            "modules": (!modulemds.is_empty()).then(|| render_modulemd_yaml(&modulemds)),
        })
    }
}

/// Build an [`RpmAdvisory`] content unit. Mirrors the upstream
/// `RpmAdvisory` model with `updated_date`, `references`, `pkglist`.
pub fn build_advisory(
    id: &str,
    severity: &str,
    advisory_type: &str,
    references: Vec<serde_json::Value>,
    pkglist: Vec<serde_json::Value>,
) -> ContentUnit {
    ContentUnit::new(
        PluginType::Rpm,
        serde_json::json!({
            "advisory_id": id,
            "severity": severity,
            "type": advisory_type,
            "updated_date": chrono::Utc::now().to_rfc3339(),
            "references": references,
            "pkglist": pkglist,
        }),
    )
}

/// Build an [`RpmModulemd`] content unit (modulemd_v2 stream).
pub fn build_modulemd(name: &str, stream: &str, version: u64, context: &str, arch: &str) -> ContentUnit {
    ContentUnit::new(
        PluginType::Rpm,
        serde_json::json!({
            "module_stream": stream,
            "name": name,
            "stream": stream,
            "version": version,
            "context": context,
            "arch": arch,
            "dependencies": [],
            "artifacts": [],
        }),
    )
}

/// Parse a filename of the form `name-version-release.arch.rpm` (epoch
/// is not in the filename, but may be in front of `version` separated
/// by a colon, e.g. `bash-2:5.1.8-6.el9.x86_64.rpm`).
pub fn parse_nevra(filename: &str) -> Option<Nevra> {
    let stem = filename.strip_suffix(".rpm")?;
    let (rest, arch) = stem.rsplit_once('.')?;
    let (rest2, release) = rest.rsplit_once('-')?;
    let (name, version_part) = rest2.rsplit_once('-')?;
    let (epoch, version) = match version_part.split_once(':') {
        Some((e, v)) => (e.to_string(), v.to_string()),
        None => ("0".to_string(), version_part.to_string()),
    };
    Some(Nevra {
        name: name.to_string(),
        epoch,
        version,
        release: release.to_string(),
        arch: arch.to_string(),
    })
}

fn first_letter(s: &str) -> &str {
    s.get(0..1).unwrap_or("_")
}

fn render_repomd_xml(
    ver: &RepositoryVersion,
    _package_count: usize,
    include_updateinfo: bool,
    include_modules: bool,
) -> String {
    let revision = ver.pulp_created.timestamp();
    let mut data_entries = String::new();
    for ty in ["primary", "filelists", "other"] {
        data_entries.push_str(&format!(
            "  <data type=\"{ty}\">\n    <location href=\"repodata/{ty}.xml.gz\"/>\n  </data>\n"
        ));
    }
    if include_updateinfo {
        data_entries.push_str(
            "  <data type=\"updateinfo\">\n    <location href=\"repodata/updateinfo.xml.gz\"/>\n  </data>\n",
        );
    }
    if include_modules {
        data_entries.push_str(
            "  <data type=\"modules\">\n    <location href=\"repodata/modules.yaml.gz\"/>\n  </data>\n",
        );
    }
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<repomd xmlns="http://linux.duke.edu/metadata/repo" xmlns:rpm="http://linux.duke.edu/metadata/rpm">
  <revision>{revision}</revision>
{data_entries}</repomd>"#
    )
}

fn render_primary_xml(packages: &[&ContentUnit]) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<metadata xmlns=\"http://linux.duke.edu/metadata/common\" packages=\"{}\">\n",
        packages.len()
    );
    for p in packages {
        let m = &p.metadata;
        let sha = p.sha256.as_deref().unwrap_or("");
        let size = p.size.unwrap_or(0);
        out.push_str(&format!(
            "  <package type=\"rpm\">\n    <name>{}</name>\n    <arch>{}</arch>\n    <version epoch=\"{}\" ver=\"{}\" rel=\"{}\"/>\n    <checksum type=\"sha256\" pkgid=\"YES\">{}</checksum>\n    <size package=\"{}\"/>\n    <location href=\"{}\"/>\n  </package>\n",
            m["name"].as_str().unwrap_or(""),
            m["arch"].as_str().unwrap_or(""),
            m["epoch"].as_str().unwrap_or("0"),
            m["version"].as_str().unwrap_or(""),
            m["release"].as_str().unwrap_or(""),
            sha,
            size,
            m["location_href"].as_str().unwrap_or(""),
        ));
    }
    out.push_str("</metadata>\n");
    out
}

fn render_filelists_xml(packages: &[&ContentUnit]) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<filelists xmlns=\"http://linux.duke.edu/metadata/filelists\" packages=\"{}\">\n",
        packages.len()
    );
    for p in packages {
        let m = &p.metadata;
        let sha = p.sha256.as_deref().unwrap_or("");
        out.push_str(&format!(
            "  <package pkgid=\"{}\" name=\"{}\" arch=\"{}\"><version epoch=\"{}\" ver=\"{}\" rel=\"{}\"/></package>\n",
            sha,
            m["name"].as_str().unwrap_or(""),
            m["arch"].as_str().unwrap_or(""),
            m["epoch"].as_str().unwrap_or("0"),
            m["version"].as_str().unwrap_or(""),
            m["release"].as_str().unwrap_or(""),
        ));
    }
    out.push_str("</filelists>\n");
    out
}

fn render_other_xml(packages: &[&ContentUnit]) -> String {
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<otherdata xmlns=\"http://linux.duke.edu/metadata/other\" packages=\"{}\">\n",
        packages.len()
    );
    for p in packages {
        let m = &p.metadata;
        let sha = p.sha256.as_deref().unwrap_or("");
        out.push_str(&format!(
            "  <package pkgid=\"{}\" name=\"{}\" arch=\"{}\"><version epoch=\"{}\" ver=\"{}\" rel=\"{}\"/></package>\n",
            sha,
            m["name"].as_str().unwrap_or(""),
            m["arch"].as_str().unwrap_or(""),
            m["epoch"].as_str().unwrap_or("0"),
            m["version"].as_str().unwrap_or(""),
            m["release"].as_str().unwrap_or(""),
        ));
    }
    out.push_str("</otherdata>\n");
    out
}

fn render_updateinfo_xml(advisories: &[&ContentUnit]) -> String {
    let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<updates>\n");
    for a in advisories {
        let m = &a.metadata;
        out.push_str(&format!(
            "  <update from=\"updates@example.com\" status=\"final\" type=\"{}\">\n    <id>{}</id>\n    <severity>{}</severity>\n  </update>\n",
            m["type"].as_str().unwrap_or("security"),
            m["advisory_id"].as_str().unwrap_or(""),
            m["severity"].as_str().unwrap_or("Low"),
        ));
    }
    out.push_str("</updates>\n");
    out
}

fn render_modulemd_yaml(modules: &[&ContentUnit]) -> String {
    let mut out = String::new();
    for m in modules {
        let md = &m.metadata;
        out.push_str(&format!(
            "---\ndocument: modulemd\nversion: 2\ndata:\n  name: {}\n  stream: {}\n  version: {}\n  context: {}\n  arch: {}\n",
            md["name"].as_str().unwrap_or(""),
            md["stream"].as_str().unwrap_or(""),
            md["version"].as_u64().unwrap_or(0),
            md["context"].as_str().unwrap_or(""),
            md["arch"].as_str().unwrap_or(""),
        ));
    }
    out.push_str("...\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_nevra() {
        let n = parse_nevra("bash-5.1.8-6.el9.x86_64.rpm").unwrap();
        assert_eq!(n.name, "bash");
        assert_eq!(n.epoch, "0");
        assert_eq!(n.version, "5.1.8");
        assert_eq!(n.release, "6.el9");
        assert_eq!(n.arch, "x86_64");
    }

    #[test]
    fn parse_nevra_with_explicit_epoch() {
        let n = parse_nevra("bash-2:5.1.8-6.el9.x86_64.rpm").unwrap();
        assert_eq!(n.name, "bash");
        assert_eq!(n.epoch, "2");
        assert_eq!(n.version, "5.1.8");
        assert_eq!(n.release, "6.el9");
        assert_eq!(n.arch, "x86_64");
    }

    #[test]
    fn parse_nevra_noarch() {
        let n = parse_nevra("python3-pip-21.2.3-7.el9.noarch.rpm").unwrap();
        assert_eq!(n.name, "python3-pip");
        assert_eq!(n.arch, "noarch");
        assert_eq!(n.version, "21.2.3");
    }

    #[test]
    fn parse_nevra_rejects_non_rpm() {
        assert!(parse_nevra("not-an-rpm.deb").is_none());
    }

    #[test]
    fn nevra_full_string_includes_epoch() {
        let n = parse_nevra("kernel-5.14.0-284.30.1.el9_2.x86_64.rpm").unwrap();
        assert_eq!(n.nevra(), "kernel-0:5.14.0-284.30.1.el9_2.x86_64");
    }

    #[test]
    fn location_href_uses_first_letter_bucket() {
        let plugin = RpmPlugin;
        let unit = plugin
            .parse_content(b"fake-rpm", "Packages/h/httpd-2.4.57-1.el9.x86_64.rpm")
            .unwrap();
        assert_eq!(
            unit.metadata["location_href"],
            "Packages/h/httpd-2.4.57-1.el9.x86_64.rpm"
        );
    }

    fn make_pkg(name: &str, ver: &str, rel: &str, arch: &str) -> ContentUnit {
        let plugin = RpmPlugin;
        plugin
            .parse_content(b"x", &format!("Packages/{}/{name}-{ver}-{rel}.{arch}.rpm", &name[..1]))
            .unwrap()
    }

    #[test]
    fn primary_xml_packages_attribute_matches_input() {
        let units = vec![
            make_pkg("bash", "5.1.8", "6.el9", "x86_64"),
            make_pkg("zsh", "5.8", "1.el9", "x86_64"),
        ];
        let refs: Vec<&ContentUnit> = units.iter().collect();
        let xml = render_primary_xml(&refs);
        assert!(xml.contains("packages=\"2\""));
        assert!(xml.contains("<name>bash</name>"));
        assert!(xml.contains("<checksum type=\"sha256\""));
        assert!(xml.contains("<location href=\"Packages/b/bash-5.1.8-6.el9.x86_64.rpm\"/>"));
    }

    #[test]
    fn repomd_includes_optional_data_types() {
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/rpm/rpm/x/", 1);
        let with_extras = render_repomd_xml(&ver, 1, true, true);
        assert!(with_extras.contains("type=\"primary\""));
        assert!(with_extras.contains("type=\"updateinfo\""));
        assert!(with_extras.contains("type=\"modules\""));

        let basic = render_repomd_xml(&ver, 1, false, false);
        assert!(!basic.contains("updateinfo"));
        assert!(!basic.contains("modules"));
    }

    #[test]
    fn generate_metadata_emits_all_streams() {
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/rpm/rpm/x/", 1);
        let plugin = RpmPlugin;
        let pkg = make_pkg("bash", "5.1.8", "6.el9", "x86_64");
        let advisory = build_advisory(
            "RHSA-2024:1234",
            "Critical",
            "security",
            vec![serde_json::json!({"href": "https://cve.example", "id": "CVE-2024-0001"})],
            vec![],
        );
        let module = build_modulemd("nodejs", "18", 20240101, "abc123", "x86_64");
        let meta = plugin.generate_metadata(&ver, &[pkg, advisory, module]);
        assert!(meta["repomd"].as_str().unwrap().contains("<repomd"));
        assert!(meta["primary"].as_str().unwrap().contains("<name>bash</name>"));
        assert!(meta["filelists"].as_str().unwrap().contains("filelists"));
        assert!(meta["other"].as_str().unwrap().contains("otherdata"));
        assert!(meta["updateinfo"].as_str().unwrap().contains("RHSA-2024:1234"));
        assert!(meta["modules"].as_str().unwrap().contains("nodejs"));
    }

    #[test]
    fn content_types_match_pulp_rpm_models() {
        let plugin = RpmPlugin;
        let kinds = plugin.content_types();
        for required in [
            "rpm.package",
            "rpm.advisory",
            "rpm.modulemd",
            "rpm.packagegroup",
            "rpm.packagecategory",
            "rpm.packageenvironment",
            "rpm.packagelangpacks",
            "rpm.repo_metadata_file",
        ] {
            assert!(kinds.contains(&required), "missing kind {required}");
        }
    }
}
