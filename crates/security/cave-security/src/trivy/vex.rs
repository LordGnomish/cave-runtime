// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! VEX — Vulnerability Exploitability eXchange statement filtering.
//!
//! Faithful in-crate line-port of:
//!   - aquasecurity/trivy `pkg/vex/vex.go` (`OpenVEX.Filter`, `Statement`,
//!     `Status`) — the experimental VEX result filter that drops detected
//!     vulnerabilities a VEX document marks `not_affected` or `fixed`.
//!   - openvex/go-vex `pkg/vex/vex.go` (`VEX.Matches`, `PurlMatches`),
//!     `pkg/vex/statement.go` (`Statement.Matches`, `SortStatements`),
//!     `pkg/vex/product.go` (`Product.Matches`), `pkg/vex/component.go`
//!     (`Component.Matches`) — the statement/product/component matching and
//!     "latest statement wins" sort ordering.
//!
//! This is a pure in-memory algorithm (no I/O, no network, no persistence):
//! given the list of vulnerabilities a scan detected and an OpenVEX document,
//! it returns the vulnerabilities that remain after applying the VEX
//! assertions. It was previously scope-cut under the
//! "SBOM attestation + VEX" skip; the *attestation* half (Cosign/Rekor) and
//! the CycloneDX-VEX BOM-Link parsing remain owned by cave-container-scan /
//! cave-sign, but the OpenVEX matcher/filter itself is genuinely in-crate
//! runtime logic and is ported here (TDD 2026-05-30).

use serde::{Deserialize, Serialize};

use super::scanner::VulnFinding;

/// VEX statement status. Port of trivy `pkg/vex/status.go`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NotAffected,
    Affected,
    Fixed,
    UnderInvestigation,
    Unknown,
}

impl Status {
    /// Parse from the OpenVEX wire string. Unknown strings → `Unknown`,
    /// mirroring trivy's tolerant handling.
    pub fn parse(s: &str) -> Status {
        match s {
            "not_affected" => Status::NotAffected,
            "affected" => Status::Affected,
            "fixed" => Status::Fixed,
            "under_investigation" => Status::UnderInvestigation,
            _ => Status::Unknown,
        }
    }
}

/// A single OpenVEX statement. Port of openvex/go-vex `pkg/vex/statement.go`
/// `Statement` (only the fields the filter consults: vulnerability name, the
/// affected products, the impact status, justification, and the per-statement
/// timestamp used for "latest wins" ordering).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VexStatement {
    /// `vulnerability.name` (e.g. a CVE id).
    pub vulnerability_id: String,
    /// Product identifiers (purls / ids) this statement applies to.
    pub products: Vec<String>,
    pub status: Status,
    #[serde(default)]
    pub justification: String,
    /// Statement timestamp (unix seconds). `None` falls back to the document
    /// timestamp in `SortStatements`.
    #[serde(default)]
    pub timestamp: Option<i64>,
}

impl VexStatement {
    /// Port of openvex/go-vex `pkg/vex/statement.go::(*Statement).Matches`.
    ///
    /// A statement matches when its vulnerability name equals `vuln` and at
    /// least one of its products matches `product`. (Subcomponents are not
    /// modeled here — trivy's OpenVEX filter calls `Matches(id, pkgRef, nil)`,
    /// i.e. always with an empty subcomponent list.)
    pub fn matches(&self, vuln: &str, product: &str) -> bool {
        if self.vulnerability_id != vuln {
            return false;
        }
        // subcomponents always empty in the trivy call site → product match
        // alone is sufficient.
        for p in &self.products {
            if product_matches(p, product) {
                return true;
            }
        }
        false
    }
}

/// Port of openvex/go-vex `pkg/vex/component.go::(*Component).Matches`
/// restricted to the id/purl case (no Identifiers map or Hashes here):
/// exact id equality, or — when the statement product is a purl — a
/// `PurlMatches` comparison.
fn product_matches(stmt_product: &str, pkg_ref: &str) -> bool {
    if stmt_product == pkg_ref && !stmt_product.is_empty() {
        return true;
    }
    if stmt_product.starts_with("pkg:") && PurlMatches(stmt_product, pkg_ref) {
        return true;
    }
    false
}

/// A vulnerability as seen by the VEX filter — the subset of trivy
/// `types.DetectedVulnerability` that `OpenVEX.Filter` reads: the
/// vulnerability id and the package reference (purl).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VexVulnerability {
    pub vulnerability_id: String,
    /// Package reference (`PkgRef`), typically a purl.
    pub pkg_ref: String,
}

impl From<&VulnFinding> for VexVulnerability {
    /// Build the VEX view of a scan finding, deriving a maven-style purl from
    /// the finding's ecosystem/package/version so VEX documents authored
    /// against purls line up with cave-security scan results.
    fn from(f: &VulnFinding) -> Self {
        let pkg_ref = if f.installed_version.is_empty() {
            format!("pkg:{}/{}", f.ecosystem, f.package_name)
        } else {
            format!(
                "pkg:{}/{}@{}",
                f.ecosystem, f.package_name, f.installed_version
            )
        };
        VexVulnerability {
            vulnerability_id: f.cve_id.clone(),
            pkg_ref,
        }
    }
}

/// An OpenVEX document — the subset trivy's `newOpenVEX` filter consumes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpenVexDocument {
    pub statements: Vec<VexStatement>,
    /// Document timestamp (unix seconds), used as the fallback in
    /// `SortStatements`.
    #[serde(default)]
    pub timestamp: Option<i64>,
}

impl OpenVexDocument {
    /// Port of openvex/go-vex `pkg/vex/vex.go::(*VEX).Matches`.
    ///
    /// Collects the statements that match `(vuln_id, product)`, iterating in
    /// reverse (preserving go-vex's reverse-append), then re-orders them with
    /// `sort_statements` so the *last* element is the statement that wins under
    /// the OpenVEX "newer statement overrides" rule.
    pub fn matches(&self, vuln_id: &str, product: &str) -> Vec<VexStatement> {
        let doc_ts = self.timestamp.unwrap_or(0);
        let mut matches: Vec<VexStatement> = Vec::new();
        // for i := len(statements)-1; i >= 0; i--
        for stmt in self.statements.iter().rev() {
            if stmt.matches(vuln_id, product) {
                matches.push(stmt.clone());
            }
        }
        sort_statements(&mut matches, doc_ts);
        matches
    }

    /// Port of trivy `pkg/vex/vex.go::(*OpenVEX).Filter`.
    ///
    /// Keeps a vulnerability unless the *latest* matching VEX statement marks
    /// it `not_affected` or `fixed`.
    pub fn filter(&self, vulns: &[VexVulnerability]) -> Vec<VexVulnerability> {
        vulns
            .iter()
            .filter(|vuln| {
                let stmts = self.matches(&vuln.vulnerability_id, &vuln.pkg_ref);
                if stmts.is_empty() {
                    return true;
                }
                // Take the latest statement for a given vulnerability and
                // product; a sequence of statements can be overridden by the
                // newer one.
                let stmt = &stmts[stmts.len() - 1];
                !(stmt.status == Status::NotAffected || stmt.status == Status::Fixed)
            })
            .cloned()
            .collect()
    }

    /// Convenience: filter a slice of scan `VulnFinding`s, returning the
    /// findings whose derived purl is not suppressed by the VEX document.
    pub fn filter_findings(&self, findings: &[VulnFinding]) -> Vec<VulnFinding> {
        findings
            .iter()
            .filter(|f| {
                let vv = VexVulnerability::from(*f);
                let stmts = self.matches(&vv.vulnerability_id, &vv.pkg_ref);
                if stmts.is_empty() {
                    return true;
                }
                let stmt = &stmts[stmts.len() - 1];
                !(stmt.status == Status::NotAffected || stmt.status == Status::Fixed)
            })
            .cloned()
            .collect()
    }
}

/// Port of openvex/go-vex `pkg/vex/statement.go::SortStatements`.
///
/// Stable sort: primarily by vulnerability name (ascending string compare);
/// for the same vulnerability, by effective timestamp ascending (statement
/// timestamp, or the document timestamp when the statement has none / is zero).
/// After this sort the last element for a given vulnerability is the newest.
fn sort_statements(stmts: &mut [VexStatement], document_timestamp: i64) {
    stmts.sort_by(|a, b| {
        let vuln_cmp = a.vulnerability_id.cmp(&b.vulnerability_id);
        if vuln_cmp != std::cmp::Ordering::Equal {
            return vuln_cmp;
        }
        let it = match a.timestamp {
            Some(t) if t != 0 => t,
            _ => document_timestamp,
        };
        let jt = match b.timestamp {
            Some(t) if t != 0 => t,
            _ => document_timestamp,
        };
        it.cmp(&jt)
    });
}

/// Minimal package-url parse for `PurlMatches`. Port of the fields go-vex's
/// `packageurl.FromString` exposes to the comparison: type, namespace, name,
/// version, and qualifiers. Format:
/// `pkg:TYPE/NAMESPACE/NAME@VERSION?QUALIFIERS`.
struct Purl {
    ptype: String,
    namespace: String,
    name: String,
    version: String,
    qualifiers: Vec<(String, String)>,
}

fn parse_purl(s: &str) -> Option<Purl> {
    let rest = s.strip_prefix("pkg:")?;
    // qualifiers
    let (head, qual_str) = match rest.split_once('?') {
        Some((h, q)) => (h, Some(q)),
        None => (rest, None),
    };
    // type is everything up to the first '/'
    let (ptype, after_type) = head.split_once('/')?;
    // version is after the last '@'
    let (path, version) = match after_type.rsplit_once('@') {
        Some((p, v)) => (p, v.to_string()),
        None => (after_type, String::new()),
    };
    // name is the last path segment; namespace is the rest joined by '/'
    let (namespace, name) = match path.rsplit_once('/') {
        Some((ns, n)) => (ns.to_string(), n.to_string()),
        None => (String::new(), path.to_string()),
    };
    let qualifiers = qual_str
        .map(|q| {
            q.split('&')
                .filter_map(|kv| kv.split_once('=').map(|(k, v)| (k.to_string(), v.to_string())))
                .collect()
        })
        .unwrap_or_default();
    Some(Purl {
        ptype: ptype.to_string(),
        namespace,
        name,
        version,
        qualifiers,
    })
}

/// Port of openvex/go-vex `pkg/vex/vex.go::PurlMatches`.
///
/// Two purls match when type, namespace and name are equal; a missing version
/// on `purl1` is a wildcard, but two differing non-empty versions fail; and
/// every qualifier present in `purl1` must be present (and equal) in `purl2`.
#[allow(non_snake_case)]
pub fn PurlMatches(purl1: &str, purl2: &str) -> bool {
    let (p1, p2) = match (parse_purl(purl1), parse_purl(purl2)) {
        (Some(a), Some(b)) => (a, b),
        _ => return false,
    };
    if p1.ptype != p2.ptype {
        return false;
    }
    if p1.namespace != p2.namespace {
        return false;
    }
    if p1.name != p2.name {
        return false;
    }
    if !p1.version.is_empty() && p2.version.is_empty() {
        return false;
    }
    if p1.version != p2.version && !p1.version.is_empty() && !p2.version.is_empty() {
        return false;
    }
    // All qualifiers in p1 must be in p2 to match.
    for (k, v1) in &p1.qualifiers {
        match p2.qualifiers.iter().find(|(k2, _)| k2 == k) {
            Some((_, v2)) if v2 == v1 => {}
            _ => return false,
        }
    }
    true
}
