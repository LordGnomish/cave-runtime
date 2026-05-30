// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! OpenVEX statement-evaluation matcher.
//!
//! Faithful in-memory line-port of aquasecurity/trivy (Apache-2.0)
//! `pkg/vex/openvex.go` (`OpenVEX.Filter` / `OpenVEX.NotAffected` /
//! `OpenVEX.Matches` / `findingStatus`) together with the
//! `filterVulnerabilities` driver in `pkg/vex/vex.go`, restricted to the
//! flat product-PURL match.
//!
//! Trivy applies VEX during *result assembly* (`pkg/vex` feeding
//! `pkg/result`), not during signing: a detected vulnerability is
//! suppressed when a VEX statement for the matching product PURL declares
//! it `not_affected` or `fixed`, taking the **latest** statement per
//! (vuln, product) because a newer statement overrides an older one
//! (cf. the OpenVEX spec). This module ports exactly that pure matcher.
//!
//! Out of scope (kept in sibling crates, mirroring Trivy's own split):
//!   * VEX document signing / cosign attestation -> cave-sign.
//!   * The SBOM dependency-tree `reachRoot` traversal in `vex.go`, which
//!     needs `pkg/sbom/core.BOM` construction -> cave-sbom. We port the
//!     direct product-level match Trivy uses for the leaf component.

use crate::models::Finding;

/// OpenVEX statement status — port of `openvex.Status`
/// (`github.com/openvex/go-vex/pkg/vex`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VexStatus {
    /// `not_affected` — the product is not affected by the vulnerability.
    NotAffected,
    /// `affected` — the product is affected.
    Affected,
    /// `fixed` — the vulnerability has been remediated.
    Fixed,
    /// `under_investigation` — assessment is still in progress.
    UnderInvestigation,
}

/// OpenVEX justification — port of `openvex.Justification`. Only the label
/// is carried through to the modified finding (Trivy stores it as a string).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VexJustification {
    ComponentNotPresent,
    VulnerableCodeNotPresent,
    VulnerableCodeNotInExecutePath,
    VulnerableCodeCannotBeControlledByAdversary,
    InlineMitigationsAlreadyExist,
    None,
}

impl VexJustification {
    /// Spec string form, matching the OpenVEX justification labels.
    pub fn as_str(&self) -> &'static str {
        match self {
            VexJustification::ComponentNotPresent => "component_not_present",
            VexJustification::VulnerableCodeNotPresent => "vulnerable_code_not_present",
            VexJustification::VulnerableCodeNotInExecutePath => {
                "vulnerable_code_not_in_execute_path"
            }
            VexJustification::VulnerableCodeCannotBeControlledByAdversary => {
                "vulnerable_code_cannot_be_controlled_by_adversary"
            }
            VexJustification::InlineMitigationsAlreadyExist => "inline_mitigations_already_exist",
            VexJustification::None => "",
        }
    }
}

/// Final status stamped on a finding after VEX evaluation — port of
/// `types.FindingStatus` (`pkg/types/finding.go`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingStatus {
    NotAffected,
    Fixed,
    UnderInvestigation,
    Unknown,
}

/// Port of `openvex.go::findingStatus` — maps a VEX status to the
/// `types.FindingStatus` recorded on the modified finding.
pub fn finding_status(status: VexStatus) -> FindingStatus {
    match status {
        VexStatus::NotAffected => FindingStatus::NotAffected,
        VexStatus::Fixed => FindingStatus::Fixed,
        VexStatus::UnderInvestigation => FindingStatus::UnderInvestigation,
        _ => FindingStatus::Unknown,
    }
}

/// A single OpenVEX statement — the subset of fields Trivy matches on.
///
/// `sub_components` mirrors the OpenVEX sub-component PURL list that
/// `OpenVEX.Matches` threads through to `vex.Matches(vulnID, productPURL,
/// []string{subComponentPURL})`.
#[derive(Debug, Clone)]
pub struct VexStatement {
    pub vuln_id: String,
    pub product_purl: String,
    pub sub_components: Vec<String>,
    pub status: VexStatus,
    pub justification: VexJustification,
}

/// A finding that VEX evaluation removed from the active result set,
/// recording why — port of `types.ModifiedFinding`
/// (`types.NewModifiedFinding`).
#[derive(Debug, Clone)]
pub struct ModifiedFinding {
    pub finding: Finding,
    pub status: FindingStatus,
    pub justification: String,
    pub source: String,
}

/// Result of filtering a finding set through a VEX document.
#[derive(Debug, Clone, Default)]
pub struct FilterOutcome {
    /// Findings that survived VEX evaluation.
    pub kept: Vec<Finding>,
    /// Findings suppressed by a `not_affected` / `fixed` statement.
    pub modified: Vec<ModifiedFinding>,
}

/// Port of `pkg/vex/openvex.go::OpenVEX`.
pub struct OpenVex {
    statements: Vec<VexStatement>,
    source: String,
}

impl OpenVex {
    /// Port of `newOpenVEX`.
    pub fn new(statements: Vec<VexStatement>, source: String) -> Self {
        Self { statements, source }
    }

    /// Port of `OpenVEX.Matches`.
    ///
    /// Returns every statement whose vulnerability id and product PURL
    /// match. When the caller passes sub-component PURLs and the statement
    /// carries its own sub-component list, both must intersect — mirroring
    /// `vex.Matches(vulnID, productPURL, subComponentPURLs)`.
    pub fn matches<'a>(
        &'a self,
        vuln_id: &str,
        product_purl: &str,
        sub_component_purls: &[String],
    ) -> Vec<&'a VexStatement> {
        if product_purl.is_empty() {
            return Vec::new();
        }
        self.statements
            .iter()
            .filter(|s| s.vuln_id == vuln_id && s.product_purl == product_purl)
            .filter(|s| sub_components_match(&s.sub_components, sub_component_purls))
            .collect()
    }

    /// Port of `OpenVEX.NotAffected`.
    ///
    /// Takes the **latest** matching statement (a newer statement overrides
    /// an older one for the same vuln+product) and suppresses the finding
    /// when that statement is `not_affected` or `fixed`.
    pub fn not_affected(
        &self,
        vuln_id: &str,
        product_purl: &str,
        sub_component_purls: &[String],
    ) -> Option<NotAffectedHit> {
        let stmts = self.matches(vuln_id, product_purl, sub_component_purls);
        if stmts.is_empty() {
            return None;
        }
        // Take the latest statement; a sequence can be overridden by the
        // newer one (cf. openvex.go comment + OPENVEX-SPEC).
        let stmt = stmts[stmts.len() - 1];
        if stmt.status == VexStatus::NotAffected || stmt.status == VexStatus::Fixed {
            return Some(NotAffectedHit {
                status: finding_status(stmt.status),
                justification: stmt.justification.as_str().to_string(),
                source: self.source.clone(),
            });
        }
        None
    }

    /// Port of `vex.go::filterVulnerabilities` over a flat finding list.
    ///
    /// Each finding's vulnerability id (the first CVE) is checked against
    /// the VEX document using the finding's package PURL as the product.
    /// Suppressed findings are moved into `modified`; the rest are kept.
    pub fn filter(&self, findings: Vec<Finding>) -> FilterOutcome {
        let mut outcome = FilterOutcome::default();
        for finding in findings {
            let product = finding
                .location
                .package
                .clone()
                .unwrap_or_default();
            let vuln_id = finding.cves.first().cloned().unwrap_or_default();

            let hit = if vuln_id.is_empty() {
                None
            } else {
                self.not_affected(&vuln_id, &product, &[])
            };

            match hit {
                Some(h) => outcome.modified.push(ModifiedFinding {
                    finding,
                    status: h.status,
                    justification: h.justification,
                    source: h.source,
                }),
                None => outcome.kept.push(finding),
            }
        }
        outcome
    }
}

/// The non-finding payload of a successful `not_affected` match — lets the
/// test inspect status/source without cloning the finding.
#[derive(Debug, Clone)]
pub struct NotAffectedHit {
    pub status: FindingStatus,
    pub justification: String,
    pub source: String,
}

/// Sub-component intersection rule from `vex.Matches`: an empty statement
/// sub-component list matches any caller sub-components, and vice versa;
/// otherwise at least one PURL must be shared.
fn sub_components_match(statement_subs: &[String], caller_subs: &[String]) -> bool {
    if statement_subs.is_empty() || caller_subs.is_empty() {
        return true;
    }
    statement_subs.iter().any(|s| caller_subs.contains(s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_product_never_matches() {
        let vex = OpenVex::new(
            vec![VexStatement {
                vuln_id: "CVE-1".to_string(),
                product_purl: "pkg:x".to_string(),
                sub_components: vec![],
                status: VexStatus::NotAffected,
                justification: VexJustification::None,
            }],
            "s".to_string(),
        );
        assert!(vex.matches("CVE-1", "", &[]).is_empty());
    }

    #[test]
    fn justification_label_roundtrip() {
        assert_eq!(
            VexJustification::ComponentNotPresent.as_str(),
            "component_not_present"
        );
    }
}
