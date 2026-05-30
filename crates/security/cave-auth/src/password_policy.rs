// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
// Line-port of Keycloak password-policy parsing + validators (Apache-2.0, v26.6.2):
//   server-spi/.../models/PasswordPolicy.java                  -- policy string -> ordered config
//   server-spi-private/.../policy/*PasswordPolicyProvider.java -- one validator each
//
// Keycloak parses a policy string such as `"length(8) and digits(2) and notUsername"`
// into an ordered map (insertion order preserved by `LinkedHashMap`), then on each
// password change runs every configured validator in order and returns the first
// `PolicyError`. This module reproduces that exactly: the policy-string grammar
// (`split(" and ")`, `key(config)`), the per-validator defaults and counting rules,
// and the canonical message-bundle ids used as the error key.

use regex::Regex;

/// A failed policy check. Mirrors Keycloak `PolicyError(message, parameters...)`:
/// `message` is the message-bundle id; `parameter` carries the single contextual
/// argument the upstream provider passes (the configured min/max for the count
/// validators, or the offending pattern for `regexPattern`). `None` for the
/// argument-less validators (`notUsername`, `notContainsUsername`, `notEmail`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyError {
    pub message: String,
    pub parameter: Option<String>,
}

impl PolicyError {
    fn new(message: &str, parameter: Option<String>) -> Self {
        Self {
            message: message.to_string(),
            parameter,
        }
    }
}

/// One configured validator (a single entry of the parsed policy).
#[derive(Debug, Clone)]
enum Rule {
    Length(usize),       // min length, default 8
    MaxLength(usize),    // max length, default 64
    Digits(usize),       // min digit chars, default 1
    UpperCase(usize),    // min upper-case chars, default 1
    LowerCase(usize),    // min lower-case chars, default 1
    SpecialChars(usize), // min non-alphanumeric chars, default 1
    NotUsername,
    NotContainsUsername,
    NotEmail,
    Regex(Regex, String), // compiled pattern + original source text
}

/// An ordered list of password-policy rules.
#[derive(Debug, Clone, Default)]
pub struct PasswordPolicy {
    rules: Vec<Rule>,
}

/// `PasswordPolicyProvider.parseInteger(value, default)` — parse or fall back.
fn parse_integer(config: Option<&str>, default: usize) -> usize {
    match config {
        Some(v) => v.trim().parse().unwrap_or(default),
        None => default,
    }
}

impl PasswordPolicy {
    /// `PasswordPolicy.Builder` — split on `" and "`, each token is `key` or
    /// `key(config)`. Unknown keys (and policies handled elsewhere, e.g. hashing,
    /// history) are ignored here; only the synchronous character validators are kept.
    pub fn parse(policy_string: &str) -> Self {
        let mut rules = Vec::new();
        let trimmed = policy_string.trim();
        if !trimmed.is_empty() {
            for raw in trimmed.split(" and ") {
                let policy = raw.trim();
                let (key, config) = match policy.find('(') {
                    None => (policy, None),
                    Some(i) => {
                        // `policy.substring(i + 1, length - 1)` — between the parens.
                        let inner = &policy[i + 1..policy.len().saturating_sub(1)];
                        (policy[..i].trim(), Some(inner))
                    }
                };
                if let Some(rule) = Self::build_rule(key, config) {
                    rules.push(rule);
                }
            }
        }
        Self { rules }
    }

    fn build_rule(key: &str, config: Option<&str>) -> Option<Rule> {
        Some(match key {
            "length" => Rule::Length(parse_integer(config, 8)),
            "maxLength" => Rule::MaxLength(parse_integer(config, 64)),
            "digits" => Rule::Digits(parse_integer(config, 1)),
            "upperCase" => Rule::UpperCase(parse_integer(config, 1)),
            "lowerCase" => Rule::LowerCase(parse_integer(config, 1)),
            "specialChars" => Rule::SpecialChars(parse_integer(config, 1)),
            "notUsername" => Rule::NotUsername,
            "notContainsUsername" => Rule::NotContainsUsername,
            "notEmail" => Rule::NotEmail,
            "regexPattern" => {
                let src = config.unwrap_or("");
                // Java `Pattern.matcher(pw).matches()` anchors the whole input; the
                // `regex` crate's `is_match` is unanchored, so wrap with \A...\z.
                let anchored = format!(r"\A(?:{src})\z");
                match Regex::new(&anchored) {
                    Ok(re) => Rule::Regex(re, src.to_string()),
                    Err(_) => return None, // invalid pattern -> not enforceable
                }
            }
            _ => return None, // policies handled outside this validator set
        })
    }

    /// Run every rule in order; return the first failure (Keycloak
    /// `PasswordPolicyManagerProvider.validate` returns the first non-null error).
    pub fn validate(
        &self,
        username: Option<&str>,
        email: Option<&str>,
        password: &str,
    ) -> Option<PolicyError> {
        for rule in &self.rules {
            if let Some(err) = Self::check(rule, username, email, password) {
                return Some(err);
            }
        }
        None
    }

    fn check(
        rule: &Rule,
        username: Option<&str>,
        email: Option<&str>,
        password: &str,
    ) -> Option<PolicyError> {
        // Java `String.length()` is UTF-16 code units; for the ASCII passwords these
        // policies target, `chars().count()` is equivalent. Counting predicates use
        // `chars()`; `is_alphanumeric` mirrors `Character.isLetterOrDigit`.
        match rule {
            Rule::Length(min) => (password.chars().count() < *min)
                .then(|| PolicyError::new("invalidPasswordMinLengthMessage", Some(min.to_string()))),
            Rule::MaxLength(max) => (password.chars().count() > *max)
                .then(|| PolicyError::new("invalidPasswordMaxLengthMessage", Some(max.to_string()))),
            Rule::Digits(min) => {
                let count = password.chars().filter(|c| c.is_ascii_digit()).count();
                (count < *min).then(|| {
                    PolicyError::new("invalidPasswordMinDigitsMessage", Some(min.to_string()))
                })
            }
            Rule::UpperCase(min) => {
                let count = password.chars().filter(|c| c.is_uppercase()).count();
                (count < *min).then(|| {
                    PolicyError::new(
                        "invalidPasswordMinUpperCaseCharsMessage",
                        Some(min.to_string()),
                    )
                })
            }
            Rule::LowerCase(min) => {
                let count = password.chars().filter(|c| c.is_lowercase()).count();
                (count < *min).then(|| {
                    PolicyError::new(
                        "invalidPasswordMinLowerCaseCharsMessage",
                        Some(min.to_string()),
                    )
                })
            }
            Rule::SpecialChars(min) => {
                let count = password.chars().filter(|c| !c.is_alphanumeric()).count();
                (count < *min).then(|| {
                    PolicyError::new(
                        "invalidPasswordMinSpecialCharsMessage",
                        Some(min.to_string()),
                    )
                })
            }
            Rule::NotUsername => {
                let u = username?;
                u.eq_ignore_ascii_case(password)
                    .then(|| PolicyError::new("invalidPasswordNotUsernameMessage", None))
            }
            Rule::NotContainsUsername => {
                let u = username?;
                password
                    .to_lowercase()
                    .contains(&u.to_lowercase())
                    .then(|| PolicyError::new("invalidPasswordNotContainsUsernameMessage", None))
            }
            Rule::NotEmail => {
                let e = email?;
                e.eq_ignore_ascii_case(password)
                    .then(|| PolicyError::new("invalidPasswordNotEmailMessage", None))
            }
            Rule::Regex(re, src) => (!re.is_match(password)).then(|| {
                PolicyError::new("invalidPasswordRegexPatternMessage", Some(src.clone()))
            }),
        }
    }
}
