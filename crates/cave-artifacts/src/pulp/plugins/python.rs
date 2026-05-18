// SPDX-License-Identifier: AGPL-3.0-or-later
//! pulp_python — Python Package Index (PyPI) content plugin.
//!
//! Implements:
//! - PEP 440 version parser + total order (`parse_pep440`, `Pep440Version`).
//! - PEP 503 simple-index name normalization (`normalize_pep503`).
//! - RFC 822 PKG-INFO / METADATA field reader with continuation lines
//!   (`parse_metadata_fields`).
//!
//! Upstream parity targets:
//! - pulp/pulp_python `pulp_python/app/models.py` (Python package model).
//! - PyPA `packaging` library — `packaging.version.Version` + `packaging.utils.canonicalize_name`.

use crate::pulp::error::ArtifactsError;
use crate::pulp::models::{ContentUnit, PluginType, RepositoryVersion};
use crate::pulp::plugin::ArtifactsPlugin;
use sha2::{Digest, Sha256};
use std::cmp::Ordering;

pub struct PythonPlugin;

// ── PEP 440 ─────────────────────────────────────────────────────────────────

/// Pre-release tag from PEP 440 §pre-releases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PreRelease {
    Alpha(u64),
    Beta(u64),
    Rc(u64),
}

impl PreRelease {
    fn rank(&self) -> u8 {
        match self {
            PreRelease::Alpha(_) => 0,
            PreRelease::Beta(_) => 1,
            PreRelease::Rc(_) => 2,
        }
    }
    fn num(&self) -> u64 {
        match self {
            PreRelease::Alpha(n) | PreRelease::Beta(n) | PreRelease::Rc(n) => *n,
        }
    }
}

impl Ord for PreRelease {
    fn cmp(&self, other: &Self) -> Ordering {
        self.rank().cmp(&other.rank()).then(self.num().cmp(&other.num()))
    }
}
impl PartialOrd for PreRelease {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parsed PEP 440 version `[N!]N(.N)*[{a|b|rc}N][.postN][.devN][+local]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pep440Version {
    pub epoch: u64,
    pub release: Vec<u64>,
    pub pre: Option<PreRelease>,
    pub post: Option<u64>,
    pub dev: Option<u64>,
    pub local: Option<String>,
}

impl Ord for Pep440Version {
    fn cmp(&self, other: &Self) -> Ordering {
        // PEP 440 §6: epoch ▶ release ▶ pre/dev/post ▶ local.
        let by_epoch = self.epoch.cmp(&other.epoch);
        if by_epoch != Ordering::Equal {
            return by_epoch;
        }
        // Release: lexicographic, padding shorter with zeros.
        let max = self.release.len().max(other.release.len());
        for i in 0..max {
            let a = self.release.get(i).copied().unwrap_or(0);
            let b = other.release.get(i).copied().unwrap_or(0);
            match a.cmp(&b) {
                Ordering::Equal => continue,
                ord => return ord,
            }
        }
        // Same release. Order: dev-only < pre-only < final < post.
        // Encode each as a 4-tuple key (final-flag, pre, post, dev).
        let key = |v: &Pep440Version| -> (i8, i64, i64, i64) {
            // bucket: -2 dev (no pre, no post, dev), -1 pre-release, 0 final, 1 post.
            let bucket = if v.post.is_some() {
                1
            } else if v.pre.is_some() {
                -1
            } else if v.dev.is_some() {
                -2
            } else {
                0
            };
            (
                bucket,
                v.pre.as_ref().map(|p| (p.rank() as i64) * 1_000_000 + p.num() as i64).unwrap_or(-1),
                v.post.map(|n| n as i64).unwrap_or(-1),
                v.dev.map(|n| n as i64).unwrap_or(-1),
            )
        };
        let ka = key(self);
        let kb = key(other);
        ka.cmp(&kb)
        // Local segments deliberately ignored for cross-equality ordering here;
        // PEP 440 says local-bearing versions compare greater than non-local
        // ones of the same public version. Equality is fine for our use.
    }
}
impl PartialOrd for Pep440Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Parse a PEP 440 public version string.
///
/// Rejects junk (no silent success on input like `not-a-version`).
pub fn parse_pep440(input: &str) -> Result<Pep440Version, ArtifactsError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(ArtifactsError::InvalidRequest("empty version".into()));
    }
    // Split on first '+' → public vs local.
    let (public, local) = match s.split_once('+') {
        Some((p, l)) => (p, Some(l.to_string())),
        None => (s, None),
    };
    let bytes = public.as_bytes();
    let mut i = 0usize;

    // Optional epoch: digits + '!'
    let epoch = {
        let start = i;
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            i += 1;
        }
        if i < bytes.len() && bytes[i] == b'!' {
            let e = public[start..i]
                .parse::<u64>()
                .map_err(|_| ArtifactsError::InvalidRequest("bad epoch".into()))?;
            i += 1;
            e
        } else {
            // No epoch; rewind so the digits become part of release.
            i = start;
            0
        }
    };

    // Release: N(.N)*
    let release_start = i;
    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
        i += 1;
    }
    if release_start == i {
        return Err(ArtifactsError::InvalidRequest("missing release segment".into()));
    }
    let release_str = &public[release_start..i];
    let release: Vec<u64> = release_str
        .split('.')
        .filter(|p| !p.is_empty())
        .map(|p| p.parse::<u64>())
        .collect::<Result<_, _>>()
        .map_err(|_| ArtifactsError::InvalidRequest(format!("bad release '{}'", release_str)))?;
    if release.is_empty() {
        return Err(ArtifactsError::InvalidRequest("missing release segment".into()));
    }

    // Pre-release: optional separator (`.` `_` `-` or none) + tag + optional number
    let mut pre: Option<PreRelease> = None;
    let mut post: Option<u64> = None;
    let mut dev: Option<u64> = None;

    // Helper to greedily consume optional `.`/`-`/`_` separators.
    let skip_sep = |idx: &mut usize, b: &[u8]| {
        while *idx < b.len() && matches!(b[*idx], b'.' | b'-' | b'_') {
            *idx += 1;
        }
    };
    let take_uint = |idx: &mut usize, b: &[u8]| -> Option<u64> {
        let start = *idx;
        while *idx < b.len() && b[*idx].is_ascii_digit() {
            *idx += 1;
        }
        if *idx > start {
            std::str::from_utf8(&b[start..*idx]).ok()?.parse().ok()
        } else {
            None
        }
    };

    // Try pre-release tag.
    let try_match = |rest: &str| -> Option<(&'static str, usize)> {
        // Longest match first.
        for &(tok, normalized) in &[
            ("alpha", "a"),
            ("beta", "b"),
            ("preview", "rc"),
            ("pre", "rc"),
            ("rc", "rc"),
            ("a", "a"),
            ("b", "b"),
            ("c", "rc"),
        ] {
            if rest.len() >= tok.len() && rest[..tok.len()].eq_ignore_ascii_case(tok) {
                return Some((normalized, tok.len()));
            }
        }
        None
    };

    skip_sep(&mut i, bytes);
    if i < bytes.len() {
        if let Some((norm, used)) = try_match(&public[i..]) {
            i += used;
            // Optional separator + number.
            let save = i;
            skip_sep(&mut i, bytes);
            let n = take_uint(&mut i, bytes).unwrap_or(0);
            if save == i && n == 0 {
                // No number, no separator consumed -> n=0 stays.
            }
            pre = Some(match norm {
                "a" => PreRelease::Alpha(n),
                "b" => PreRelease::Beta(n),
                "rc" => PreRelease::Rc(n),
                _ => unreachable!(),
            });
        }
    }

    // Try .post / .postN
    skip_sep(&mut i, bytes);
    if i + 4 <= bytes.len() && public[i..i + 4].eq_ignore_ascii_case("post") {
        i += 4;
        skip_sep(&mut i, bytes);
        let n = take_uint(&mut i, bytes).unwrap_or(0);
        post = Some(n);
    }

    // Try .dev / .devN
    skip_sep(&mut i, bytes);
    if i + 3 <= bytes.len() && public[i..i + 3].eq_ignore_ascii_case("dev") {
        i += 3;
        skip_sep(&mut i, bytes);
        let n = take_uint(&mut i, bytes).unwrap_or(0);
        dev = Some(n);
    }

    if i != bytes.len() {
        return Err(ArtifactsError::InvalidRequest(format!(
            "trailing junk in '{}': {}",
            input,
            &public[i..]
        )));
    }

    Ok(Pep440Version {
        epoch,
        release,
        pre,
        post,
        dev,
        local,
    })
}

// ── PEP 503 ─────────────────────────────────────────────────────────────────

/// Canonicalize a distribution name per PEP 503: lowercase, runs of
/// `[-_.]+` collapsed to a single `-`.
pub fn normalize_pep503(name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_dash = false;
    for ch in lower.chars() {
        if matches!(ch, '-' | '_' | '.') {
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        } else {
            out.push(ch);
            last_dash = false;
        }
    }
    // Trim leading / trailing dashes that could result from edge inputs.
    out.trim_matches('-').to_string()
}

// ── METADATA / PKG-INFO ─────────────────────────────────────────────────────

/// Subset of PKG-INFO / METADATA fields we surface.
#[derive(Debug, Clone, Default)]
pub struct Metadata {
    pub metadata_version: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub summary: Option<String>,
    pub home_page: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub requires_python: Option<String>,
    pub requires_dist: Vec<String>,
    pub description: Option<String>,
}

/// Parse an RFC 822-style METADATA / PKG-INFO body. Supports continuation
/// lines (any line starting with whitespace continues the previous field).
/// Body after the first blank line is captured as `description`.
pub fn parse_metadata_fields(raw: &str) -> Result<Metadata, ArtifactsError> {
    let mut m = Metadata::default();
    let mut headers: Vec<(String, String)> = Vec::new();
    let mut body_start: Option<usize> = None;

    let lines: Vec<&str> = raw.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.is_empty() {
            body_start = Some(i + 1);
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // Continuation of the previous header.
            if let Some(last) = headers.last_mut() {
                last.1.push('\n');
                last.1.push_str(line.trim_start());
            } else {
                return Err(ArtifactsError::InvalidRequest(
                    "continuation line with no preceding header".into(),
                ));
            }
        } else if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        } else {
            return Err(ArtifactsError::InvalidRequest(format!(
                "malformed header line: {line}"
            )));
        }
        i += 1;
    }

    for (k, v) in &headers {
        match k.as_str() {
            "Metadata-Version" => m.metadata_version = Some(v.clone()),
            "Name" => m.name = Some(v.clone()),
            "Version" => m.version = Some(v.clone()),
            "Summary" => m.summary = Some(v.clone()),
            "Home-page" => m.home_page = Some(v.clone()),
            "Author" => m.author = Some(v.clone()),
            "License" => m.license = Some(v.clone()),
            "Requires-Python" => m.requires_python = Some(v.clone()),
            "Requires-Dist" => m.requires_dist.push(v.clone()),
            _ => {}
        }
    }

    if let Some(start) = body_start {
        let body = lines[start..].join("\n");
        let trimmed = body.trim();
        if !trimmed.is_empty() {
            m.description = Some(trimmed.to_string());
        }
    }
    Ok(m)
}

// ── Plugin trait ────────────────────────────────────────────────────────────

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

    fn parse_content(&self, data: &[u8], relative_path: &str) -> Result<ContentUnit, ArtifactsError> {
        let filename = relative_path.rsplit('/').next().unwrap_or(relative_path);
        let (name, version) = parse_python_filename(filename);
        let sha256 = hex::encode(Sha256::digest(data));
        // Normalize name for the simple-index URL slot per PEP 503.
        let canonical = normalize_pep503(&name);

        let mut unit = ContentUnit::new(
            PluginType::Python,
            serde_json::json!({
                "name": name,
                "name_canonical": canonical,
                "version": version,
                "filename": filename,
                "packagetype": if filename.ends_with(".whl") { "bdist_wheel" } else { "sdist" },
                "sha256_digest": sha256,
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
        // PEP 503 simple-index — grouped by canonical name.
        let mut packages: std::collections::BTreeMap<String, Vec<serde_json::Value>> =
            std::collections::BTreeMap::new();
        for unit in units {
            let canonical = unit
                .metadata
                .get("name_canonical")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
                .to_string();
            let filename = unit.metadata.get("filename").and_then(|v| v.as_str()).unwrap_or("");
            let sha256 = unit.sha256.clone().unwrap_or_default();
            packages.entry(canonical).or_default().push(serde_json::json!({
                "filename": filename,
                "url": format!("../../packages/{filename}#sha256={sha256}"),
                "digests": { "sha256": sha256 },
            }));
        }
        serde_json::json!({ "packages": packages })
    }
}

/// Parse `name` and `version` from a Python distribution filename.
/// Wheel: `{name}-{ver}-{python}-{abi}-{platform}.whl`.
/// Sdist: `{name}-{ver}.tar.gz` / `{name}-{ver}.zip`.
fn parse_python_filename(filename: &str) -> (String, String) {
    let stem = filename
        .strip_suffix(".whl")
        .or_else(|| filename.strip_suffix(".tar.gz"))
        .or_else(|| filename.strip_suffix(".zip"))
        .unwrap_or(filename);

    let parts: Vec<&str> = stem.splitn(3, '-').collect();
    let name = parts.first().copied().unwrap_or("unknown").replace('_', "-");
    let version = parts.get(1).copied().unwrap_or("0.0.0").to_string();
    (name, version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wheel_filename() {
        let (name, ver) = parse_python_filename("requests-2.31.0-py3-none-any.whl");
        assert_eq!(name, "requests");
        assert_eq!(ver, "2.31.0");
    }

    #[test]
    fn python_plugin_parse_content() {
        let plugin = PythonPlugin;
        let unit = plugin
            .parse_content(b"fake wheel data", "simple/Requests-2.31.0-py3-none-any.whl")
            .unwrap();
        assert_eq!(unit.metadata["name"], "Requests");
        assert_eq!(unit.metadata["name_canonical"], "requests");
        assert_eq!(unit.metadata["packagetype"], "bdist_wheel");
    }

    #[test]
    fn python_simple_index_groups_by_canonical() {
        let plugin = PythonPlugin;
        let ver = RepositoryVersion::new("/repo/", 1);
        let units = vec![
            plugin
                .parse_content(b"data1", "Requests-2.31.0-py3-none-any.whl")
                .unwrap(),
            plugin
                .parse_content(b"data2", "requests-2.32.0-py3-none-any.whl")
                .unwrap(),
        ];
        let meta = plugin.generate_metadata(&ver, &units);
        let arr = meta["packages"]["requests"].as_array().unwrap();
        assert_eq!(arr.len(), 2);
    }
}
