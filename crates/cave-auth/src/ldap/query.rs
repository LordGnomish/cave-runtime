// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/query/internal/LDAPQueryBuilder.java + RFC 4515 §3 (LDAP filter syntax)

//! LDAP filter builder + scope enum. Port of Keycloak's
//! `LDAPQueryBuilder` — same fluent shape, plain `Filter`
//! variants serialised back to RFC 4515 `(&(...)(...))` strings.

use super::LdapError;

/// Search scope (RFC 4511 §4.5.1.2). `wholeSubtree` is the
/// Keycloak default for federation queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// `baseObject(0)` — examine only the base DN.
    Base,
    /// `singleLevel(1)` — base's immediate children.
    OneLevel,
    /// `wholeSubtree(2)` — base + everything below.
    Subtree,
}

impl Scope {
    /// Wire integer per RFC 4511 §4.5.1.2.
    pub fn as_raw(self) -> u32 {
        match self {
            Scope::Base => 0,
            Scope::OneLevel => 1,
            Scope::Subtree => 2,
        }
    }
}

/// LDAP filter AST (RFC 4515 §2). Subset Keycloak's
/// `LDAPQueryBuilder` actually emits — `=`, `~=`, `>=`, `<=`,
/// `present`, `&`, `|`, `!`, `substring`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    /// `(attr=value)` — equalityMatch.
    Equal { attr: String, value: String },
    /// `(attr~=value)` — approxMatch.
    Approx { attr: String, value: String },
    /// `(attr>=value)` — greaterOrEqual.
    Gte { attr: String, value: String },
    /// `(attr<=value)` — lessOrEqual.
    Lte { attr: String, value: String },
    /// `(attr=*)` — present.
    Present { attr: String },
    /// `(attr=*val*)` etc — substring filter.
    Substring {
        attr: String,
        /// Optional initial fragment before the first `*`.
        initial: Option<String>,
        /// Zero or more `*FRAG*` `any` parts.
        any: Vec<String>,
        /// Optional final fragment after the last `*`.
        finals: Option<String>,
    },
    /// `(&(...)(...)...)` — AND.
    And(Vec<Filter>),
    /// `(|(...)(...)...)` — OR.
    Or(Vec<Filter>),
    /// `(!(...))` — NOT.
    Not(Box<Filter>),
}

impl Filter {
    /// Render to an RFC 4515 string. Values are escaped per
    /// §3 (NUL `*` `(` `)` `\` all backslash-encoded).
    pub fn to_rfc4515(&self) -> String {
        match self {
            Filter::Equal { attr, value } => {
                format!("({}={})", attr, escape_value(value))
            }
            Filter::Approx { attr, value } => {
                format!("({}~={})", attr, escape_value(value))
            }
            Filter::Gte { attr, value } => {
                format!("({}>={})", attr, escape_value(value))
            }
            Filter::Lte { attr, value } => {
                format!("({}<={})", attr, escape_value(value))
            }
            Filter::Present { attr } => format!("({attr}=*)"),
            Filter::Substring {
                attr,
                initial,
                any,
                finals,
            } => {
                let mut s = String::from("(");
                s.push_str(attr);
                s.push('=');
                if let Some(i) = initial {
                    s.push_str(&escape_value(i));
                }
                s.push('*');
                for a in any {
                    s.push_str(&escape_value(a));
                    s.push('*');
                }
                if let Some(f) = finals {
                    s.push_str(&escape_value(f));
                }
                s.push(')');
                s
            }
            Filter::And(parts) => {
                let mut s = String::from("(&");
                for p in parts {
                    s.push_str(&p.to_rfc4515());
                }
                s.push(')');
                s
            }
            Filter::Or(parts) => {
                let mut s = String::from("(|");
                for p in parts {
                    s.push_str(&p.to_rfc4515());
                }
                s.push(')');
                s
            }
            Filter::Not(inner) => format!("(!{})", inner.to_rfc4515()),
        }
    }

    /// Parse an RFC 4515 filter string into the AST. Strict —
    /// rejects unbalanced parens / unknown operators.
    pub fn parse(input: &str) -> Result<Self, LdapError> {
        let mut p = Parser::new(input);
        let filter = p.parse_filter()?;
        if p.pos != input.len() {
            return Err(LdapError::FilterParse(format!(
                "trailing content at offset {}",
                p.pos
            )));
        }
        Ok(filter)
    }

    /// Match this filter against an attribute map — pure
    /// in-memory evaluation, used by [`crate::ldap::storage_provider`]'s
    /// `InMemoryDirectory` and by tests that don't talk to a real
    /// server. Strings compared case-insensitively per RFC 4517 §3
    /// `caseIgnoreMatch`.
    pub fn matches(&self, entry: &std::collections::BTreeMap<String, Vec<String>>) -> bool {
        match self {
            Filter::Equal { attr, value } => entry
                .get(attr)
                .map(|vs| {
                    vs.iter()
                        .any(|v| v.eq_ignore_ascii_case(value))
                })
                .unwrap_or(false),
            Filter::Approx { attr, value } => entry
                .get(attr)
                .map(|vs| {
                    vs.iter()
                        .any(|v| v.eq_ignore_ascii_case(value))
                })
                .unwrap_or(false),
            Filter::Gte { attr, value } => entry
                .get(attr)
                .map(|vs| vs.iter().any(|v| v.as_str() >= value.as_str()))
                .unwrap_or(false),
            Filter::Lte { attr, value } => entry
                .get(attr)
                .map(|vs| vs.iter().any(|v| v.as_str() <= value.as_str()))
                .unwrap_or(false),
            Filter::Present { attr } => entry.contains_key(attr),
            Filter::Substring {
                attr,
                initial,
                any,
                finals,
            } => entry
                .get(attr)
                .map(|vs| {
                    vs.iter().any(|v| {
                        let lv = v.to_ascii_lowercase();
                        let mut cursor = 0;
                        if let Some(i) = initial {
                            if !lv.starts_with(&i.to_ascii_lowercase()) {
                                return false;
                            }
                            cursor = i.len();
                        }
                        for a in any {
                            let needle = a.to_ascii_lowercase();
                            match lv[cursor..].find(&needle) {
                                Some(p) => cursor += p + needle.len(),
                                None => return false,
                            }
                        }
                        if let Some(f) = finals {
                            return lv.ends_with(&f.to_ascii_lowercase());
                        }
                        true
                    })
                })
                .unwrap_or(false),
            Filter::And(parts) => parts.iter().all(|p| p.matches(entry)),
            Filter::Or(parts) => parts.iter().any(|p| p.matches(entry)),
            Filter::Not(inner) => !inner.matches(entry),
        }
    }
}

fn escape_value(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for ch in v.chars() {
        match ch {
            '\0' => out.push_str("\\00"),
            '*' => out.push_str("\\2a"),
            '(' => out.push_str("\\28"),
            ')' => out.push_str("\\29"),
            '\\' => out.push_str("\\5c"),
            c => out.push(c),
        }
    }
    out
}

fn unescape_value(v: &str) -> Result<String, LdapError> {
    let bytes = v.as_bytes();
    let mut out = String::with_capacity(v.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            if i + 2 >= bytes.len() {
                return Err(LdapError::FilterParse(
                    "dangling backslash escape".into(),
                ));
            }
            let hi = hex_nibble(bytes[i + 1])?;
            let lo = hex_nibble(bytes[i + 2])?;
            out.push((hi << 4 | lo) as char);
            i += 3;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, LdapError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(LdapError::FilterParse(format!(
            "non-hex character in escape: {}",
            b as char
        ))),
    }
}

struct Parser<'a> {
    s: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(s: &'a str) -> Self {
        Parser { s, pos: 0 }
    }
    fn peek(&self) -> Option<char> {
        self.s[self.pos..].chars().next()
    }
    fn consume(&mut self, expected: char) -> Result<(), LdapError> {
        match self.peek() {
            Some(c) if c == expected => {
                self.pos += c.len_utf8();
                Ok(())
            }
            Some(other) => Err(LdapError::FilterParse(format!(
                "expected '{expected}' at {}, got '{other}'",
                self.pos
            ))),
            None => Err(LdapError::FilterParse(format!(
                "expected '{expected}' at end of input"
            ))),
        }
    }
    fn parse_filter(&mut self) -> Result<Filter, LdapError> {
        self.consume('(')?;
        let head = self.peek().ok_or_else(|| {
            LdapError::FilterParse("empty parenthesis group".into())
        })?;
        let f = match head {
            '&' => {
                self.pos += 1;
                let mut parts = Vec::new();
                while self.peek() == Some('(') {
                    parts.push(self.parse_filter()?);
                }
                if parts.is_empty() {
                    return Err(LdapError::FilterParse("empty AND".into()));
                }
                Filter::And(parts)
            }
            '|' => {
                self.pos += 1;
                let mut parts = Vec::new();
                while self.peek() == Some('(') {
                    parts.push(self.parse_filter()?);
                }
                if parts.is_empty() {
                    return Err(LdapError::FilterParse("empty OR".into()));
                }
                Filter::Or(parts)
            }
            '!' => {
                self.pos += 1;
                let inner = self.parse_filter()?;
                Filter::Not(Box::new(inner))
            }
            _ => self.parse_simple()?,
        };
        self.consume(')')?;
        Ok(f)
    }
    fn parse_simple(&mut self) -> Result<Filter, LdapError> {
        // attribute = chars up to one of `=`, `~`, `>`, `<`, `)`
        let start = self.pos;
        while let Some(c) = self.peek() {
            if matches!(c, '=' | '~' | '>' | '<' | ')') {
                break;
            }
            self.pos += c.len_utf8();
        }
        let attr = self.s[start..self.pos].to_owned();
        if attr.is_empty() {
            return Err(LdapError::FilterParse("missing attribute".into()));
        }
        // operator
        let op = self.peek().ok_or_else(|| {
            LdapError::FilterParse("missing operator".into())
        })?;
        let op_kind = match op {
            '=' => {
                self.pos += 1;
                "eq"
            }
            '~' => {
                self.pos += 1;
                self.consume('=')?;
                "approx"
            }
            '>' => {
                self.pos += 1;
                self.consume('=')?;
                "gte"
            }
            '<' => {
                self.pos += 1;
                self.consume('=')?;
                "lte"
            }
            other => {
                return Err(LdapError::FilterParse(format!(
                    "unknown operator '{other}'"
                )))
            }
        };
        // value runs to next `)`
        let v_start = self.pos;
        while let Some(c) = self.peek() {
            if c == ')' {
                break;
            }
            self.pos += c.len_utf8();
        }
        let raw_value = &self.s[v_start..self.pos];
        // substring detection — `=` operator with at least one `*`
        if op_kind == "eq" {
            if raw_value == "*" {
                return Ok(Filter::Present { attr });
            }
            if raw_value.contains('*') {
                let pieces: Vec<&str> = raw_value.split('*').collect();
                let initial = if pieces.first().map(|s| s.is_empty()) == Some(false) {
                    Some(unescape_value(pieces[0])?)
                } else {
                    None
                };
                let finals = if pieces.last().map(|s| s.is_empty()) == Some(false) {
                    Some(unescape_value(pieces[pieces.len() - 1])?)
                } else {
                    None
                };
                let any_start = if initial.is_some() { 1 } else { 1 };
                let any_end = if finals.is_some() {
                    pieces.len() - 1
                } else {
                    pieces.len()
                };
                let any = if any_end > any_start {
                    pieces[any_start..any_end]
                        .iter()
                        .map(|p| unescape_value(p))
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    Vec::new()
                };
                return Ok(Filter::Substring {
                    attr,
                    initial,
                    any,
                    finals,
                });
            }
        }
        let value = unescape_value(raw_value)?;
        Ok(match op_kind {
            "eq" => Filter::Equal { attr, value },
            "approx" => Filter::Approx { attr, value },
            "gte" => Filter::Gte { attr, value },
            "lte" => Filter::Lte { attr, value },
            _ => unreachable!(),
        })
    }
}

/// Fluent builder mirroring Keycloak's `LDAPQueryBuilder`. Pure
/// data-carrier — produces an [`LdapSearchSpec`].
#[derive(Debug, Clone)]
pub struct LdapQueryBuilder {
    pub base_dn: String,
    pub scope: Scope,
    pub filters: Vec<Filter>,
    pub attributes: Vec<String>,
    pub size_limit: u32,
    pub time_limit: u32,
}

impl LdapQueryBuilder {
    /// Matches `LDAPQueryBuilder.LDAPQueryBuilder(baseDn)`.
    pub fn new(base_dn: impl Into<String>) -> Self {
        LdapQueryBuilder {
            base_dn: base_dn.into(),
            scope: Scope::Subtree,
            filters: Vec::new(),
            attributes: Vec::new(),
            size_limit: 0,
            time_limit: 0,
        }
    }
    pub fn scope(mut self, scope: Scope) -> Self {
        self.scope = scope;
        self
    }
    pub fn add_filter(mut self, f: Filter) -> Self {
        self.filters.push(f);
        self
    }
    pub fn attributes<I: IntoIterator<Item = impl Into<String>>>(mut self, attrs: I) -> Self {
        self.attributes = attrs.into_iter().map(Into::into).collect();
        self
    }
    pub fn size_limit(mut self, n: u32) -> Self {
        self.size_limit = n;
        self
    }
    pub fn time_limit(mut self, n: u32) -> Self {
        self.time_limit = n;
        self
    }
    /// Materialise to a search spec — the structure that
    /// `LdapStorageProvider::search` accepts.
    pub fn build(self) -> LdapSearchSpec {
        let filter = match self.filters.len() {
            0 => Filter::Present {
                attr: "objectClass".into(),
            },
            1 => self.filters.into_iter().next().unwrap(),
            _ => Filter::And(self.filters),
        };
        LdapSearchSpec {
            base_dn: self.base_dn,
            scope: self.scope,
            filter,
            attributes: self.attributes,
            size_limit: self.size_limit,
            time_limit: self.time_limit,
        }
    }
}

/// Immutable search request — what `LdapStorageProvider::search`
/// consumes. Maps onto RFC 4511 §4.5.1 `SearchRequest`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LdapSearchSpec {
    pub base_dn: String,
    pub scope: Scope,
    pub filter: Filter,
    pub attributes: Vec<String>,
    pub size_limit: u32,
    pub time_limit: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_filter_to_rfc4515() {
        let f = Filter::Equal {
            attr: "uid".into(),
            value: "jdoe".into(),
        };
        assert_eq!(f.to_rfc4515(), "(uid=jdoe)");
    }

    #[test]
    fn and_filter_to_rfc4515() {
        let f = Filter::And(vec![
            Filter::Equal {
                attr: "objectClass".into(),
                value: "person".into(),
            },
            Filter::Equal {
                attr: "uid".into(),
                value: "jdoe".into(),
            },
        ]);
        assert_eq!(
            f.to_rfc4515(),
            "(&(objectClass=person)(uid=jdoe))"
        );
    }

    #[test]
    fn or_and_not_compose() {
        let f = Filter::Or(vec![
            Filter::Equal {
                attr: "uid".into(),
                value: "a".into(),
            },
            Filter::Not(Box::new(Filter::Equal {
                attr: "uid".into(),
                value: "b".into(),
            })),
        ]);
        assert_eq!(f.to_rfc4515(), "(|(uid=a)(!(uid=b)))");
    }

    #[test]
    fn present_filter_renders_with_asterisk() {
        let f = Filter::Present {
            attr: "cn".into(),
        };
        assert_eq!(f.to_rfc4515(), "(cn=*)");
    }

    #[test]
    fn substring_initial_only() {
        let f = Filter::Substring {
            attr: "cn".into(),
            initial: Some("J".into()),
            any: vec![],
            finals: None,
        };
        assert_eq!(f.to_rfc4515(), "(cn=J*)");
    }

    #[test]
    fn substring_with_middle_and_final() {
        let f = Filter::Substring {
            attr: "cn".into(),
            initial: Some("J".into()),
            any: vec!["oh".into()],
            finals: Some("Doe".into()),
        };
        assert_eq!(f.to_rfc4515(), "(cn=J*oh*Doe)");
    }

    #[test]
    fn parse_equal_filter() {
        assert_eq!(
            Filter::parse("(uid=jdoe)").unwrap(),
            Filter::Equal {
                attr: "uid".into(),
                value: "jdoe".into()
            }
        );
    }

    #[test]
    fn parse_present_filter() {
        assert_eq!(
            Filter::parse("(cn=*)").unwrap(),
            Filter::Present {
                attr: "cn".into()
            }
        );
    }

    #[test]
    fn parse_and_filter() {
        let f = Filter::parse("(&(objectClass=person)(uid=jdoe))").unwrap();
        assert_eq!(
            f,
            Filter::And(vec![
                Filter::Equal {
                    attr: "objectClass".into(),
                    value: "person".into()
                },
                Filter::Equal {
                    attr: "uid".into(),
                    value: "jdoe".into()
                },
            ])
        );
    }

    #[test]
    fn parse_substring_filter() {
        let f = Filter::parse("(cn=J*oh*Doe)").unwrap();
        match f {
            Filter::Substring {
                attr,
                initial,
                any,
                finals,
            } => {
                assert_eq!(attr, "cn");
                assert_eq!(initial.as_deref(), Some("J"));
                assert_eq!(any, vec!["oh".to_string()]);
                assert_eq!(finals.as_deref(), Some("Doe"));
            }
            _ => panic!("expected substring"),
        }
    }

    #[test]
    fn parse_roundtrips_complex_filter() {
        let input = "(&(objectClass=person)(|(uid=a)(!(uid=b))))";
        let f = Filter::parse(input).unwrap();
        assert_eq!(f.to_rfc4515(), input);
    }

    #[test]
    fn parse_rejects_unbalanced_parens() {
        assert!(Filter::parse("(uid=jdoe").is_err());
        assert!(Filter::parse("uid=jdoe)").is_err());
    }

    #[test]
    fn escape_handles_metacharacters() {
        let f = Filter::Equal {
            attr: "cn".into(),
            value: "J(o*hn)".into(),
        };
        assert_eq!(f.to_rfc4515(), "(cn=J\\28o\\2ahn\\29)");
    }

    #[test]
    fn matches_evaluates_equality() {
        let mut e = std::collections::BTreeMap::new();
        e.insert("uid".to_string(), vec!["JDoe".to_string()]);
        let f = Filter::Equal {
            attr: "uid".into(),
            value: "jdoe".into(),
        };
        assert!(f.matches(&e));
    }

    #[test]
    fn matches_evaluates_and() {
        let mut e = std::collections::BTreeMap::new();
        e.insert("uid".to_string(), vec!["jdoe".to_string()]);
        e.insert(
            "objectClass".to_string(),
            vec!["person".to_string()],
        );
        let f = Filter::And(vec![
            Filter::Equal {
                attr: "objectClass".into(),
                value: "person".into(),
            },
            Filter::Equal {
                attr: "uid".into(),
                value: "jdoe".into(),
            },
        ]);
        assert!(f.matches(&e));
    }

    #[test]
    fn matches_substring_case_insensitive() {
        let mut e = std::collections::BTreeMap::new();
        e.insert("cn".to_string(), vec!["John Doe".to_string()]);
        let f = Filter::Substring {
            attr: "cn".into(),
            initial: Some("j".into()),
            any: vec!["oh".into()],
            finals: Some("doe".into()),
        };
        assert!(f.matches(&e));
    }

    #[test]
    fn scope_raw_values_match_rfc4511() {
        assert_eq!(Scope::Base.as_raw(), 0);
        assert_eq!(Scope::OneLevel.as_raw(), 1);
        assert_eq!(Scope::Subtree.as_raw(), 2);
    }

    #[test]
    fn query_builder_aggregates_filters_into_and() {
        let q = LdapQueryBuilder::new("dc=example,dc=com")
            .scope(Scope::Subtree)
            .add_filter(Filter::Equal {
                attr: "objectClass".into(),
                value: "person".into(),
            })
            .add_filter(Filter::Equal {
                attr: "uid".into(),
                value: "jdoe".into(),
            })
            .build();
        match q.filter {
            Filter::And(parts) => assert_eq!(parts.len(), 2),
            _ => panic!("expected AND"),
        }
    }

    #[test]
    fn query_builder_single_filter_unwraps() {
        let q = LdapQueryBuilder::new("dc=example,dc=com")
            .add_filter(Filter::Present {
                attr: "uid".into(),
            })
            .build();
        assert!(matches!(q.filter, Filter::Present { .. }));
    }
}
