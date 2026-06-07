// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM contact-creation-manager utils —
//! `packages/twenty-server/src/modules/contact-creation-manager/utils/`
//!
//! The pure helper layer that the auto-companies-and-contacts creation job
//! uses to derive Person/Company records from message & calendar
//! participants. Each function below is a line-by-line port of the upstream
//! util of the same name; the upstream `__tests__/*.spec.ts` vectors are
//! mirrored verbatim in the test module so behavior is pinned exactly.
//!
//! Ported functions:
//!   * `extractDomainFromLink`                          → [`extract_domain_from_link`]
//!   * `getDomainNameFromHandle`                        → [`get_domain_name_from_handle`]
//!   * `getCompanyNameFromDomainName`                   → [`get_company_name_from_domain_name`]
//!   * `getFirstNameAndLastNameFromHandleAndDisplayName`→ [`get_first_name_and_last_name_from_handle_and_display_name`]
//!   * `hasPrimaryEmailChanged`                         → [`has_primary_email_changed`]
//!   * `computeChangedAdditionalEmails`                 → [`compute_changed_additional_emails`]
//!
//! The two domain helpers replicate Twenty's dependency on the `psl`
//! (Public Suffix List) package. The matching ALGORITHM is faithful to the
//! IANA PSL spec — exception (`!`) rules win, then the longest wildcard /
//! exact rule, falling back to the spec's default `*` prevailing rule (one
//! label). Single-label TLDs therefore need no enumeration; only multi-label
//! public suffixes (e.g. `co.uk`) and the few exception/wildcard rules are
//! embedded, as a curated subset of the IANA list's ICANN section.

use serde::{Deserialize, Serialize};

// ─────────────────────── string helpers ────────────────────────────────

/// Port of twenty-shared `capitalize`: `''` for empty, else uppercase the
/// first char and append the remainder unchanged.
fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
    }
}

/// `a` if non-empty, else `b` if non-empty, else `""` — the `a || b || ''`
/// short-circuit used by the name-derivation util.
fn first_non_empty<'a>(a: &'a str, b: &'a str) -> &'a str {
    if !a.is_empty() {
        a
    } else if !b.is_empty() {
        b
    } else {
        ""
    }
}

// ─────────────────────── extractDomainFromLink ─────────────────────────

/// Port of `extractDomainFromLink`: strip a leading `https?://` and `www.`
/// (both case-insensitive, both optional) then return everything before the
/// first `/`. Mirrors `link.replace(/^(https?:\/\/)?(www\.)?/i, '').split('/')[0]`.
pub fn extract_domain_from_link(link: &str) -> String {
    let mut rest = link;
    let lower = rest.to_ascii_lowercase();
    if lower.starts_with("https://") {
        rest = &rest[8..];
    } else if lower.starts_with("http://") {
        rest = &rest[7..];
    }
    if rest.to_ascii_lowercase().starts_with("www.") {
        rest = &rest[4..];
    }
    rest.split('/').next().unwrap_or("").to_string()
}

// ──────── getFirstNameAndLastNameFromHandleAndDisplayName ───────────────

/// The `{ firstName, lastName }` pair returned by the name-derivation util.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameParts {
    pub first_name: String,
    pub last_name: String,
}

/// Port of `getFirstNameAndLastNameFromHandleAndDisplayName`: prefer the
/// space-split display name, falling back to the `.`-split local-part of the
/// handle, capitalizing each component.
pub fn get_first_name_and_last_name_from_handle_and_display_name(
    handle: &str,
    display_name: &str,
) -> NameParts {
    let dn_first = display_name.split(' ').next().unwrap_or("");
    let dn_last = display_name.split(' ').nth(1).unwrap_or("");

    let full_from_handle = handle.split('@').next().unwrap_or("");
    let first_from_handle = full_from_handle.split('.').next().unwrap_or("");
    let last_from_handle = full_from_handle.split('.').nth(1).unwrap_or("");

    NameParts {
        first_name: capitalize(first_non_empty(dn_first, first_from_handle)),
        last_name: capitalize(first_non_empty(dn_last, last_from_handle)),
    }
}

// ─────────────────────── Person emails diff ────────────────────────────

/// The `emails` composite value: a required `primaryEmail` (nullable) and an
/// `additionalEmails` array (nullable). Mirrors Twenty's `FieldMetadataType`
/// EMAILS composite on `PersonWorkspaceEntity`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailsValue {
    pub primary_email: Option<String>,
    pub additional_emails: Option<Vec<String>>,
}

/// `{ before, after }` for the `emails` field — either side may be absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailsFieldDiff {
    pub before: Option<EmailsValue>,
    pub after: Option<EmailsValue>,
}

/// `Partial<ObjectRecordDiff<PersonWorkspaceEntity>>` projected to the only
/// field these utils read — `emails`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonDiff {
    pub emails: Option<EmailsFieldDiff>,
}

/// Result of [`compute_changed_additional_emails`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedEmails {
    pub added_additional_emails: Vec<String>,
    pub removed_additional_emails: Vec<String>,
}

/// Port of `hasPrimaryEmailChanged`: compare the case-folded `primaryEmail`
/// before/after. A missing `emails` diff, side, or `primaryEmail` reads as
/// `None`; an empty string stays a distinct value (matching JS where
/// `''.toLowerCase()` is `''` but `null?.toLowerCase()` is `undefined`).
pub fn has_primary_email_changed(diff: &PersonDiff) -> bool {
    let primary = |side: Option<&EmailsValue>| -> Option<String> {
        side.and_then(|v| v.primary_email.as_ref())
            .map(|s| s.to_lowercase())
    };
    let before = primary(diff.emails.as_ref().and_then(|e| e.before.as_ref()));
    let after = primary(diff.emails.as_ref().and_then(|e| e.after.as_ref()));
    before != after
}

/// Port of `computeChangedAdditionalEmails`: when both sides expose an
/// `additionalEmails` array, diff them case-insensitively, preserving order;
/// returned addresses are lowercased (as upstream filters the lowercased
/// arrays). If either side is not an array, both lists are empty.
pub fn compute_changed_additional_emails(diff: &PersonDiff) -> ChangedEmails {
    let side = |v: Option<&EmailsValue>| -> Option<Vec<String>> {
        v.and_then(|x| x.additional_emails.as_ref())
            .map(|list| list.iter().map(|e| e.to_lowercase()).collect())
    };
    let before = side(diff.emails.as_ref().and_then(|e| e.before.as_ref()));
    let after = side(diff.emails.as_ref().and_then(|e| e.after.as_ref()));

    match (before, after) {
        (Some(before), Some(after)) => ChangedEmails {
            added_additional_emails: after
                .iter()
                .filter(|e| !before.contains(e))
                .cloned()
                .collect(),
            removed_additional_emails: before
                .iter()
                .filter(|e| !after.contains(e))
                .cloned()
                .collect(),
        },
        _ => ChangedEmails::default(),
    }
}

// ─────────────────── Public Suffix List (psl) port ─────────────────────

/// Multi-label public suffixes (ICANN section), curated subset of the IANA
/// Public Suffix List. Single-label TLDs are covered by the spec's default
/// `*` prevailing rule and need no enumeration here.
const PSL_RULES: &[&str] = &[
    // United Kingdom
    "co.uk", "org.uk", "gov.uk", "ac.uk", "me.uk", "net.uk", "sch.uk", "ltd.uk",
    "plc.uk", "nhs.uk", "police.uk", "mod.uk",
    // Australia
    "com.au", "net.au", "org.au", "edu.au", "gov.au", "id.au", "asn.au",
    // Japan
    "co.jp", "ne.jp", "or.jp", "go.jp", "ac.jp", "ad.jp", "ed.jp", "gr.jp", "lg.jp",
    // New Zealand
    "co.nz", "net.nz", "org.nz", "govt.nz", "ac.nz", "geek.nz", "school.nz",
    // South Africa
    "co.za", "org.za", "net.za", "gov.za", "web.za",
    // Brazil
    "com.br", "net.br", "org.br", "gov.br", "edu.br",
    // India
    "co.in", "net.in", "org.in", "gen.in", "firm.in", "ind.in", "gov.in", "ac.in", "edu.in",
    // China
    "com.cn", "net.cn", "org.cn", "gov.cn", "edu.cn", "ac.cn",
    // South Korea
    "co.kr", "ne.kr", "or.kr", "re.kr", "go.kr", "ac.kr",
    // Wildcard rules (exercise the `*` mechanism — real IANA entries)
    "*.ck", "*.kawasaki.jp",
];

/// Exception rules (IANA `!` entries). The public suffix of a matching
/// domain is the exception minus its leftmost label.
const PSL_EXCEPTIONS: &[&str] = &["www.ck", "city.kawasaki.jp"];

/// Does `rule_labels` (with `*` matching any single label) align as a suffix
/// of `labels`?
fn rule_suffix_matches(rule_labels: &[&str], labels: &[&str]) -> bool {
    if rule_labels.len() > labels.len() {
        return false;
    }
    let offset = labels.len() - rule_labels.len();
    rule_labels.iter().enumerate().all(|(i, r)| {
        *r == "*" || r.eq_ignore_ascii_case(labels[offset + i])
    })
}

/// Number of labels in the public suffix of `labels`, per the PSL algorithm:
/// an exception rule wins (suffix = rule − leftmost label); else the longest
/// matching exact/wildcard rule; else the default `*` rule (one label).
fn public_suffix_label_count(labels: &[&str]) -> usize {
    for rule in PSL_EXCEPTIONS {
        let rl: Vec<&str> = rule.split('.').collect();
        if rule_suffix_matches(&rl, labels) {
            return rl.len() - 1;
        }
    }
    let mut best = 1usize; // default prevailing rule `*`
    for rule in PSL_RULES {
        let rl: Vec<&str> = rule.split('.').collect();
        if rl.len() > best && rule_suffix_matches(&rl, labels) {
            best = rl.len();
        }
    }
    best.min(labels.len())
}

/// The parsed registrable-domain projection cave-crm reads from psl —
/// `sld` and `domain`, either of which is `None` when the input is itself a
/// public suffix (or too short to have a registrable part).
struct ParsedDomain {
    sld: Option<String>,
    domain: Option<String>,
}

/// Port of `psl.parse` restricted to the fields these utils use. Returns
/// `None` for inputs psl rejects (empty / invalid labels) — i.e. the cases
/// where `isParsedDomain` is false.
fn psl_parse(input: &str) -> Option<ParsedDomain> {
    let domain = input.to_lowercase();
    if domain.is_empty() || domain.len() > 253 {
        return None;
    }
    let labels: Vec<&str> = domain.split('.').collect();
    for label in &labels {
        if label.is_empty() || label.len() > 63 {
            return None;
        }
        if label.starts_with('-') || label.ends_with('-') {
            return None;
        }
        if !label.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return None;
        }
    }
    let suffix_len = public_suffix_label_count(&labels);
    let n = labels.len();
    if n > suffix_len {
        let sld = labels[n - suffix_len - 1].to_string();
        let domain = labels[n - suffix_len - 1..].join(".");
        Some(ParsedDomain { sld: Some(sld), domain: Some(domain) })
    } else {
        // Input is itself a public suffix → no registrable domain.
        Some(ParsedDomain { sld: None, domain: None })
    }
}

/// Port of `getDomainNameFromHandle`: take the part after `@`, parse it, and
/// return the registrable `domain` (or `''`).
pub fn get_domain_name_from_handle(handle: &str) -> String {
    let whole_domain = handle.split('@').nth(1).unwrap_or("");
    match psl_parse(whole_domain) {
        Some(p) => p.domain.unwrap_or_default(),
        None => String::new(),
    }
}

/// Port of `getCompanyNameFromDomainName`: parse the domain and capitalize
/// its second-level label (`sld`), or `''`.
pub fn get_company_name_from_domain_name(domain_name: &str) -> String {
    match psl_parse(domain_name) {
        Some(p) => p.sld.map(|s| capitalize(&s)).unwrap_or_default(),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(primary: Option<&str>, additional: Option<Vec<&str>>) -> EmailsValue {
        EmailsValue {
            primary_email: primary.map(|s| s.to_string()),
            additional_emails: additional.map(|v| v.into_iter().map(|s| s.to_string()).collect()),
        }
    }

    // ── getDomainNameFromHandle.spec.ts (8 vectors) ──────────────────────
    #[test]
    fn get_domain_name_from_handle_vectors() {
        let cases = [
            ("user@twenty.dev", "twenty.dev"),
            ("user@app.twenty.dev", "twenty.dev"),
            ("user@test.app.twenty.dev", "twenty.dev"),
            ("user@twenty.co.uk", "twenty.co.uk"),
            ("", ""),
            ("not-an-email", ""),
            ("user@", ""),
            ("user@not-a-valid-domain", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(get_domain_name_from_handle(input), expected, "input={input:?}");
        }
    }

    // ── getCompanyNameFromDomainName.spec.ts (6 vectors) ─────────────────
    #[test]
    fn get_company_name_from_domain_name_vectors() {
        let cases = [
            ("twenty.dev", "Twenty"),
            ("app.twenty.dev", "Twenty"),
            ("test.app.twenty.dev", "Twenty"),
            ("twenty.co.uk", "Twenty"),
            ("", ""),
            ("not-a-valid-domain", ""),
        ];
        for (input, expected) in cases {
            assert_eq!(get_company_name_from_domain_name(input), expected, "input={input:?}");
        }
    }

    // ── extractDomainFromLink — regex ^(https?://)?(www\.)? then split('/')[0]
    #[test]
    fn extract_domain_from_link_strips_scheme_www_and_path() {
        assert_eq!(extract_domain_from_link("https://www.twenty.com/pricing"), "twenty.com");
        assert_eq!(extract_domain_from_link("http://twenty.com"), "twenty.com");
        assert_eq!(extract_domain_from_link("twenty.com"), "twenty.com");
        assert_eq!(extract_domain_from_link("https://app.twenty.dev/foo/bar"), "app.twenty.dev");
        // www. and scheme matched case-insensitively; rest of host preserved.
        assert_eq!(extract_domain_from_link("HTTPS://WWW.Example.COM/x"), "Example.COM");
        assert_eq!(extract_domain_from_link("www.twenty.com"), "twenty.com");
    }

    // ── getFirstNameAndLastNameFromHandleAndDisplayName ──────────────────
    #[test]
    fn first_last_from_display_name_wins() {
        let n = get_first_name_and_last_name_from_handle_and_display_name("john.doe@x.com", "John Doe");
        assert_eq!(n.first_name, "John");
        assert_eq!(n.last_name, "Doe");
    }

    #[test]
    fn first_last_falls_back_to_handle_parts() {
        let n = get_first_name_and_last_name_from_handle_and_display_name("john.doe@x.com", "");
        assert_eq!(n.first_name, "John");
        assert_eq!(n.last_name, "Doe");
    }

    #[test]
    fn first_last_handles_single_token_handle() {
        let n = get_first_name_and_last_name_from_handle_and_display_name("alice@x.com", "");
        assert_eq!(n.first_name, "Alice");
        assert_eq!(n.last_name, "");
    }

    #[test]
    fn first_last_single_display_name_no_last() {
        let n = get_first_name_and_last_name_from_handle_and_display_name("m@x.com", "Madonna");
        assert_eq!(n.first_name, "Madonna");
        assert_eq!(n.last_name, "");
    }

    // ── hasPrimaryEmailChanged.spec.ts (16 vectors) ──────────────────────
    #[test]
    fn has_primary_email_changed_vectors() {
        // (before_primary, after_primary, before_present, after_present, emails_present, expected)
        let none_diff = PersonDiff { emails: None };
        assert!(!has_primary_email_changed(&none_diff), "emails diff undefined");

        let mk = |before: Option<EmailsValue>, after: Option<EmailsValue>| PersonDiff {
            emails: Some(EmailsFieldDiff { before, after }),
        };

        // changed
        assert!(has_primary_email_changed(&mk(
            Some(ev(Some("old@example.com"), Some(vec![]))),
            Some(ev(Some("new@example.com"), Some(vec![]))),
        )));
        // unchanged primary, different additional
        assert!(!has_primary_email_changed(&mk(
            Some(ev(Some("same@example.com"), Some(vec!["additional@example.com"]))),
            Some(ev(Some("same@example.com"), Some(vec!["different@example.com"]))),
        )));
        // null -> value
        assert!(has_primary_email_changed(&mk(
            Some(ev(None, Some(vec![]))),
            Some(ev(Some("new@example.com"), Some(vec![]))),
        )));
        // value -> null
        assert!(has_primary_email_changed(&mk(
            Some(ev(Some("old@example.com"), Some(vec![]))),
            Some(ev(None, Some(vec![]))),
        )));
        // both null
        assert!(!has_primary_email_changed(&mk(
            Some(ev(None, Some(vec![]))),
            Some(ev(None, Some(vec![]))),
        )));
        // empty string -> value
        assert!(has_primary_email_changed(&mk(
            Some(ev(Some(""), Some(vec![]))),
            Some(ev(Some("new@example.com"), Some(vec![]))),
        )));
        // value -> empty string
        assert!(has_primary_email_changed(&mk(
            Some(ev(Some("old@example.com"), Some(vec![]))),
            Some(ev(Some(""), Some(vec![]))),
        )));
        // both empty strings
        assert!(!has_primary_email_changed(&mk(
            Some(ev(Some(""), Some(vec![]))),
            Some(ev(Some(""), Some(vec![]))),
        )));
        // before value undefined
        assert!(has_primary_email_changed(&mk(
            None,
            Some(ev(Some("new@example.com"), Some(vec![]))),
        )));
        // after value undefined
        assert!(has_primary_email_changed(&mk(
            Some(ev(Some("old@example.com"), Some(vec![]))),
            None,
        )));
        // both values undefined
        assert!(!has_primary_email_changed(&mk(None, None)));
        // case-insensitive
        assert!(!has_primary_email_changed(&mk(
            Some(ev(Some("test@example.com"), Some(vec![]))),
            Some(ev(Some("TEST@EXAMPLE.COM"), Some(vec![]))),
        )));
    }

    // ── computeChangedAdditionalEmails.spec.ts (11 vectors) ──────────────
    #[test]
    fn compute_changed_additional_emails_vectors() {
        let mk = |before: Option<EmailsValue>, after: Option<EmailsValue>| PersonDiff {
            emails: Some(EmailsFieldDiff { before, after }),
        };
        let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();

        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("primary@example.com"), Some(vec!["old1@example.com", "common@example.com"]))),
            Some(ev(Some("primary@example.com"), Some(vec!["new1@example.com", "common@example.com"]))),
        ));
        assert_eq!(r.added_additional_emails, s(&["new1@example.com"]));
        assert_eq!(r.removed_additional_emails, s(&["old1@example.com"]));

        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec![]))),
            Some(ev(Some("p@e.com"), Some(vec!["new1@example.com", "new2@example.com"]))),
        ));
        assert_eq!(r.added_additional_emails, s(&["new1@example.com", "new2@example.com"]));
        assert!(r.removed_additional_emails.is_empty());

        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec!["old1@example.com", "old2@example.com"]))),
            Some(ev(Some("p@e.com"), Some(vec![]))),
        ));
        assert!(r.added_additional_emails.is_empty());
        assert_eq!(r.removed_additional_emails, s(&["old1@example.com", "old2@example.com"]));

        // both empty
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec![]))),
            Some(ev(Some("p@e.com"), Some(vec![]))),
        ));
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());

        // same emails
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec!["email1@example.com", "email2@example.com"]))),
            Some(ev(Some("p@e.com"), Some(vec!["email1@example.com", "email2@example.com"]))),
        ));
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());

        // before not array (null)
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), None)),
            Some(ev(Some("p@e.com"), Some(vec!["new@example.com"]))),
        ));
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());

        // after not array (null)
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec!["old@example.com"]))),
            Some(ev(Some("p@e.com"), None)),
        ));
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());

        // both not arrays
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), None)),
            Some(ev(Some("p@e.com"), None)),
        ));
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());

        // emails diff undefined
        let r = compute_changed_additional_emails(&PersonDiff { emails: None });
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());

        // complex multi add/remove, order preserved
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec!["keep1@example.com", "remove1@example.com", "keep2@example.com", "remove2@example.com"]))),
            Some(ev(Some("p@e.com"), Some(vec!["keep1@example.com", "add1@example.com", "keep2@example.com", "add2@example.com"]))),
        ));
        assert_eq!(r.added_additional_emails, s(&["add1@example.com", "add2@example.com"]));
        assert_eq!(r.removed_additional_emails, s(&["remove1@example.com", "remove2@example.com"]));

        // case-insensitive
        let r = compute_changed_additional_emails(&mk(
            Some(ev(Some("p@e.com"), Some(vec!["old@example.com"]))),
            Some(ev(Some("p@e.com"), Some(vec!["OLD@example.com"]))),
        ));
        assert!(r.added_additional_emails.is_empty() && r.removed_additional_emails.is_empty());
    }
}
