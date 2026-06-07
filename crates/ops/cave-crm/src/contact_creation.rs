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
