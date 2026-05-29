// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy@8a3177a pkg/detector/library/driver.go
//! CVE-to-installed-package matching.
//!
//! Two phases:
//!
//! 1. **PURL parse** → `PackageRef { ecosystem, name, version }` (subset of
//!    `package-url` spec sufficient for our scanners — does not implement full
//!    pURL quoting/escaping).
//! 2. **Version compare** against [`Advisory::affected_version`] / `fixed_version`.
//!    Supports semver-style operators (`<`, `<=`, `>=`, `=`, `*`) and OR (`||`)
//!    and AND (`,`) joins, plus dpkg-style exact list (whitespace-separated).
//!
//! This is intentionally smaller than upstream `pkg/version/version.go` —
//! we omit pre-release ordering nuance (the upstream uses go-version per ecosystem).

use crate::go_pseudo::{is_pseudo_version, normalize_pseudo_version, pseudo_version_cmp};
use crate::{Advisory, OsAdvisoryDb, Result};

/// Minimal pURL parse — `pkg:<eco>/<name>@<version>`.
///
/// Examples accepted:
/// * `pkg:deb/debian/openssl@1.1.1n-0+deb11u3`
/// * `pkg:npm/lodash@4.17.20`
/// * `pkg:cargo/serde@1.0.150`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageRef {
    pub ecosystem: String,
    pub name: String,
    pub version: String,
}

impl PackageRef {
    pub fn parse_purl(purl: &str) -> Option<Self> {
        let rest = purl.strip_prefix("pkg:")?;
        let (eco_path, version) = rest.split_once('@')?;
        let mut parts = eco_path.splitn(2, '/');
        let eco = parts.next()?;
        let name = parts.next()?;
        // For OS packages, debian/openssl style — keep both segments in name.
        Some(Self {
            ecosystem: eco.to_string(),
            name: name.to_string(),
            version: version.to_string(),
        })
    }
}

/// Compare two version strings element-wise.
///
/// Splits on `.` and `-`, compares numerically when both sides parse as u64,
/// otherwise lexically. Returns -1/0/1.
pub fn version_cmp(a: &str, b: &str) -> i8 {
    let pa: Vec<&str> = a
        .split(|c: char| c == '.' || c == '-' || c == '+')
        .collect();
    let pb: Vec<&str> = b
        .split(|c: char| c == '.' || c == '-' || c == '+')
        .collect();
    let n = pa.len().max(pb.len());
    for i in 0..n {
        let ai = pa.get(i).copied().unwrap_or("0");
        let bi = pb.get(i).copied().unwrap_or("0");
        match (ai.parse::<u64>(), bi.parse::<u64>()) {
            (Ok(x), Ok(y)) => {
                if x < y {
                    return -1;
                }
                if x > y {
                    return 1;
                }
            }
            _ => {
                if ai < bi {
                    return -1;
                }
                if ai > bi {
                    return 1;
                }
            }
        }
    }
    0
}

/// Does `version` satisfy the constraint expression `spec`?
///
/// Grammar:
/// * `*` → always true.
/// * empty → always true.
/// * `<X`, `<=X`, `>X`, `>=X`, `=X`, `==X`, bare `X` (treated as `=X`).
/// * `A , B` → AND.
/// * `A || B` → OR (lower precedence than `,`).
/// * `dpkg-style: "1.0 2.0 3.0"` → equality against any (whitespace list of bare versions).
pub fn version_satisfies(version: &str, spec: &str) -> bool {
    let s = spec.trim();
    if s.is_empty() || s == "*" {
        return true;
    }
    // OR split
    if let Some((l, r)) = s.split_once("||") {
        return version_satisfies(version, l) || version_satisfies(version, r);
    }
    // AND split
    if let Some((l, r)) = s.split_once(',') {
        return version_satisfies(version, l) && version_satisfies(version, r);
    }
    // Whitespace-separated dpkg list — only when there's no operator at all.
    if !s.starts_with(['<', '>', '=']) && s.contains(char::is_whitespace) {
        return s.split_whitespace().any(|v| version_cmp(version, v) == 0);
    }
    let (op, rhs) = parse_op(s);
    let cmp = version_cmp(version, rhs);
    match op {
        Op::Lt => cmp < 0,
        Op::Le => cmp <= 0,
        Op::Gt => cmp > 0,
        Op::Ge => cmp >= 0,
        Op::Eq => cmp == 0,
    }
}

enum Op {
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
}

fn parse_op(s: &str) -> (Op, &str) {
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("<=") {
        (Op::Le, rest.trim())
    } else if let Some(rest) = s.strip_prefix(">=") {
        (Op::Ge, rest.trim())
    } else if let Some(rest) = s.strip_prefix("==") {
        (Op::Eq, rest.trim())
    } else if let Some(rest) = s.strip_prefix('<') {
        (Op::Lt, rest.trim())
    } else if let Some(rest) = s.strip_prefix('>') {
        (Op::Gt, rest.trim())
    } else if let Some(rest) = s.strip_prefix('=') {
        (Op::Eq, rest.trim())
    } else {
        (Op::Eq, s)
    }
}

/// Apply a pkg ref against a DB, returning each Advisory whose
/// `affected_version` matches and whose `fixed_version` does NOT.
///
/// For `go` / `golang` ecosystem PURLs, use [`match_purl_go`] instead, which
/// applies pseudo-version-aware comparison.
pub fn match_purl<D: OsAdvisoryDb + ?Sized>(db: &D, purl: &str) -> Result<Vec<Advisory>> {
    let r = match PackageRef::parse_purl(purl) {
        Some(r) => r,
        None => return Ok(Vec::new()),
    };
    let all = db.advisories_for_pkg(&r.ecosystem, &r.name)?;
    Ok(all
        .into_iter()
        .filter(|a| {
            if !a.affected_version.is_empty() && !version_satisfies(&r.version, &a.affected_version)
            {
                return false;
            }
            if !a.fixed_version.is_empty() && version_cmp(&r.version, &a.fixed_version) >= 0 {
                return false;
            }
            true
        })
        .collect())
}

/// Go-module-aware PURL matcher.
///
/// Behaves like [`match_purl`] for regular version strings but uses
/// [`pseudo_version_cmp`] when either the installed version or the advisory's
/// `fixed_version` is a Go pseudo-version.
///
/// Accepts both `pkg:golang/...` and `pkg:go/...` PURLs.
pub fn match_purl_go<D: OsAdvisoryDb + ?Sized>(db: &D, purl: &str) -> Result<Vec<Advisory>> {
    let r = match PackageRef::parse_purl(purl) {
        Some(r) => r,
        None => return Ok(Vec::new()),
    };

    // Normalise the ecosystem key: trivy-db stores Go advisories under "go".
    let eco = normalise_go_eco(&r.ecosystem);

    let all = db.advisories_for_pkg(eco, &r.name)?;
    Ok(all
        .into_iter()
        .filter(|a| {
            // Affected-version check (semver range).
            if !a.affected_version.is_empty()
                && !version_satisfies(&r.version, &a.affected_version)
            {
                return false;
            }
            // Fixed-version check with pseudo-version awareness.
            if !a.fixed_version.is_empty() {
                let cmp = go_version_cmp(&r.version, &a.fixed_version);
                if cmp >= 0 {
                    // Installed version >= fixed → already patched.
                    return false;
                }
            }
            true
        })
        .collect())
}

/// Compare two Go module version strings using pseudo-version-aware ordering.
///
/// Delegates to [`pseudo_version_cmp`] if either side is a pseudo-version,
/// otherwise falls back to the generic [`version_cmp`].
pub fn go_version_cmp(a: &str, b: &str) -> i8 {
    if is_pseudo_version(a) || is_pseudo_version(b) {
        let r = pseudo_version_cmp(a, b);
        if r < 0 {
            -1
        } else if r > 0 {
            1
        } else {
            0
        }
    } else {
        version_cmp(a, b)
    }
}

/// Normalise a PURL ecosystem component for the Go module ecosystem.
///
/// Both `golang` and `go` are used in the wild; trivy-db keys on `go`.
fn normalise_go_eco(eco: &str) -> &str {
    if eco.eq_ignore_ascii_case("golang") {
        "go"
    } else {
        eco
    }
}
