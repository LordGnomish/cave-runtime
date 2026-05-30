// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Strict-TDD line-port of Keycloak password-policy validators.
// Upstream (Apache-2.0, v26.6.2):
//   server-spi/.../models/PasswordPolicy.java                 (policy-string parsing: split " and ")
//   server-spi-private/.../policy/LengthPasswordPolicyProvider.java          (default min 8)
//   server-spi-private/.../policy/MaximumLengthPasswordPolicyProviderFactory (default max 64)
//   server-spi-private/.../policy/DigitsPasswordPolicyProvider.java          (default 1)
//   server-spi-private/.../policy/UpperCasePasswordPolicyProvider.java       (default 1)
//   server-spi-private/.../policy/LowerCasePasswordPolicyProvider.java       (default 1)
//   server-spi-private/.../policy/SpecialCharsPasswordPolicyProvider.java    (default 1; !isLetterOrDigit)
//   server-spi-private/.../policy/NotUsernamePasswordPolicyProvider.java     (equalsIgnoreCase)
//   server-spi-private/.../policy/NotContainsUsernamePasswordPolicyProvider  (toLowerCase().contains)
//   server-spi-private/.../policy/NotEmailPasswordPolicyProvider.java        (equalsIgnoreCase)
//   server-spi-private/.../policy/RegexPatternsPasswordPolicyProvider.java   (Matcher.matches)
//
// Error-message ids are the exact Keycloak message bundle keys.

use cave_auth::password_policy::PasswordPolicy;

fn err_msg(p: &PasswordPolicy, username: Option<&str>, email: Option<&str>, pw: &str) -> Option<String> {
    p.validate(username, email, pw).map(|e| e.message)
}

#[test]
fn length_default_is_eight() {
    let p = PasswordPolicy::parse("length");
    let e = p.validate(None, None, "short").unwrap(); // 5 < 8
    assert_eq!(e.message, "invalidPasswordMinLengthMessage");
    assert_eq!(e.parameter.as_deref(), Some("8"));
    assert!(p.validate(None, None, "longenough").is_none()); // 10 >= 8
}

#[test]
fn length_explicit_min() {
    let p = PasswordPolicy::parse("length(10)");
    assert!(p.validate(None, None, "abc").is_some());
    assert!(p.validate(None, None, "abcdefghij").is_none()); // exactly 10
}

#[test]
fn max_length_default_and_explicit() {
    let def = PasswordPolicy::parse("maxLength");
    assert!(def.validate(None, None, &"a".repeat(64)).is_none()); // 64 ok
    assert_eq!(
        def.validate(None, None, &"a".repeat(65)).unwrap().message,
        "invalidPasswordMaxLengthMessage"
    );
    let p = PasswordPolicy::parse("maxLength(12)");
    assert!(p.validate(None, None, "abcdefghijkl").is_none()); // 12 ok
    assert!(p.validate(None, None, "abcdefghijklm").is_some()); // 13 > 12
}

#[test]
fn digits_count() {
    let p = PasswordPolicy::parse("digits(2)");
    assert_eq!(
        err_msg(&p, None, None, "abc1").as_deref(),
        Some("invalidPasswordMinDigitsMessage")
    );
    assert!(p.validate(None, None, "ab12cd").is_none()); // two digits
    // default is 1
    let d = PasswordPolicy::parse("digits");
    assert!(d.validate(None, None, "abc").is_some());
    assert!(d.validate(None, None, "abc7").is_none());
}

#[test]
fn upper_lower_special_counts() {
    let up = PasswordPolicy::parse("upperCase(2)");
    assert_eq!(err_msg(&up, None, None, "Abc").as_deref(), Some("invalidPasswordMinUpperCaseCharsMessage"));
    assert!(up.validate(None, None, "ABc").is_none());

    let low = PasswordPolicy::parse("lowerCase(2)");
    assert_eq!(err_msg(&low, None, None, "ABc").as_deref(), Some("invalidPasswordMinLowerCaseCharsMessage"));
    assert!(low.validate(None, None, "abC").is_none());

    let sp = PasswordPolicy::parse("specialChars(2)");
    // '!' and '@' are non-alphanumeric; letters/digits are not special.
    assert_eq!(err_msg(&sp, None, None, "ab!cd").as_deref(), Some("invalidPasswordMinSpecialCharsMessage"));
    assert!(sp.validate(None, None, "a!b@c").is_none());
}

#[test]
fn not_username_equals_ignore_case() {
    let p = PasswordPolicy::parse("notUsername");
    assert_eq!(
        err_msg(&p, Some("Alice"), None, "alice").as_deref(),
        Some("invalidPasswordNotUsernameMessage")
    );
    // Different password passes; missing username short-circuits to OK.
    assert!(p.validate(Some("Alice"), None, "secretpw").is_none());
    assert!(p.validate(None, None, "alice").is_none());
}

#[test]
fn not_contains_username_substring() {
    let p = PasswordPolicy::parse("notContainsUsername");
    assert_eq!(
        err_msg(&p, Some("bob"), None, "xxBOByy").as_deref(),
        Some("invalidPasswordNotContainsUsernameMessage")
    );
    assert!(p.validate(Some("bob"), None, "unrelated").is_none());
    assert!(p.validate(None, None, "anything").is_none());
}

#[test]
fn not_email_equals_ignore_case() {
    let p = PasswordPolicy::parse("notEmail");
    assert_eq!(
        err_msg(&p, None, Some("User@Example.com"), "user@example.com").as_deref(),
        Some("invalidPasswordNotEmailMessage")
    );
    assert!(p.validate(None, Some("user@example.com"), "different").is_none());
    assert!(p.validate(None, None, "user@example.com").is_none());
}

#[test]
fn regex_pattern_must_match_fully() {
    let p = PasswordPolicy::parse("regexPattern(^[a-z]+$)");
    assert!(p.validate(None, None, "abcdef").is_none());
    let e = p.validate(None, None, "abc123").unwrap();
    assert_eq!(e.message, "invalidPasswordRegexPatternMessage");
    assert_eq!(e.parameter.as_deref(), Some("^[a-z]+$"));
}

#[test]
fn composite_policy_returns_first_failure_in_order() {
    // length ok (13), then digits(1) fails before upperCase is evaluated.
    let p = PasswordPolicy::parse("length(8) and digits(1) and upperCase(1)");
    let e = p.validate(None, None, "lowercaseonly").unwrap();
    assert_eq!(e.message, "invalidPasswordMinDigitsMessage");
    // A password satisfying all three passes.
    assert!(p.validate(None, None, "Lowercase1xx").is_none());
}

#[test]
fn empty_policy_accepts_everything() {
    let p = PasswordPolicy::parse("");
    assert!(p.validate(None, None, "").is_none());
    assert!(p.validate(Some("x"), Some("y"), "z").is_none());
}
