// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/store/ldap/LDAPIdentityStore.java
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/main/java/org/keycloak/storage/ldap/idm/store/ldap/LDAPOperationManager.java
//
// LDAP SearchRequest / SearchResultEntry / SearchResultDone — RFC
// 4511 §4.5.  Includes RFC 2696 simple-paged-results control
// (`1.2.840.113556.1.4.319`) which Keycloak relies on for sync of
// directories with >1000 users.
//
//    SearchRequest ::= [APPLICATION 3] SEQUENCE {
//        baseObject      LDAPDN,
//        scope           ENUMERATED { baseObject(0), singleLevel(1), wholeSubtree(2) },
//        derefAliases    ENUMERATED { neverDerefAliases(0), derefInSearching(1),
//                                     derefFindingBaseObj(2), derefAlways(3) },
//        sizeLimit       INTEGER (0 .. maxInt),
//        timeLimit       INTEGER (0 .. maxInt),
//        typesOnly       BOOLEAN,
//        filter          Filter,
//        attributes      AttributeSelection }

use super::{
    ber::{self, integer, octet_string, sequence, Decoder, Element, Form, Tag},
    filter::Filter,
    object::{LdapAttribute, LdapObject},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Scope {
    BaseObject = 0,
    SingleLevel = 1,
    WholeSubtree = 2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum DerefAliases {
    Never = 0,
    InSearching = 1,
    FindingBaseObj = 2,
    Always = 3,
}

/// One SearchRequest payload.
#[derive(Debug, Clone)]
pub struct SearchRequest {
    pub message_id: u32,
    pub base_object: String,
    pub scope: Scope,
    pub deref_aliases: DerefAliases,
    pub size_limit: u32,
    pub time_limit: u32,
    pub types_only: bool,
    pub filter: Filter,
    pub attributes: Vec<String>,
    /// Optional paged-results control state.  `None` ⇒ unpaged.
    pub paged: Option<PagedResultsState>,
}

/// RFC 2696 control state.  `cookie` is empty on the first request
/// and echoed back from each `SearchResultDone`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PagedResultsState {
    pub size: u32,
    pub cookie: Vec<u8>,
}

impl SearchRequest {
    pub fn encode(&self) -> Vec<u8> {
        let scope_elem = ber::enumerated(self.scope as i64);
        let deref_elem = ber::enumerated(self.deref_aliases as i64);
        let size_elem = integer(self.size_limit as i64);
        let time_elem = integer(self.time_limit as i64);
        let types_only = ber::boolean(self.types_only);
        let filter_elem = self.filter.encode();
        let attrs = sequence(&self.attributes.iter().map(|a| octet_string(a.as_bytes())).collect::<Vec<_>>());

        let body = sequence(&[
            octet_string(self.base_object.as_bytes()),
            scope_elem,
            deref_elem,
            size_elem,
            time_elem,
            types_only,
            filter_elem,
            attrs,
        ]);
        let body = Element::new(Tag::application(3, Form::Constructed), body.bytes);

        let mut envelope_children = vec![integer(self.message_id as i64), body];
        if let Some(pr) = &self.paged {
            envelope_children.push(encode_paged_control(pr));
        }
        sequence(&envelope_children).encode()
    }
}

fn encode_paged_control(state: &PagedResultsState) -> Element {
    // RFC 2696 control: SEQUENCE { OID, criticality, controlValue }
    // controlValue is an OCTET STRING that wraps a BER SEQUENCE
    // { size INTEGER, cookie OCTET STRING }.
    let inner = sequence(&[integer(state.size as i64), octet_string(&state.cookie)]);
    let control = sequence(&[
        octet_string(b"1.2.840.113556.1.4.319"),
        ber::boolean(false),
        octet_string(&inner.encode()),
    ]);
    Element::new(Tag::context(0, Form::Constructed), vec![control].iter().fold(Vec::new(), |mut a, e| {
        a.extend_from_slice(&e.encode());
        a
    }))
}

/// A single SearchResultEntry (APPLICATION 4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResultEntry {
    pub message_id: u32,
    pub object: LdapObject,
}

impl SearchResultEntry {
    /// Encode for fixtures.
    pub fn encode(&self) -> Vec<u8> {
        let dn = octet_string(self.object.dn.as_bytes());
        let mut attr_seqs: Vec<Element> = Vec::new();
        for attr in self.object.attributes.values() {
            let vals = ber::set(&attr.values.iter().map(|v| octet_string(v)).collect::<Vec<_>>());
            let entry = sequence(&[octet_string(attr.name.as_bytes()), vals]);
            attr_seqs.push(entry);
        }
        let attrs_seq = sequence(&attr_seqs);
        let body = sequence(&[dn, attrs_seq]);
        let body = Element::new(Tag::application(4, Form::Constructed), body.bytes);
        sequence(&[integer(self.message_id as i64), body]).encode()
    }

    pub fn decode(frame: &[u8]) -> Result<Self, ber::DecodeError> {
        let mut d = Decoder::new(frame);
        let envelope = d.read_expected(Tag::universal(16, Form::Constructed))?;
        let mut e = Decoder::new(envelope);
        let message_id = e.read_integer()? as u32;
        let body = e.read_expected(Tag::application(4, Form::Constructed))?;
        let mut b = Decoder::new(body);
        let dn = b.read_octet_string_utf8()?;
        let attrs_payload = b.read_expected(Tag::universal(16, Form::Constructed))?;
        let mut ap = Decoder::new(attrs_payload);
        let mut obj = LdapObject::new(dn);
        while !ap.eof() {
            let one = ap.read_expected(Tag::universal(16, Form::Constructed))?;
            let mut od = Decoder::new(one);
            let name = od.read_octet_string_utf8()?;
            let set_payload = od.read_expected(Tag::universal(17, Form::Constructed))?;
            let mut sp = Decoder::new(set_payload);
            let mut attr = LdapAttribute::new(&name);
            while !sp.eof() {
                attr.values.push(sp.read_octet_string()?.to_vec());
            }
            // objectClass values feed the dedicated field too.
            if name.eq_ignore_ascii_case("objectClass") {
                obj.object_classes = attr.values.iter().filter_map(|v| std::str::from_utf8(v).ok().map(String::from)).collect();
            }
            obj.attributes.insert(name, attr);
        }
        Ok(Self { message_id, object: obj })
    }
}

/// SearchResultDone (APPLICATION 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchResultDone {
    pub message_id: u32,
    pub result_code: u32,
    pub diagnostic_message: String,
    pub paged_cookie: Option<Vec<u8>>,
}

impl SearchResultDone {
    pub fn encode(&self) -> Vec<u8> {
        let body = sequence(&[
            ber::enumerated(self.result_code as i64),
            octet_string(b""),
            octet_string(self.diagnostic_message.as_bytes()),
        ]);
        let body = Element::new(Tag::application(5, Form::Constructed), body.bytes);
        let mut envelope = vec![integer(self.message_id as i64), body];
        if let Some(cookie) = &self.paged_cookie {
            let inner = sequence(&[integer(0), octet_string(cookie)]);
            let ctrl_value = octet_string(&inner.encode());
            let ctrl = sequence(&[
                octet_string(b"1.2.840.113556.1.4.319"),
                ber::boolean(false),
                ctrl_value,
            ]);
            let controls = Element::new(Tag::context(0, Form::Constructed), ctrl.encode());
            envelope.push(controls);
        }
        sequence(&envelope).encode()
    }

    pub fn decode(frame: &[u8]) -> Result<Self, ber::DecodeError> {
        let mut d = Decoder::new(frame);
        let envelope = d.read_expected(Tag::universal(16, Form::Constructed))?;
        let mut e = Decoder::new(envelope);
        let message_id = e.read_integer()? as u32;
        let body = e.read_expected(Tag::application(5, Form::Constructed))?;
        let mut b = Decoder::new(body);
        let result_code = b.read_enumerated()? as u32;
        let _matched_dn = b.read_octet_string_utf8()?;
        let diagnostic_message = b.read_octet_string_utf8()?;

        let mut paged_cookie = None;
        if !e.eof() {
            // controls block.
            let (tag, payload) = e.read_tlv()?;
            if tag == Tag::context(0, Form::Constructed) {
                let mut cd = Decoder::new(payload);
                while !cd.eof() {
                    let ctrl_payload = cd.read_expected(Tag::universal(16, Form::Constructed))?;
                    let mut cp = Decoder::new(ctrl_payload);
                    let oid = cp.read_octet_string_utf8()?;
                    // optional criticality
                    if cp.peek_tag()? == Tag::universal(1, Form::Primitive) {
                        let _ = cp.read_tlv()?;
                    }
                    let value = cp.read_octet_string()?;
                    if oid == "1.2.840.113556.1.4.319" {
                        let mut vp = Decoder::new(value);
                        let inner = vp.read_expected(Tag::universal(16, Form::Constructed))?;
                        let mut ip = Decoder::new(inner);
                        let _size = ip.read_integer()?;
                        let cookie = ip.read_octet_string()?.to_vec();
                        paged_cookie = Some(cookie);
                    }
                }
            }
        }
        Ok(Self { message_id, result_code, diagnostic_message, paged_cookie })
    }
}

/// Build a Keycloak-shape user-search filter.  Mirrors
/// `LDAPQueryConditionsBuilder#equal` over multiple username
/// attributes plus the object-class gate.
pub fn user_lookup_filter(username_attr: &str, username: &str, object_classes: &[String]) -> Filter {
    let mut top = Vec::new();
    for oc in object_classes {
        top.push(Filter::equal("objectClass", oc.as_bytes()));
    }
    top.push(Filter::equal(username_attr, username.as_bytes()));
    Filter::And(top)
}

/// Build the mail/userPrincipalName/sAMAccountName fallback filter.
/// Keycloak surfaces this as `LDAPStorageProvider#searchForUser`.
pub fn multi_attr_user_filter(attrs: &[&str], value: &str, object_classes: &[String]) -> Filter {
    let or_branch = Filter::Or(attrs.iter().map(|a| Filter::equal(*a, value.as_bytes())).collect());
    let mut top = Vec::new();
    for oc in object_classes {
        top.push(Filter::equal("objectClass", oc.as_bytes()));
    }
    top.push(or_branch);
    Filter::And(top)
}

/// Cursor-style paged-results iterator.  The caller is expected to
/// drive the transport — this struct only holds the cookie + page
/// size + has_more flag, so it is fully testable without a network
/// socket.
#[derive(Debug, Clone)]
pub struct PagedIterator {
    pub page_size: u32,
    pub cookie: Vec<u8>,
    pub exhausted: bool,
}

impl PagedIterator {
    pub fn new(page_size: u32) -> Self {
        Self { page_size, cookie: Vec::new(), exhausted: false }
    }

    /// Take the cookie from a `SearchResultDone` and update state.
    /// Returns whether there is more to fetch.
    pub fn advance(&mut self, done: &SearchResultDone) -> bool {
        match &done.paged_cookie {
            Some(c) if !c.is_empty() => {
                self.cookie = c.clone();
                true
            }
            _ => {
                self.exhausted = true;
                false
            }
        }
    }

    pub fn next_state(&self) -> Option<PagedResultsState> {
        if self.exhausted {
            None
        } else {
            Some(PagedResultsState { size: self.page_size, cookie: self.cookie.clone() })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry_alice() -> SearchResultEntry {
        let mut obj = LdapObject::new("uid=alice,dc=acme,dc=corp");
        obj.set("cn", "Alice");
        obj.set("uid", "alice");
        obj.set("mail", "alice@acme.corp");
        obj.set("objectClass", "inetOrgPerson");
        SearchResultEntry { message_id: 7, object: obj }
    }

    #[test]
    fn search_result_entry_round_trip() {
        let e = entry_alice();
        let bytes = e.encode();
        let decoded = SearchResultEntry::decode(&bytes).unwrap();
        assert_eq!(decoded.message_id, 7);
        assert_eq!(decoded.object.dn, "uid=alice,dc=acme,dc=corp");
        assert_eq!(decoded.object.first_str("cn"), Some("Alice"));
        assert!(decoded.object.object_classes.contains(&"inetOrgPerson".to_string()));
    }

    #[test]
    fn search_result_done_paged_cookie_round_trip() {
        let d = SearchResultDone {
            message_id: 8,
            result_code: 0,
            diagnostic_message: String::new(),
            paged_cookie: Some(b"opaque-cookie".to_vec()),
        };
        let bytes = d.encode();
        let decoded = SearchResultDone::decode(&bytes).unwrap();
        assert_eq!(decoded.paged_cookie.as_deref(), Some(b"opaque-cookie".as_ref()));
    }

    #[test]
    fn paged_iterator_advances_then_exhausts() {
        let mut iter = PagedIterator::new(100);
        let done_more = SearchResultDone {
            message_id: 1,
            result_code: 0,
            diagnostic_message: String::new(),
            paged_cookie: Some(b"c1".to_vec()),
        };
        assert!(iter.advance(&done_more));
        assert_eq!(iter.cookie, b"c1");
        assert!(!iter.exhausted);
        let done_done = SearchResultDone {
            message_id: 2,
            result_code: 0,
            diagnostic_message: String::new(),
            paged_cookie: Some(Vec::new()),
        };
        assert!(!iter.advance(&done_done));
        assert!(iter.exhausted);
        assert!(iter.next_state().is_none());
    }

    #[test]
    fn user_lookup_filter_ands_object_class_with_username_attr() {
        let f = user_lookup_filter("uid", "alice", &["inetOrgPerson".into()]);
        if let Filter::And(parts) = f {
            assert_eq!(parts.len(), 2);
        } else {
            panic!("expected And");
        }
    }

    #[test]
    fn multi_attr_filter_or_branches_over_attrs() {
        let f = multi_attr_user_filter(&["mail", "uid"], "alice@acme.corp", &["user".into()]);
        if let Filter::And(parts) = f {
            assert_eq!(parts.len(), 2);
            if let Filter::Or(or) = &parts[1] {
                assert_eq!(or.len(), 2);
            } else {
                panic!("expected Or inside And");
            }
        } else {
            panic!("expected And");
        }
    }

    #[test]
    fn search_request_encodes_application_three() {
        let req = SearchRequest {
            message_id: 1,
            base_object: "dc=acme,dc=corp".into(),
            scope: Scope::WholeSubtree,
            deref_aliases: DerefAliases::Never,
            size_limit: 0,
            time_limit: 0,
            types_only: false,
            filter: Filter::Present("uid".into()),
            attributes: vec!["cn".into(), "mail".into()],
            paged: None,
        };
        let bytes = req.encode();
        // 0x63 = APPLICATION 3 CONSTRUCTED.
        assert!(bytes.contains(&0x63));
    }

    #[test]
    fn search_request_with_paged_carries_control_oid() {
        let req = SearchRequest {
            message_id: 2,
            base_object: "dc=acme,dc=corp".into(),
            scope: Scope::WholeSubtree,
            deref_aliases: DerefAliases::Never,
            size_limit: 0,
            time_limit: 0,
            types_only: false,
            filter: Filter::Present("uid".into()),
            attributes: vec![],
            paged: Some(PagedResultsState { size: 50, cookie: Vec::new() }),
        };
        let bytes = req.encode();
        assert!(bytes.windows(b"1.2.840.113556.1.4.319".len())
            .any(|w| w == b"1.2.840.113556.1.4.319"));
    }
}
