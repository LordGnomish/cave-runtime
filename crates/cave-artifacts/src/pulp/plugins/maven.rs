// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! pulp_maven — Maven2 artifact content plugin.
//!
//! Mirrors `pulp_maven`:
//! - GAV (Group:Artifact:Version) parsing from a repository-relative path
//!   `com/example/mylib/1.0.0/mylib-1.0.0[-classifier].{ext}`.
//! - Classifier parsing (`sources`, `javadoc`, `linux-x86_64`, …).
//! - SNAPSHOT recognition.
//! - `maven-metadata.xml` rendering per `groupId/artifactId`:
//!   `<versioning>` block with `<release>` (latest non-SNAPSHOT),
//!   `<latest>` (newest overall), `<versions>` list, and
//!   `<lastUpdated>` timestamp.
//! - File-extension awareness (`jar`, `pom`, `aar`, `war`, `ear`, `module`,
//!   `tar`, `zip`, plus signature/checksum companions `.asc`, `.md5`,
//!   `.sha1`, `.sha256`, `.sha512`).

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct MavenPlugin;

/// Parsed Maven GAV (Group : Artifact : Version) coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MavenCoordinates {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl MavenCoordinates {
    /// Parse from a repository-relative path like
    /// `com/example/mylib/1.0/mylib-1.0-sources.jar`.
    pub fn from_path(path: &str) -> Option<Self> {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let filename = *parts.last()?;
        let version = parts[parts.len() - 2].to_string();
        let artifact_id = parts[parts.len() - 3].to_string();
        let group_id = parts[..parts.len() - 3].join(".");

        let (primary, companion) = strip_companion_extension(filename);

        let prefix = format!("{artifact_id}-");
        if !primary.starts_with(&prefix) {
            return None;
        }
        let rest = &primary[prefix.len()..];

        let (after_version, has_classifier) = match rest.strip_prefix(&format!("{version}-")) {
            Some(rest) => (rest, true),
            None => (rest.strip_prefix(&version)?, false),
        };

        let (classifier, extension) = if has_classifier {
            let dot = after_version.rfind('.')?;
            (
                Some(after_version[..dot].to_string()),
                after_version[dot + 1..].to_string(),
            )
        } else {
            let ext = after_version.strip_prefix('.')?.to_string();
            (None, ext)
        };

        let final_ext = if let Some(c) = companion {
            format!("{extension}.{c}")
        } else {
            extension
        };

        Some(Self {
            group_id,
            artifact_id,
            version,
            classifier,
            extension: final_ext,
        })
    }

    pub fn is_snapshot(&self) -> bool {
        self.version.contains("SNAPSHOT")
    }

    /// `groupId:artifactId` — the maven-metadata.xml grouping key.
    pub fn ga_key(&self) -> String {
        format!("{}:{}", self.group_id, self.artifact_id)
    }
}

/// Identify and strip a companion-file extension. Returns `(primary, extension)`
/// where `extension` is the companion suffix (e.g. `asc`, `md5`, `sha1`,
/// `sha256`, `sha512`) or `None` if the file is its own primary.
fn strip_companion_extension(filename: &str) -> (&str, Option<&str>) {
    for ext in &["asc", "md5", "sha1", "sha256", "sha512"] {
        if let Some(stem) = filename.strip_suffix(&format!(".{ext}")) {
            return (stem, Some(ext));
        }
    }
    (filename, None)
}

impl ArtifactsPlugin for MavenPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Maven
    }

    fn name(&self) -> &str {
        "pulp_maven"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["maven.artifact", "maven.metadata"]
    }

    fn parse_content(
        &self,
        data: &[u8],
        relative_path: &str,
    ) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let coords = MavenCoordinates::from_path(relative_path).ok_or_else(|| {
            ArtifactsError::InvalidRequest(format!("not a maven path: {relative_path}"))
        })?;
        let metadata = serde_json::json!({
            "group_id": coords.group_id,
            "artifact_id": coords.artifact_id,
            "version": coords.version,
            "classifier": coords.classifier,
            "extension": coords.extension,
            "is_snapshot": coords.is_snapshot(),
            "ga_key": coords.ga_key(),
        });

        let mut unit = ContentUnit::new(PluginType::Maven, metadata);
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
        let mut by_ga: BTreeMap<String, Vec<&ContentUnit>> = BTreeMap::new();
        for u in units {
            let ga = u
                .metadata
                .get("ga_key")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            if ga.is_empty() {
                continue;
            }
            by_ga.entry(ga).or_default().push(u);
        }

        let docs: serde_json::Map<String, serde_json::Value> = by_ga
            .iter()
            .map(|(ga, units)| (ga.clone(), serde_json::json!(render_maven_metadata_xml(units))))
            .collect();

        serde_json::json!({
            "maven_metadata_xml": serde_json::Value::Object(docs),
        })
    }
}

/// Render `maven-metadata.xml` for one groupId/artifactId. All units in
/// `versions` are assumed to share `groupId` and `artifactId`.
pub fn render_maven_metadata_xml(versions: &[&ContentUnit]) -> String {
    if versions.is_empty() {
        return String::new();
    }
    let group_id = versions[0].metadata["group_id"].as_str().unwrap_or("");
    let artifact_id = versions[0].metadata["artifact_id"].as_str().unwrap_or("");

    let mut unique_versions: Vec<String> = versions
        .iter()
        .filter_map(|u| u.metadata.get("version").and_then(|v| v.as_str()).map(String::from))
        .collect();
    unique_versions.sort();
    unique_versions.dedup();

    let release = unique_versions
        .iter()
        .rev()
        .find(|v| !v.contains("SNAPSHOT"))
        .cloned();
    let latest = unique_versions.last().cloned();
    let last_updated = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string();

    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<metadata>\n  <groupId>{group_id}</groupId>\n  <artifactId>{artifact_id}</artifactId>\n  <versioning>\n",
    );
    if let Some(ref l) = latest {
        out.push_str(&format!("    <latest>{l}</latest>\n"));
    }
    if let Some(ref r) = release {
        out.push_str(&format!("    <release>{r}</release>\n"));
    }
    out.push_str("    <versions>\n");
    for v in &unique_versions {
        out.push_str(&format!("      <version>{v}</version>\n"));
    }
    out.push_str(&format!(
        "    </versions>\n    <lastUpdated>{last_updated}</lastUpdated>\n  </versioning>\n</metadata>\n",
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maven_path_jar() {
        let c = MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0.jar").unwrap();
        assert_eq!(c.group_id, "com.example");
        assert_eq!(c.artifact_id, "mylib");
        assert_eq!(c.version, "1.0.0");
        assert_eq!(c.classifier, None);
        assert_eq!(c.extension, "jar");
        assert!(!c.is_snapshot());
    }

    #[test]
    fn parse_maven_path_with_classifier() {
        let c =
            MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0-sources.jar").unwrap();
        assert_eq!(c.classifier, Some("sources".to_string()));
        assert_eq!(c.extension, "jar");
    }

    #[test]
    fn parse_maven_path_javadoc() {
        let c =
            MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0-javadoc.jar").unwrap();
        assert_eq!(c.classifier, Some("javadoc".to_string()));
    }

    #[test]
    fn parse_maven_path_native_classifier() {
        let c = MavenCoordinates::from_path(
            "io/netty/netty-transport-native-epoll/4.1.94.Final/netty-transport-native-epoll-4.1.94.Final-linux-x86_64.jar",
        )
        .unwrap();
        assert_eq!(c.classifier, Some("linux-x86_64".to_string()));
        assert_eq!(c.extension, "jar");
    }

    #[test]
    fn parse_maven_pom_companion_asc() {
        let c =
            MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0.pom.asc").unwrap();
        assert_eq!(c.extension, "pom.asc");
        assert_eq!(c.classifier, None);
    }

    #[test]
    fn parse_maven_jar_companion_sha256() {
        let c =
            MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0.jar.sha256").unwrap();
        assert_eq!(c.extension, "jar.sha256");
    }

    #[test]
    fn parse_maven_snapshot_recognised() {
        let c = MavenCoordinates::from_path(
            "org/acme/service/2.0.0-SNAPSHOT/service-2.0.0-SNAPSHOT.jar",
        )
        .unwrap();
        assert!(c.is_snapshot());
    }

    #[test]
    fn parse_maven_returns_none_on_malformed_path() {
        assert!(MavenCoordinates::from_path("short/path.jar").is_none());
        assert!(MavenCoordinates::from_path("com/example/wrongname/1.0/notmylib-1.0.jar").is_none());
    }

    #[test]
    fn maven_metadata_xml_picks_release_and_latest() {
        let plugin = MavenPlugin;
        let v1 = plugin
            .parse_content(b"x", "com/example/lib/1.0.0/lib-1.0.0.jar")
            .unwrap();
        let v2 = plugin
            .parse_content(b"x", "com/example/lib/1.1.0/lib-1.1.0.jar")
            .unwrap();
        let v3 = plugin
            .parse_content(b"x", "com/example/lib/2.0.0-SNAPSHOT/lib-2.0.0-SNAPSHOT.jar")
            .unwrap();
        let refs: Vec<&ContentUnit> = vec![&v1, &v2, &v3];
        let xml = render_maven_metadata_xml(&refs);
        assert!(xml.contains("<groupId>com.example</groupId>"));
        assert!(xml.contains("<artifactId>lib</artifactId>"));
        assert!(xml.contains("<release>1.1.0</release>"));
        assert!(xml.contains("<latest>2.0.0-SNAPSHOT</latest>"));
        assert!(xml.contains("<version>1.0.0</version>"));
        assert!(xml.contains("<lastUpdated>"));
    }

    #[test]
    fn generate_metadata_emits_one_xml_per_ga() {
        let plugin = MavenPlugin;
        let a = plugin
            .parse_content(b"x", "com/example/lib/1.0.0/lib-1.0.0.jar")
            .unwrap();
        let b = plugin
            .parse_content(b"x", "com/example/other/3.0.0/other-3.0.0.jar")
            .unwrap();
        let ver = RepositoryVersion::new("/pulp/api/v3/repositories/maven/maven/x/", 1);
        let meta = plugin.generate_metadata(&ver, &[a, b]);
        let docs = meta["maven_metadata_xml"].as_object().unwrap();
        assert!(docs.contains_key("com.example:lib"));
        assert!(docs.contains_key("com.example:other"));
    }

    #[test]
    fn ga_key_is_groupid_artifactid() {
        let c = MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0.jar").unwrap();
        assert_eq!(c.ga_key(), "com.example:mylib");
    }
}
