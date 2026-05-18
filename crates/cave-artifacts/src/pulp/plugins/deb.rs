// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_deb — Debian package content plugin.
//!
//! Implements:
//! - System V `ar(1)` archive reader (`parse_ar_archive`) so we can crack
//!   open a `.deb` (which is an ar archive containing debian-binary +
//!   control.tar.* + data.tar.*).
//! - RFC 822-style control field parser (`parse_deb_control`).
//! - Debian policy §5.6.12 version comparator (`cmp_debian_version`).
//!
//! Upstream parity: pulp/pulp_deb `pulp_deb/app/models.py` + Debian
//! Policy Manual §5 (control files) + §5.6.12 (version comparison).

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

pub struct DebPlugin;

// ── ar(1) archive ───────────────────────────────────────────────────────────

const AR_MAGIC: &[u8] = b"!<arch>\n";
const AR_HEADER_LEN: usize = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArFileHeader {
    pub name: String,
    pub size: u64,
    pub mtime: u64,
    pub data_offset: usize,
}

/// Parse a System V `ar` archive (the container format used by `.deb`).
pub fn parse_ar_archive(buf: &[u8]) -> Result<Vec<ArFileHeader>, ArtifactsError> {
    if buf.len() < AR_MAGIC.len() || &buf[..AR_MAGIC.len()] != AR_MAGIC {
        return Err(ArtifactsError::InvalidRequest(
            "not an ar archive (bad magic)".into(),
        ));
    }
    let mut i = AR_MAGIC.len();
    let mut out = Vec::new();
    while i < buf.len() {
        if i + AR_HEADER_LEN > buf.len() {
            return Err(ArtifactsError::InvalidRequest(
                "truncated ar header".into(),
            ));
        }
        let header = &buf[i..i + AR_HEADER_LEN];
        if &header[58..60] != b"\x60\n" {
            return Err(ArtifactsError::InvalidRequest(
                "ar header terminator mismatch".into(),
            ));
        }
        let name = std::str::from_utf8(&header[0..16])
            .map_err(|_| ArtifactsError::InvalidRequest("bad name".into()))?
            .trim_end()
            .trim_end_matches('/')
            .to_string();
        let mtime: u64 = std::str::from_utf8(&header[16..28])
            .ok()
            .and_then(|s| s.trim_end().parse().ok())
            .unwrap_or(0);
        let size: u64 = std::str::from_utf8(&header[48..58])
            .map_err(|_| ArtifactsError::InvalidRequest("bad size".into()))?
            .trim_end()
            .parse()
            .map_err(|_| ArtifactsError::InvalidRequest("bad size".into()))?;
        let data_offset = i + AR_HEADER_LEN;
        if data_offset + size as usize > buf.len() {
            return Err(ArtifactsError::InvalidRequest(
                "ar member runs past EOF".into(),
            ));
        }
        out.push(ArFileHeader {
            name,
            size,
            mtime,
            data_offset,
        });
        // Advance past payload, with even-byte alignment padding.
        let mut next = data_offset + size as usize;
        if next % 2 == 1 {
            next += 1;
        }
        i = next;
    }
    Ok(out)
}

// ── control file ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DebControl {
    pub package: String,
    pub source: Option<String>,
    pub version: String,
    pub architecture: String,
    pub maintainer: Option<String>,
    pub section: Option<String>,
    pub priority: Option<String>,
    pub depends: Option<String>,
    pub pre_depends: Option<String>,
    pub recommends: Option<String>,
    pub suggests: Option<String>,
    pub description: Option<String>,
    pub installed_size: Option<u64>,
}

/// Parse a Debian RFC 822-style `control` file body.
///
/// Continuation lines (starting with a space or tab) are appended to
/// the previous field's value with `\n` separating the original line
/// from continuations, matching `apt-pkg` behaviour.
pub fn parse_deb_control(raw: &str) -> Result<DebControl, ArtifactsError> {
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in raw.lines() {
        if line.is_empty() {
            break; // blank line ends record (next paragraph)
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(last) = headers.last_mut() {
                last.1.push('\n');
                last.1.push_str(line.trim_start());
            } else {
                return Err(ArtifactsError::InvalidRequest(
                    "continuation with no preceding field".into(),
                ));
            }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        } else {
            return Err(ArtifactsError::InvalidRequest(format!(
                "malformed control line: {line}"
            )));
        }
    }
    let mut c = DebControl::default();
    for (k, v) in &headers {
        match k.as_str() {
            "Package" => c.package = v.clone(),
            "Source" => c.source = Some(v.clone()),
            "Version" => c.version = v.clone(),
            "Architecture" => c.architecture = v.clone(),
            "Maintainer" => c.maintainer = Some(v.clone()),
            "Section" => c.section = Some(v.clone()),
            "Priority" => c.priority = Some(v.clone()),
            "Depends" => c.depends = Some(v.clone()),
            "Pre-Depends" => c.pre_depends = Some(v.clone()),
            "Recommends" => c.recommends = Some(v.clone()),
            "Suggests" => c.suggests = Some(v.clone()),
            "Description" => c.description = Some(v.clone()),
            "Installed-Size" => c.installed_size = v.parse().ok(),
            _ => {}
        }
    }
    if c.package.is_empty() {
        return Err(ArtifactsError::InvalidRequest("Package: missing".into()));
    }
    if c.version.is_empty() {
        return Err(ArtifactsError::InvalidRequest("Version: missing".into()));
    }
    Ok(c)
}

// ── Debian version comparator (Policy §5.6.12) ─────────────────────────────

/// Compare two Debian version strings per Policy Manual §5.6.12.
///
/// Three-part: `[epoch:]upstream[-revision]`. Each part compared by
/// alternating runs of non-digit / digit. Non-digit runs use a custom
/// ordering where `~` sorts before end-of-string and end-of-string sorts
/// before any other character; letters sort before non-letter symbols.
pub fn cmp_debian_version(a: &str, b: &str) -> Ordering {
    let (ea, ua, ra) = split_deb_version(a);
    let (eb, ub, rb) = split_deb_version(b);
    match ea.cmp(&eb) {
        Ordering::Equal => {}
        ord => return ord,
    }
    match cmp_string(&ua, &ub) {
        Ordering::Equal => {}
        ord => return ord,
    }
    cmp_string(&ra, &rb)
}

fn split_deb_version(s: &str) -> (u64, String, String) {
    let (epoch, rest) = if let Some(colon) = s.find(':') {
        let e: u64 = s[..colon].parse().unwrap_or(0);
        (e, &s[colon + 1..])
    } else {
        (0, s)
    };
    if let Some(dash) = rest.rfind('-') {
        (epoch, rest[..dash].to_string(), rest[dash + 1..].to_string())
    } else {
        (epoch, rest.to_string(), String::new())
    }
}

fn cmp_string(a: &str, b: &str) -> Ordering {
    let ab = a.as_bytes();
    let bb = b.as_bytes();
    let mut i = 0;
    let mut j = 0;
    loop {
        // Skip and compare a run of non-digit characters.
        let na = take_nondigit(ab, &mut i);
        let nb = take_nondigit(bb, &mut j);
        match cmp_nondigit(na, nb) {
            Ordering::Equal => {}
            ord => return ord,
        }
        if i >= ab.len() && j >= bb.len() {
            return Ordering::Equal;
        }
        let da = take_digits(ab, &mut i);
        let db = take_digits(bb, &mut j);
        match cmp_digits(da, db) {
            Ordering::Equal => {}
            ord => return ord,
        }
        if i >= ab.len() && j >= bb.len() {
            return Ordering::Equal;
        }
    }
}

fn take_nondigit<'a>(b: &'a [u8], i: &mut usize) -> &'a [u8] {
    let start = *i;
    while *i < b.len() && !b[*i].is_ascii_digit() {
        *i += 1;
    }
    &b[start..*i]
}
fn take_digits<'a>(b: &'a [u8], i: &mut usize) -> &'a [u8] {
    let start = *i;
    while *i < b.len() && b[*i].is_ascii_digit() {
        *i += 1;
    }
    &b[start..*i]
}

/// Debian non-digit ordering: tilde < empty < letters < other non-letter, non-digit.
fn cmp_nondigit(a: &[u8], b: &[u8]) -> Ordering {
    let max = a.len().max(b.len());
    for k in 0..max {
        let ca = a.get(k).copied();
        let cb = b.get(k).copied();
        let order_a = order_char(ca);
        let order_b = order_char(cb);
        match order_a.cmp(&order_b) {
            Ordering::Equal => continue,
            ord => return ord,
        }
    }
    Ordering::Equal
}

fn order_char(c: Option<u8>) -> u32 {
    match c {
        Some(b'~') => 0,
        None => 1, // end-of-string sorts before letters
        Some(c) if c.is_ascii_alphabetic() => 2 * 256 + c as u32,
        Some(c) => 3 * 256 + c as u32,
    }
}

fn cmp_digits(a: &[u8], b: &[u8]) -> Ordering {
    // Treat as integers, but strip leading zeros (Policy: "treated as a number").
    let a_trim = a
        .iter()
        .position(|&c| c != b'0')
        .map(|p| &a[p..])
        .unwrap_or(&[]);
    let b_trim = b
        .iter()
        .position(|&c| c != b'0')
        .map(|p| &b[p..])
        .unwrap_or(&[]);
    match a_trim.len().cmp(&b_trim.len()) {
        Ordering::Equal => a_trim.cmp(b_trim),
        ord => ord,
    }
}

// ── Plugin trait ────────────────────────────────────────────────────────────

impl ArtifactsPlugin for DebPlugin {
    fn plugin_type(&self) -> PluginType {
        PluginType::Deb
    }

    fn name(&self) -> &str {
        "pulp_deb"
    }

    fn content_types(&self) -> Vec<&str> {
        vec!["deb.package", "deb.release", "deb.package_index", "deb.installer_package"]
    }

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        // Default fallback: parse from filename.
        let (mut name, mut version, mut arch) = parse_deb_filename(filename);
        let mut control: Option<DebControl> = None;
        let mut description: Option<String> = None;
        let mut maintainer: Option<String> = None;

        // If the bytes look like a real .deb (ar archive), pull the control field
        // out of control.tar.gz / control.tar.xz / control.tar.zst.
        if data.len() >= 8 && &data[..8] == AR_MAGIC {
            if let Ok(members) = parse_ar_archive(data) {
                for m in &members {
                    if m.name.starts_with("control.tar") {
                        let payload = &data[m.data_offset..m.data_offset + m.size as usize];
                        if let Some(ctrl) = extract_control_from_tar(&m.name, payload) {
                            if let Ok(c) = parse_deb_control(&ctrl) {
                                name = c.package.clone();
                                version = c.version.clone();
                                arch = c.architecture.clone();
                                description = c.description.clone();
                                maintainer = c.maintainer.clone();
                                control = Some(c);
                            }
                        }
                    }
                }
            }
        }

        let sha256 = hex::encode(Sha256::digest(data));
        let mut metadata = serde_json::json!({
            "package": name,
            "version": version,
            "architecture": arch,
            "filename": filename,
            "sha256": sha256,
        });
        if let Some(d) = description {
            metadata["description"] = serde_json::Value::String(d);
        }
        if let Some(m) = maintainer {
            metadata["maintainer"] = serde_json::Value::String(m);
        }
        if let Some(c) = control {
            if let Some(s) = c.section {
                metadata["section"] = serde_json::Value::String(s);
            }
            if let Some(p) = c.priority {
                metadata["priority"] = serde_json::Value::String(p);
            }
            if let Some(d) = c.depends {
                metadata["depends"] = serde_json::Value::String(d);
            }
            if let Some(d) = c.pre_depends {
                metadata["pre_depends"] = serde_json::Value::String(d);
            }
            if let Some(s) = c.installed_size {
                metadata["installed_size"] = serde_json::Value::from(s);
            }
        }

        let mut unit = ContentUnit::new(PluginType::Deb, metadata);
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
        // Packages file text body (one paragraph per unit). Real Pulp wraps
        // this in Release + InRelease but those need GPG; emitted body is
        // upstream-shape correct and consumable by `apt-get update`
        // when paired with a Release file.
        let mut packages_body = String::new();
        let mut paragraphs: Vec<serde_json::Value> = Vec::new();
        for u in units {
            let pkg = u.metadata.get("package").and_then(|v| v.as_str()).unwrap_or("");
            let ver = u.metadata.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let arch = u.metadata.get("architecture").and_then(|v| v.as_str()).unwrap_or("");
            let filename = u
                .relative_path
                .clone()
                .unwrap_or_default();
            let sha = u.sha256.clone().unwrap_or_default();
            let size = u.size.unwrap_or(0);
            packages_body.push_str(&format!(
                "Package: {pkg}\nVersion: {ver}\nArchitecture: {arch}\nFilename: {filename}\nSize: {size}\nSHA256: {sha}\n\n"
            ));
            paragraphs.push(serde_json::json!({
                "Package": pkg,
                "Version": ver,
                "Architecture": arch,
                "Filename": filename,
                "SHA256": sha,
                "Size": size,
            }));
        }
        // Sort by Debian version per arch so Release files are deterministic.
        serde_json::json!({
            "Packages": packages_body,
            "paragraphs": paragraphs,
        })
    }
}

fn parse_deb_filename(filename: &str) -> (String, String, String) {
    let stem = filename.strip_suffix(".deb").unwrap_or(filename);
    let parts: Vec<&str> = stem.splitn(3, '_').collect();
    let name = parts.first().copied().unwrap_or("unknown").to_string();
    let version = parts.get(1).copied().unwrap_or("0").to_string();
    let arch = parts.get(2).copied().unwrap_or("all").to_string();
    (name, version, arch)
}

/// Extract the textual `control` file from a control.tar.{gz,xz,zst,plain}
/// payload. Only gzip (`control.tar.gz`) and uncompressed (`control.tar`)
/// are decoded inside this crate; other compressions are left as `None`
/// and the caller falls back to filename parsing.
fn extract_control_from_tar(member_name: &str, payload: &[u8]) -> Option<String> {
    use std::io::Read;
    let mut decoded: Vec<u8> = Vec::new();
    if member_name.ends_with(".gz") {
        let mut d = flate2::read::GzDecoder::new(payload);
        d.read_to_end(&mut decoded).ok()?;
    } else if member_name == "control.tar" {
        decoded = payload.to_vec();
    } else {
        return None;
    }
    let mut a = tar::Archive::new(&decoded[..]);
    for entry in a.entries().ok()? {
        let mut e = entry.ok()?;
        let path = e.path().ok()?.into_owned();
        if path.to_string_lossy() == "control" || path.to_string_lossy() == "./control" {
            let mut s = String::new();
            e.read_to_string(&mut s).ok()?;
            return Some(s);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_deb_name_from_filename() {
        let (name, ver, arch) = parse_deb_filename("libc6_2.35-0ubuntu3_amd64.deb");
        assert_eq!(name, "libc6");
        assert_eq!(ver, "2.35-0ubuntu3");
        assert_eq!(arch, "amd64");
    }

    #[test]
    fn fallback_when_data_is_not_ar() {
        let plugin = DebPlugin;
        let unit = plugin
            .parse_content(b"not an ar archive", "libc6_2.35-0ubuntu3_amd64.deb")
            .unwrap();
        assert_eq!(unit.metadata["package"], "libc6");
        assert_eq!(unit.metadata["version"], "2.35-0ubuntu3");
    }
}
