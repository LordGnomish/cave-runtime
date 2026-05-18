// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_rpm — RPM package content plugin.
//!
//! Implements a real RPM v3 binary header reader (Lead + Header), with
//! convenience accessors for the well-known NEVRA tags. Reference:
//! RPM Reference Manual ch. 5 (Package Format).
//!
//! Upstream parity: pulp/pulp_rpm `pulp_rpm/app/models.py` —
//! we surface the same NEVRA fields (name/epoch/version/release/arch)
//! plus a structured repomd metadata generator.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};

pub struct RpmPlugin;

// ── Lead (96 bytes) ─────────────────────────────────────────────────────────

const LEAD_MAGIC: [u8; 4] = [0xED, 0xAB, 0xEE, 0xDB];
const LEAD_SIZE: usize = 96;
const HEADER_MAGIC: [u8; 4] = [0x8E, 0xAD, 0xE8, 0x01];
const HEADER_PREFIX_SIZE: usize = 16;
const INDEX_ENTRY_SIZE: usize = 16;

// Well-known RPM tag numbers (from rpm/rpmtag.h).
pub const RPMTAG_NAME: u32 = 1000;
pub const RPMTAG_VERSION: u32 = 1001;
pub const RPMTAG_RELEASE: u32 = 1002;
pub const RPMTAG_EPOCH: u32 = 1003;
pub const RPMTAG_SUMMARY: u32 = 1004;
pub const RPMTAG_DESCRIPTION: u32 = 1005;
pub const RPMTAG_LICENSE: u32 = 1014;
pub const RPMTAG_GROUP: u32 = 1016;
pub const RPMTAG_URL: u32 = 1020;
pub const RPMTAG_OS: u32 = 1021;
pub const RPMTAG_ARCH: u32 = 1022;
pub const RPMTAG_SOURCERPM: u32 = 1044;

// RPM type IDs (rpm/rpmtypes.h).
pub const RPM_NULL_TYPE: u32 = 0;
pub const RPM_CHAR_TYPE: u32 = 1;
pub const RPM_INT8_TYPE: u32 = 2;
pub const RPM_INT16_TYPE: u32 = 3;
pub const RPM_INT32_TYPE: u32 = 4;
pub const RPM_INT64_TYPE: u32 = 5;
pub const RPM_STRING_TYPE: u32 = 6;
pub const RPM_BIN_TYPE: u32 = 7;
pub const RPM_STRING_ARRAY_TYPE: u32 = 8;
pub const RPM_I18NSTRING_TYPE: u32 = 9;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpmLead {
    pub major: u8,
    pub minor: u8,
    pub rpm_type: u16,
    pub archnum: u16,
    pub name: String,
    pub osnum: u16,
    pub signature_type: u16,
}

/// Parse the 96-byte RPM v3 Lead.
pub fn parse_rpm_lead(buf: &[u8]) -> Result<RpmLead, ArtifactsError> {
    if buf.len() < LEAD_SIZE {
        return Err(ArtifactsError::InvalidRequest("RPM Lead truncated".into()));
    }
    if buf[0..4] != LEAD_MAGIC {
        return Err(ArtifactsError::InvalidRequest("bad RPM Lead magic".into()));
    }
    let major = buf[4];
    let minor = buf[5];
    let rpm_type = u16::from_be_bytes([buf[6], buf[7]]);
    let archnum = u16::from_be_bytes([buf[8], buf[9]]);
    let name_buf = &buf[10..76];
    let name_end = name_buf.iter().position(|&b| b == 0).unwrap_or(name_buf.len());
    let name = String::from_utf8_lossy(&name_buf[..name_end]).to_string();
    let osnum = u16::from_be_bytes([buf[76], buf[77]]);
    let signature_type = u16::from_be_bytes([buf[78], buf[79]]);
    // bytes 80..96 reserved
    Ok(RpmLead {
        major,
        minor,
        rpm_type,
        archnum,
        name,
        osnum,
        signature_type,
    })
}

// ── Header (variable size) ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RpmIndexEntry {
    pub tag: u32,
    pub type_id: u32,
    pub offset: u32,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct RpmHeader {
    pub entries: Vec<RpmIndexEntry>,
    pub store: Vec<u8>,
}

impl RpmHeader {
    /// Return a string-typed tag (RPM_STRING_TYPE or RPM_I18NSTRING_TYPE first slot).
    pub fn string_tag(&self, tag: u32) -> Option<String> {
        let e = self.entries.iter().find(|e| e.tag == tag)?;
        if e.type_id != RPM_STRING_TYPE && e.type_id != RPM_I18NSTRING_TYPE {
            return None;
        }
        let start = e.offset as usize;
        if start >= self.store.len() {
            return None;
        }
        let rest = &self.store[start..];
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        Some(String::from_utf8_lossy(&rest[..end]).to_string())
    }

    /// Return a 32-bit integer tag (RPM_INT32_TYPE, first slot).
    pub fn int_tag(&self, tag: u32) -> Option<u32> {
        let e = self.entries.iter().find(|e| e.tag == tag)?;
        if e.type_id != RPM_INT32_TYPE {
            return None;
        }
        let start = e.offset as usize;
        if start + 4 > self.store.len() {
            return None;
        }
        Some(u32::from_be_bytes([
            self.store[start],
            self.store[start + 1],
            self.store[start + 2],
            self.store[start + 3],
        ]))
    }
}

/// Parse an RPM header (signature or main).
pub fn parse_rpm_header(buf: &[u8]) -> Result<RpmHeader, ArtifactsError> {
    if buf.len() < HEADER_PREFIX_SIZE {
        return Err(ArtifactsError::InvalidRequest("RPM header truncated".into()));
    }
    if buf[0..4] != HEADER_MAGIC {
        return Err(ArtifactsError::InvalidRequest("bad RPM header magic".into()));
    }
    // bytes 4..7 reserved, byte 7 is version-ish
    let n_entries = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]) as usize;
    let store_size = u32::from_be_bytes([buf[12], buf[13], buf[14], buf[15]]) as usize;
    let entries_size = n_entries * INDEX_ENTRY_SIZE;
    let total = HEADER_PREFIX_SIZE + entries_size + store_size;
    if buf.len() < total {
        return Err(ArtifactsError::InvalidRequest(
            "RPM header: entries+store extend past buffer".into(),
        ));
    }
    let mut entries = Vec::with_capacity(n_entries);
    let mut i = HEADER_PREFIX_SIZE;
    for _ in 0..n_entries {
        let tag = u32::from_be_bytes([buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]);
        let type_id = u32::from_be_bytes([buf[i + 4], buf[i + 5], buf[i + 6], buf[i + 7]]);
        let offset = u32::from_be_bytes([buf[i + 8], buf[i + 9], buf[i + 10], buf[i + 11]]);
        let count = u32::from_be_bytes([buf[i + 12], buf[i + 13], buf[i + 14], buf[i + 15]]);
        entries.push(RpmIndexEntry {
            tag,
            type_id,
            offset,
            count,
        });
        i += INDEX_ENTRY_SIZE;
    }
    let store = buf[i..i + store_size].to_vec();
    Ok(RpmHeader { entries, store })
}

/// Parse the full NEVRA tuple from a Header.
pub fn nevra_from_header(h: &RpmHeader) -> (String, String, String, String, String) {
    let name = h.string_tag(RPMTAG_NAME).unwrap_or_default();
    let version = h.string_tag(RPMTAG_VERSION).unwrap_or_default();
    let release = h.string_tag(RPMTAG_RELEASE).unwrap_or_default();
    let arch = h.string_tag(RPMTAG_ARCH).unwrap_or_default();
    let epoch = h
        .int_tag(RPMTAG_EPOCH)
        .map(|n| n.to_string())
        .or_else(|| h.string_tag(RPMTAG_EPOCH))
        .unwrap_or_else(|| "0".to_string());
    (name, epoch, version, release, arch)
}

// ── Plugin ───────────────────────────────────────────────────────────────────

impl ArtifactsPlugin for RpmPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Rpm
    }

    fn name(&self) -> &str {
        "pulp_rpm"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["rpm.package", "rpm.advisory", "rpm.modulemd", "rpm.repo_metadata_file"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let (mut name, mut version, mut release, mut arch) = parse_rpm_filename(filename);
        let mut epoch = "0".to_string();
        let mut summary: Option<String> = None;
        let mut description: Option<String> = None;
        let mut license: Option<String> = None;

        if data.len() >= LEAD_SIZE && data[0..4] == LEAD_MAGIC {
            // Lead is fixed-size; signature header starts immediately after.
            // It is 8-byte aligned in real RPMs; for our parser we just attempt
            // to find the FIRST header magic after the lead, then the MAIN
            // header magic after the signature header.
            if let Some(sig_start) = find_header(&data[LEAD_SIZE..]) {
                let sig_abs = LEAD_SIZE + sig_start;
                if let Ok(sig_hdr) = parse_rpm_header(&data[sig_abs..]) {
                    // Move past signature header (rounded up to 8 bytes per RPM ABI).
                    let sig_total = HEADER_PREFIX_SIZE
                        + sig_hdr.entries.len() * INDEX_ENTRY_SIZE
                        + sig_hdr.store.len();
                    let mut after_sig = sig_abs + sig_total;
                    let pad = (8 - (after_sig % 8)) % 8;
                    after_sig += pad;
                    if let Some(main_off) = find_header(&data[after_sig..]) {
                        let main_abs = after_sig + main_off;
                        if let Ok(main) = parse_rpm_header(&data[main_abs..]) {
                            let (n, e, v, r, a) = nevra_from_header(&main);
                            if !n.is_empty() {
                                name = n;
                            }
                            if !v.is_empty() {
                                version = v;
                            }
                            if !r.is_empty() {
                                release = r;
                            }
                            if !a.is_empty() {
                                arch = a;
                            }
                            epoch = e;
                            summary = main.string_tag(RPMTAG_SUMMARY);
                            description = main.string_tag(RPMTAG_DESCRIPTION);
                            license = main.string_tag(RPMTAG_LICENSE);
                        }
                    }
                }
            }
        }

        let sha256 = hex::encode(Sha256::digest(data));
        let mut md = serde_json::json!({
            "name": name,
            "version": version,
            "release": release,
            "arch": arch,
            "epoch": epoch,
            "filename": filename,
            "sha256": sha256,
        });
        if let Some(s) = summary {
            md["summary"] = serde_json::Value::String(s);
        }
        if let Some(d) = description {
            md["description"] = serde_json::Value::String(d);
        }
        if let Some(l) = license {
            md["license"] = serde_json::Value::String(l);
        }
        let mut unit = ContentUnit::new(PluginType::Rpm, md);
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
        // Emit a structured repomd that mirrors repomd.xml + primary.xml shapes.
        let primary: Vec<serde_json::Value> = units
            .iter()
            .map(|u| {
                serde_json::json!({
                    "name":    u.metadata.get("name").cloned().unwrap_or(serde_json::Value::Null),
                    "epoch":   u.metadata.get("epoch").cloned().unwrap_or(serde_json::Value::String("0".into())),
                    "version": u.metadata.get("version").cloned().unwrap_or(serde_json::Value::Null),
                    "release": u.metadata.get("release").cloned().unwrap_or(serde_json::Value::Null),
                    "arch":    u.metadata.get("arch").cloned().unwrap_or(serde_json::Value::Null),
                    "summary": u.metadata.get("summary").cloned().unwrap_or(serde_json::Value::Null),
                    "checksum": {
                        "type": "sha256",
                        "value": u.sha256.clone().unwrap_or_default(),
                    },
                    "size": {
                        "package": u.size.unwrap_or(0),
                    },
                    "location": {
                        "href": u.relative_path.clone().unwrap_or_default(),
                    },
                })
            })
            .collect();
        // Also render a minimal primary.xml so it's directly publishable.
        let mut primary_xml = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<metadata xmlns=\"http://linux.duke.edu/metadata/common\" packages=\"",
        );
        primary_xml.push_str(&units.len().to_string());
        primary_xml.push_str("\">\n");
        for p in &primary {
            primary_xml.push_str("  <package type=\"rpm\">\n");
            for field in ["name", "arch", "summary"] {
                if let Some(v) = p.get(field).and_then(|x| x.as_str()) {
                    primary_xml.push_str(&format!(
                        "    <{field}>{}</{field}>\n",
                        xml_escape(v)
                    ));
                }
            }
            if let (Some(epoch), Some(ver), Some(rel)) = (
                p.get("epoch").and_then(|x| x.as_str()),
                p.get("version").and_then(|x| x.as_str()),
                p.get("release").and_then(|x| x.as_str()),
            ) {
                primary_xml.push_str(&format!(
                    "    <version epoch=\"{}\" ver=\"{}\" rel=\"{}\"/>\n",
                    xml_escape(epoch),
                    xml_escape(ver),
                    xml_escape(rel),
                ));
            }
            if let Some(href) = p.pointer("/location/href").and_then(|x| x.as_str()) {
                primary_xml.push_str(&format!(
                    "    <location href=\"{}\"/>\n",
                    xml_escape(href)
                ));
            }
            if let Some(sha) = p.pointer("/checksum/value").and_then(|x| x.as_str()) {
                primary_xml.push_str(&format!(
                    "    <checksum type=\"sha256\" pkgid=\"YES\">{}</checksum>\n",
                    xml_escape(sha)
                ));
            }
            primary_xml.push_str("  </package>\n");
        }
        primary_xml.push_str("</metadata>\n");

        serde_json::json!({
            "repomd": {
                "revision": chrono::Utc::now().timestamp(),
                "packages": units.len(),
            },
            "primary": primary,
            "primary_xml": primary_xml,
        })
    }
}

fn find_header(buf: &[u8]) -> Option<usize> {
    if buf.len() < 4 {
        return None;
    }
    // Common case: header begins exactly at offset 0 (8-byte aligned in real
    // RPMs after the lead; we tolerate skipping up to 8 bytes of padding).
    for off in 0..=buf.len().saturating_sub(4).min(16) {
        if buf[off..off + 4] == HEADER_MAGIC {
            return Some(off);
        }
    }
    None
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn parse_rpm_filename(filename: &str) -> (String, String, String, String) {
    let stem = filename.strip_suffix(".rpm").unwrap_or(filename);
    let (rest, arch) = stem.rsplit_once('.').unwrap_or((stem, "noarch"));
    let (rest2, release) = rest.rsplit_once('-').unwrap_or((rest, "1"));
    let (name, version) = rest2.rsplit_once('-').unwrap_or((rest2, "0"));
    (
        name.to_string(),
        version.to_string(),
        release.to_string(),
        arch.to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_rpm_name_from_filename() {
        let (name, ver, rel, arch) = parse_rpm_filename("bash-5.1.8-6.el9.x86_64.rpm");
        assert_eq!(name, "bash");
        assert_eq!(ver, "5.1.8");
        assert_eq!(rel, "6.el9");
        assert_eq!(arch, "x86_64");
    }

    #[test]
    fn xml_escape_basic() {
        assert_eq!(xml_escape("a&b<c>d"), "a&amp;b&lt;c&gt;d");
    }
}
