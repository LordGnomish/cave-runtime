// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_maven — Maven 2 artifact content plugin.
//!
//! Implements:
//! - POM 4.0.0 XML reader (`parse_pom_xml`) — surfaces GAV + parent +
//!   dependencies. POM is intentionally shallow + heavily nested but
//!   the well-known tags we care about (groupId / artifactId / version /
//!   packaging / parent.* / dependencies.dependency.*) can be parsed
//!   with a path-driven streaming walker — no external XML dep needed
//!   for the parity surface we expose.
//! - maven-metadata.xml generator (`generate_maven_metadata_xml`)
//!   producing the canonical <metadata>/<versioning> body.
//! - SnapshotInfo::from_filename — extract `{timestamp}-{buildnumber}`
//!   from a unique-snapshot artifact filename.
//!
//! Upstream parity: pulp/pulp_maven `pulp_maven/app/models.py` +
//! Apache Maven Reference: POM Reference.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct MavenPlugin;

/// Parsed Maven GAV coordinates (filename-derived, kept from Phase 1).
#[derive(Debug, PartialEq)]
pub struct MavenCoordinates {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub classifier: Option<String>,
    pub extension: String,
}

impl MavenCoordinates {
    pub fn from_path(path: &str) -> Option<Self> {
        let parts: Vec<&str> = path.split('/').collect();
        if parts.len() < 4 {
            return None;
        }
        let filename = *parts.last()?;
        let version = parts[parts.len() - 2].to_string();
        let artifact_id = parts[parts.len() - 3].to_string();
        let group_id = parts[..parts.len() - 3].join(".");
        let stem = filename.splitn(2, &format!("{artifact_id}-")).nth(1)?;
        let (classifier, ext) = if let Some(rest) = stem.strip_prefix(&format!("{version}-")) {
            let dot = rest.rfind('.')?;
            (Some(rest[..dot].to_string()), rest[dot + 1..].to_string())
        } else {
            let dot = stem.rfind('.')?;
            (None, stem[dot + 1..].to_string())
        };
        Some(Self {
            group_id,
            artifact_id,
            version,
            classifier,
            extension: ext,
        })
    }

    pub fn is_snapshot(&self) -> bool {
        self.version.contains("SNAPSHOT")
    }
}

// ── POM parsing ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MavenParent {
    pub group_id: String,
    pub artifact_id: String,
    pub version: String,
    pub relative_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MavenDependency {
    pub group_id: String,
    pub artifact_id: String,
    pub version: Option<String>,
    pub scope: Option<String>,
    pub classifier: Option<String>,
    pub r#type: Option<String>,
    pub optional: Option<bool>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MavenPom {
    pub group_id: Option<String>,
    pub artifact_id: String,
    pub version: Option<String>,
    pub packaging: Option<String>,
    pub name: Option<String>,
    pub parent: Option<MavenParent>,
    pub dependencies: Vec<MavenDependency>,
}

/// Parse a Maven POM 4.0.0 body. Walks element open/close events and
/// tracks the path stack to dispatch text content to the correct field.
pub fn parse_pom_xml(raw: &str) -> Result<MavenPom, ArtifactsError> {
    let mut pom = MavenPom::default();
    let mut stack: Vec<String> = Vec::new();
    let mut cur_dep: Option<MavenDependency> = None;
    let mut cur_parent: Option<MavenParent> = None;

    let events = tokenize_xml(raw)?;
    let mut i = 0;
    while i < events.len() {
        match &events[i] {
            XmlEvent::Open(name) => {
                stack.push(name.clone());
                let path = stack.join("/");
                if path == "project/parent" {
                    cur_parent = Some(MavenParent::default());
                } else if path == "project/dependencies/dependency" {
                    cur_dep = Some(MavenDependency::default());
                }
            }
            XmlEvent::Text(text) => {
                let path = stack.join("/");
                let val = text.trim().to_string();
                if val.is_empty() {
                    i += 1;
                    continue;
                }
                match path.as_str() {
                    "project/groupId" => pom.group_id = Some(val),
                    "project/artifactId" => pom.artifact_id = val,
                    "project/version" => pom.version = Some(val),
                    "project/packaging" => pom.packaging = Some(val),
                    "project/name" => pom.name = Some(val),
                    "project/parent/groupId" => {
                        if let Some(p) = cur_parent.as_mut() {
                            p.group_id = val;
                        }
                    }
                    "project/parent/artifactId" => {
                        if let Some(p) = cur_parent.as_mut() {
                            p.artifact_id = val;
                        }
                    }
                    "project/parent/version" => {
                        if let Some(p) = cur_parent.as_mut() {
                            p.version = val;
                        }
                    }
                    "project/parent/relativePath" => {
                        if let Some(p) = cur_parent.as_mut() {
                            p.relative_path = Some(val);
                        }
                    }
                    "project/dependencies/dependency/groupId" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.group_id = val;
                        }
                    }
                    "project/dependencies/dependency/artifactId" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.artifact_id = val;
                        }
                    }
                    "project/dependencies/dependency/version" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.version = Some(val);
                        }
                    }
                    "project/dependencies/dependency/scope" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.scope = Some(val);
                        }
                    }
                    "project/dependencies/dependency/classifier" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.classifier = Some(val);
                        }
                    }
                    "project/dependencies/dependency/type" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.r#type = Some(val);
                        }
                    }
                    "project/dependencies/dependency/optional" => {
                        if let Some(d) = cur_dep.as_mut() {
                            d.optional = Some(val == "true");
                        }
                    }
                    _ => {}
                }
            }
            XmlEvent::Close(name) => {
                let path = stack.join("/");
                if path == "project/parent" {
                    if let Some(p) = cur_parent.take() {
                        pom.parent = Some(p);
                    }
                } else if path == "project/dependencies/dependency" {
                    if let Some(d) = cur_dep.take() {
                        pom.dependencies.push(d);
                    }
                }
                let last = stack.pop();
                if last.as_deref() != Some(name.as_str()) {
                    // Tolerate self-closing reorderings by quick-xml-style writers;
                    // POM is small and human-written so mismatches are rare.
                }
            }
        }
        i += 1;
    }
    if pom.artifact_id.is_empty() {
        return Err(ArtifactsError::InvalidRequest(
            "POM missing <artifactId>".into(),
        ));
    }
    // Inherit groupId/version from parent if absent on child (Maven rules).
    if pom.group_id.is_none() {
        if let Some(p) = &pom.parent {
            pom.group_id = Some(p.group_id.clone());
        }
    }
    if pom.version.is_none() {
        if let Some(p) = &pom.parent {
            pom.version = Some(p.version.clone());
        }
    }
    Ok(pom)
}

// ── minimal XML tokenizer ────────────────────────────────────────────────────
//
// Recognises: comments (`<!-- ... -->`), CDATA (`<![CDATA[...]]>`),
// processing instructions (`<?...?>`), open/close/self-close tags,
// character data. Sufficient for POM and Chart.yaml-adjacent XML.

#[derive(Debug)]
enum XmlEvent {
    Open(String),
    Close(String),
    Text(String),
}

fn tokenize_xml(raw: &str) -> Result<Vec<XmlEvent>, ArtifactsError> {
    let bytes = raw.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        if bytes[i] == b'<' {
            if i + 4 <= bytes.len() && &bytes[i..i + 4] == b"<!--" {
                // comment
                let end = find_subseq(&bytes[i + 4..], b"-->")
                    .ok_or_else(|| ArtifactsError::InvalidRequest("unterminated comment".into()))?;
                i = i + 4 + end + 3;
                continue;
            }
            if i + 9 <= bytes.len() && &bytes[i..i + 9] == b"<![CDATA[" {
                let end = find_subseq(&bytes[i + 9..], b"]]>")
                    .ok_or_else(|| ArtifactsError::InvalidRequest("unterminated CDATA".into()))?;
                let text = std::str::from_utf8(&bytes[i + 9..i + 9 + end])
                    .map_err(|_| ArtifactsError::InvalidRequest("CDATA utf8".into()))?
                    .to_string();
                out.push(XmlEvent::Text(text));
                i = i + 9 + end + 3;
                continue;
            }
            if i + 2 <= bytes.len() && bytes[i + 1] == b'?' {
                // PI: skip to ?>
                let end = find_subseq(&bytes[i + 2..], b"?>")
                    .ok_or_else(|| ArtifactsError::InvalidRequest("unterminated PI".into()))?;
                i = i + 2 + end + 2;
                continue;
            }
            // Find closing '>'.
            let close = bytes[i + 1..]
                .iter()
                .position(|&b| b == b'>')
                .ok_or_else(|| ArtifactsError::InvalidRequest("unterminated tag".into()))?;
            let inner = std::str::from_utf8(&bytes[i + 1..i + 1 + close])
                .map_err(|_| ArtifactsError::InvalidRequest("tag utf8".into()))?
                .trim();
            if let Some(rest) = inner.strip_prefix('/') {
                let name = local_name(rest.trim());
                out.push(XmlEvent::Close(name));
            } else if let Some(stripped) = inner.strip_suffix('/') {
                let name_raw = stripped.trim();
                let name = local_name(name_raw.split_whitespace().next().unwrap_or(""));
                out.push(XmlEvent::Open(name.clone()));
                out.push(XmlEvent::Close(name));
            } else {
                let name = local_name(inner.split_whitespace().next().unwrap_or(""));
                out.push(XmlEvent::Open(name));
            }
            i = i + 1 + close + 1;
        } else {
            // Text up to next '<'.
            let next = bytes[i..].iter().position(|&b| b == b'<').unwrap_or(bytes.len() - i);
            let text = std::str::from_utf8(&bytes[i..i + next])
                .map_err(|_| ArtifactsError::InvalidRequest("text utf8".into()))?;
            let decoded = decode_entities(text);
            if !decoded.trim().is_empty() {
                out.push(XmlEvent::Text(decoded));
            }
            i += next;
        }
    }
    Ok(out)
}

fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Strip namespace prefix `ns:name` → `name`.
fn local_name(s: &str) -> String {
    s.split(':').last().unwrap_or(s).to_string()
}

fn decode_entities(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
}

// ── maven-metadata.xml generator ────────────────────────────────────────────

/// Render a `maven-metadata.xml` body for the given artifact.
pub fn generate_maven_metadata_xml(
    group_id: &str,
    artifact_id: &str,
    versions: &[String],
    latest: Option<&str>,
    release: Option<&str>,
    last_updated: u64,
) -> String {
    let mut x = String::with_capacity(512);
    x.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    x.push_str("<metadata>\n");
    x.push_str(&format!("  <groupId>{}</groupId>\n", group_id));
    x.push_str(&format!("  <artifactId>{}</artifactId>\n", artifact_id));
    x.push_str("  <versioning>\n");
    if let Some(l) = latest {
        x.push_str(&format!("    <latest>{}</latest>\n", l));
    }
    if let Some(r) = release {
        x.push_str(&format!("    <release>{}</release>\n", r));
    }
    x.push_str("    <versions>\n");
    for v in versions {
        x.push_str(&format!("      <version>{}</version>\n", v));
    }
    x.push_str("    </versions>\n");
    x.push_str(&format!("    <lastUpdated>{}</lastUpdated>\n", last_updated));
    x.push_str("  </versioning>\n");
    x.push_str("</metadata>\n");
    x
}

// ── SNAPSHOT filename parser ────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotInfo {
    pub timestamp: String,
    pub build_number: u32,
}

impl SnapshotInfo {
    /// Parse a unique-snapshot Maven artifact filename of the form
    /// `{artifact}-{base}-{yyyyMMdd.HHmmss}-{buildnum}.{ext}`.
    /// Returns None if the filename is not a unique-snapshot file.
    pub fn from_filename(filename: &str) -> Option<Self> {
        // Strip extension.
        let dot = filename.rfind('.')?;
        let stem = &filename[..dot];
        // Build number is the last `-` segment if it's numeric.
        let (rest, build_str) = stem.rsplit_once('-')?;
        let build_number: u32 = build_str.parse().ok()?;
        // Timestamp is the previous `-` segment with format `dddddddd.dddddd`.
        let (_, ts) = rest.rsplit_once('-')?;
        if ts.len() == 15 && ts.as_bytes()[8] == b'.'
            && ts[..8].chars().all(|c| c.is_ascii_digit())
            && ts[9..].chars().all(|c| c.is_ascii_digit())
        {
            Some(Self {
                timestamp: ts.to_string(),
                build_number,
            })
        } else {
            None
        }
    }
}

// ── Plugin trait ────────────────────────────────────────────────────────────

impl ArtifactsPlugin for MavenPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Maven
    }

    fn name(&self) -> &str {
        "pulp_maven"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["maven.artifact"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let sha256 = hex::encode(Sha256::digest(data));
        let coords = MavenCoordinates::from_path(relative_path);

        // If the payload is a .pom (XML), pull the structured GAV out of it.
        let mut pom: Option<MavenPom> = None;
        if relative_path.ends_with(".pom") {
            if let Ok(p) = parse_pom_xml(std::str::from_utf8(data).unwrap_or("")) {
                pom = Some(p);
            }
        }

        let metadata = if let Some(p) = &pom {
            serde_json::json!({
                "group_id": p.group_id,
                "artifact_id": p.artifact_id,
                "version": p.version,
                "packaging": p.packaging,
                "name": p.name,
                "parent": p.parent.as_ref().map(|x| serde_json::json!({
                    "group_id": x.group_id,
                    "artifact_id": x.artifact_id,
                    "version": x.version,
                })),
                "dependency_count": p.dependencies.len(),
            })
        } else if let Some(c) = &coords {
            serde_json::json!({
                "group_id": c.group_id,
                "artifact_id": c.artifact_id,
                "version": c.version,
                "classifier": c.classifier,
                "extension": c.extension,
                "is_snapshot": c.is_snapshot(),
            })
        } else {
            serde_json::json!({ "relative_path": relative_path })
        };

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
        // Group by (groupId, artifactId), collect versions, then emit a
        // maven-metadata.xml per group.
        use std::collections::BTreeMap;
        let mut groups: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
        for u in units {
            let gid = u.metadata.get("group_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let aid = u.metadata.get("artifact_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let ver = u.metadata.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string();
            if !gid.is_empty() && !aid.is_empty() && !ver.is_empty() {
                groups.entry((gid, aid)).or_default().push(ver);
            }
        }
        let now = chrono::Utc::now().format("%Y%m%d%H%M%S").to_string().parse::<u64>().unwrap_or(0);
        let docs: Vec<serde_json::Value> = groups
            .iter()
            .map(|((g, a), versions)| {
                let latest = versions.last().cloned();
                let release = versions
                    .iter()
                    .filter(|v| !v.contains("SNAPSHOT"))
                    .last()
                    .cloned();
                let xml = generate_maven_metadata_xml(
                    g,
                    a,
                    versions,
                    latest.as_deref(),
                    release.as_deref(),
                    now,
                );
                serde_json::json!({
                    "group_id": g,
                    "artifact_id": a,
                    "versions": versions,
                    "maven_metadata_xml": xml,
                })
            })
            .collect();
        serde_json::json!({ "maven_metadata": docs })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_maven_path_jar() {
        let coords = MavenCoordinates::from_path("com/example/mylib/1.0.0/mylib-1.0.0.jar").unwrap();
        assert_eq!(coords.group_id, "com.example");
        assert_eq!(coords.artifact_id, "mylib");
        assert_eq!(coords.version, "1.0.0");
        assert!(!coords.is_snapshot());
    }

    #[test]
    fn parse_maven_snapshot() {
        let coords =
            MavenCoordinates::from_path("org/acme/service/2.0.0-SNAPSHOT/service-2.0.0-SNAPSHOT.jar")
                .unwrap();
        assert!(coords.is_snapshot());
    }

    #[test]
    fn local_name_strips_ns() {
        assert_eq!(local_name("pom:project"), "project");
        assert_eq!(local_name("project"), "project");
    }
}
