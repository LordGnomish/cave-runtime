// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/query/internal/LDAPQueryConditionsBuilder.java
//
// RFC 4515 LDAP search-filter parser + BER encoder.  Keycloak builds
// filters as Java `String`s and JNDI parses + serialises them; we
// do both ourselves.  Supported subset (matches Keycloak's actual
// usage):
//
//   filter         = "(" filtercomp ")"
//   filtercomp     = and / or / not / item
//   and            = "&" filterlist
//   or             = "|" filterlist
//   not            = "!" filter
//   item           = simple / present / substring
//   simple         = attr filtertype value
//   filtertype     = "=" / "~=" / ">=" / "<="
//   present        = attr "=*"
//   substring      = attr "=" [initial] any [final]
//
// (Extensible match is _not_ supported — Keycloak never builds those
// dynamically.)

use super::ber::{octet_string, sequence, Element, Form, Tag};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Filter {
    And(Vec<Filter>),
    Or(Vec<Filter>),
    Not(Box<Filter>),
    Equal { attr: String, value: Vec<u8> },
    GreaterOrEqual { attr: String, value: Vec<u8> },
    LessOrEqual { attr: String, value: Vec<u8> },
    Present(String),
    /// Substring filter: optional `initial`, zero-or-more `any`s,
    /// optional `final_`.
    Substring { attr: String, initial: Option<Vec<u8>>, any: Vec<Vec<u8>>, final_: Option<Vec<u8>> },
    ApproxMatch { attr: String, value: Vec<u8> },
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum FilterError {
    #[error("unexpected end of filter")]
    Eof,
    #[error("unbalanced parenthesis")]
    Unbalanced,
    #[error("invalid filter operator near `{0}`")]
    BadOperator(String),
    #[error("missing comparison in `{0}`")]
    MissingComparison(String),
}

impl Filter {
    /// Build the [`Equal`] equivalent quickly — matches
    /// `EqualCondition` in Keycloak.
    pub fn equal(attr: impl Into<String>, value: impl Into<Vec<u8>>) -> Self {
        Filter::Equal { attr: attr.into(), value: value.into() }
    }

    pub fn present(attr: impl Into<String>) -> Self {
        Filter::Present(attr.into())
    }

    /// Parse RFC 4515.  Tolerant of whitespace inside attribute
    /// names but rejects unbalanced parens.
    pub fn parse(input: &str) -> Result<Self, FilterError> {
        let mut p = Parser { src: input.as_bytes(), pos: 0 };
        let f = p.parse_filter()?;
        if p.pos != p.src.len() {
            return Err(FilterError::Unbalanced);
        }
        Ok(f)
    }

    /// Serialize as RFC 4511 §4.5.1 Filter element.
    pub fn encode(&self) -> Element {
        match self {
            Filter::And(items) => {
                let body = items.iter().map(|f| f.encode()).collect::<Vec<_>>();
                let mut bytes = Vec::new();
                for e in &body {
                    bytes.extend_from_slice(&e.encode());
                }
                Element::new(Tag::context(0, Form::Constructed), bytes)
            }
            Filter::Or(items) => {
                let body = items.iter().map(|f| f.encode()).collect::<Vec<_>>();
                let mut bytes = Vec::new();
                for e in &body {
                    bytes.extend_from_slice(&e.encode());
                }
                Element::new(Tag::context(1, Form::Constructed), bytes)
            }
            Filter::Not(inner) => {
                let e = inner.encode();
                Element::new(Tag::context(2, Form::Constructed), e.encode())
            }
            Filter::Equal { attr, value } => {
                let inner = sequence(&[octet_string(attr.as_bytes()), octet_string(value)]);
                Element::new(Tag::context(3, Form::Constructed), inner.bytes)
            }
            Filter::Substring { attr, initial, any, final_ } => {
                let mut subs: Vec<Element> = Vec::new();
                if let Some(i) = initial {
                    subs.push(Element::new(Tag::context(0, Form::Primitive), i.clone()));
                }
                for a in any {
                    subs.push(Element::new(Tag::context(1, Form::Primitive), a.clone()));
                }
                if let Some(f) = final_ {
                    subs.push(Element::new(Tag::context(2, Form::Primitive), f.clone()));
                }
                let mut sub_bytes = Vec::new();
                for s in &subs {
                    sub_bytes.extend_from_slice(&s.encode());
                }
                let inner = sequence(&[
                    octet_string(attr.as_bytes()),
                    Element::new(Tag::universal(16, Form::Constructed), sub_bytes),
                ]);
                Element::new(Tag::context(4, Form::Constructed), inner.bytes)
            }
            Filter::GreaterOrEqual { attr, value } => {
                let inner = sequence(&[octet_string(attr.as_bytes()), octet_string(value)]);
                Element::new(Tag::context(5, Form::Constructed), inner.bytes)
            }
            Filter::LessOrEqual { attr, value } => {
                let inner = sequence(&[octet_string(attr.as_bytes()), octet_string(value)]);
                Element::new(Tag::context(6, Form::Constructed), inner.bytes)
            }
            Filter::Present(attr) => {
                Element::new(Tag::context(7, Form::Primitive), attr.as_bytes().to_vec())
            }
            Filter::ApproxMatch { attr, value } => {
                let inner = sequence(&[octet_string(attr.as_bytes()), octet_string(value)]);
                Element::new(Tag::context(8, Form::Constructed), inner.bytes)
            }
        }
    }
}

struct Parser<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }
    fn bump(&mut self) -> Option<u8> {
        let b = self.peek();
        if b.is_some() {
            self.pos += 1;
        }
        b
    }

    fn parse_filter(&mut self) -> Result<Filter, FilterError> {
        let lp = self.bump().ok_or(FilterError::Eof)?;
        if lp != b'(' {
            return Err(FilterError::BadOperator("expected (".into()));
        }
        let next = self.peek().ok_or(FilterError::Eof)?;
        let result = match next {
            b'&' => {
                self.bump();
                Filter::And(self.parse_filter_list()?)
            }
            b'|' => {
                self.bump();
                Filter::Or(self.parse_filter_list()?)
            }
            b'!' => {
                self.bump();
                let inner = self.parse_filter()?;
                Filter::Not(Box::new(inner))
            }
            _ => self.parse_item()?,
        };
        let rp = self.bump().ok_or(FilterError::Unbalanced)?;
        if rp != b')' {
            return Err(FilterError::Unbalanced);
        }
        Ok(result)
    }

    fn parse_filter_list(&mut self) -> Result<Vec<Filter>, FilterError> {
        let mut v = Vec::new();
        while self.peek() == Some(b'(') {
            v.push(self.parse_filter()?);
        }
        if v.is_empty() {
            return Err(FilterError::BadOperator("empty filter list".into()));
        }
        Ok(v)
    }

    fn parse_item(&mut self) -> Result<Filter, FilterError> {
        let mut attr = Vec::new();
        loop {
            match self.peek() {
                Some(b) if b != b'=' && b != b'~' && b != b'<' && b != b'>' && b != b')' => {
                    attr.push(self.bump().unwrap());
                }
                _ => break,
            }
        }
        let attr = String::from_utf8(attr).map_err(|_| FilterError::BadOperator("non-utf8 attr".into()))?;
        let op = self.bump().ok_or(FilterError::MissingComparison(attr.clone()))?;
        let op2 = if op == b'=' {
            None
        } else {
            // ~= >= <= each consume an extra '='
            let eq = self.bump().ok_or(FilterError::MissingComparison(attr.clone()))?;
            if eq != b'=' {
                return Err(FilterError::MissingComparison(attr));
            }
            Some(op)
        };
        // Read value until the closing ).
        let mut value = Vec::new();
        while let Some(b) = self.peek() {
            if b == b')' {
                break;
            }
            value.push(self.bump().unwrap());
        }
        match op2 {
            None => {
                // = could be Present (value == "*"), Substring (contains *),
                // or Equal.
                if value == b"*" {
                    Ok(Filter::Present(attr))
                } else if value.contains(&b'*') {
                    let mut parts: Vec<Vec<u8>> = Vec::new();
                    let mut cur = Vec::new();
                    for &b in &value {
                        if b == b'*' {
                            parts.push(std::mem::take(&mut cur));
                        } else {
                            cur.push(b);
                        }
                    }
                    parts.push(cur);
                    // parts.len() >= 2 because we know value contains '*'.
                    let initial = if parts[0].is_empty() { None } else { Some(parts[0].clone()) };
                    let final_ = if parts[parts.len() - 1].is_empty() {
                        None
                    } else {
                        Some(parts[parts.len() - 1].clone())
                    };
                    let any: Vec<Vec<u8>> = parts[1..parts.len() - 1].to_vec();
                    Ok(Filter::Substring { attr, initial, any, final_ })
                } else {
                    Ok(Filter::Equal { attr, value })
                }
            }
            Some(b'~') => Ok(Filter::ApproxMatch { attr, value }),
            Some(b'<') => Ok(Filter::LessOrEqual { attr, value }),
            Some(b'>') => Ok(Filter::GreaterOrEqual { attr, value }),
            Some(o) => Err(FilterError::BadOperator(format!("unknown op {o}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_equality() {
        assert_eq!(Filter::parse("(uid=alice)"), Ok(Filter::Equal { attr: "uid".into(), value: b"alice".to_vec() }));
    }

    #[test]
    fn parse_present_filter() {
        assert_eq!(Filter::parse("(mail=*)"), Ok(Filter::Present("mail".into())));
    }

    #[test]
    fn parse_substring_with_initial_and_final() {
        let f = Filter::parse("(cn=al*ce*Smith)").unwrap();
        match f {
            Filter::Substring { attr, initial, any, final_ } => {
                assert_eq!(attr, "cn");
                assert_eq!(initial, Some(b"al".to_vec()));
                assert_eq!(any, vec![b"ce".to_vec()]);
                assert_eq!(final_, Some(b"Smith".to_vec()));
            }
            other => panic!("expected substring, got {:?}", other),
        }
    }

    #[test]
    fn parse_and_with_two_children() {
        let f = Filter::parse("(&(objectClass=user)(uid=alice))").unwrap();
        match f {
            Filter::And(v) => {
                assert_eq!(v.len(), 2);
            }
            _ => panic!("expected And"),
        }
    }

    #[test]
    fn parse_not_negates_inner_filter() {
        let f = Filter::parse("(!(uid=bob))").unwrap();
        if let Filter::Not(inner) = f {
            assert_eq!(*inner, Filter::Equal { attr: "uid".into(), value: b"bob".to_vec() });
        } else {
            panic!("expected Not");
        }
    }

    #[test]
    fn parse_rejects_unbalanced() {
        assert!(matches!(Filter::parse("(uid=alice"), Err(FilterError::Unbalanced | FilterError::Eof)));
    }

    #[test]
    fn parse_ge_and_le() {
        assert_eq!(Filter::parse("(age>=18)"), Ok(Filter::GreaterOrEqual { attr: "age".into(), value: b"18".to_vec() }));
        assert_eq!(Filter::parse("(age<=65)"), Ok(Filter::LessOrEqual { attr: "age".into(), value: b"65".to_vec() }));
    }

    #[test]
    fn equal_encodes_with_context_tag_three() {
        let f = Filter::Equal { attr: "uid".into(), value: b"alice".to_vec() };
        let bytes = f.encode().encode();
        // [3] AttributeValueAssertion → 0b1010_0011 = 0xa3.
        assert_eq!(bytes[0], 0xa3);
    }

    #[test]
    fn present_encodes_with_primitive_context_seven() {
        let f = Filter::Present("mail".into());
        let bytes = f.encode().encode();
        // [7] PRIMITIVE → 0b1000_0111 = 0x87.
        assert_eq!(bytes[0], 0x87);
    }

    #[test]
    fn and_encodes_constructed_context_zero() {
        let f = Filter::And(vec![Filter::equal("uid", "x"), Filter::Present("cn".into())]);
        let bytes = f.encode().encode();
        // [0] CONSTRUCTED → 0b1010_0000 = 0xa0.
        assert_eq!(bytes[0], 0xa0);
    }
}
