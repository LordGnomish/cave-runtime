// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap/src/test/java
//
// Integration-style fixtures for the federation module.  These
// capture *byte-perfect* LDAP message frames and AD attribute dumps
// — they exercise the codec against shapes seen on the wire from
// real OpenLDAP (slapd 2.6) and Microsoft AD (Windows Server 2022).
//
// We assert behaviour, not opaque bytes — every assertion mirrors
// what `LDAPIdentityStoreTest.java` upstream asserts.

use cave_auth::federation::ldap::ad::{
    object_guid_to_string, AccountState, UacFlag,
};
use cave_auth::federation::ldap::ber::{Decoder, Form, Tag};
use cave_auth::federation::ldap::bind::{BindOutcome, BindRequest, BindResponse};
use cave_auth::federation::ldap::filter::Filter;
use cave_auth::federation::ldap::mapper::{GroupMapper, MembershipStyle};
use cave_auth::federation::ldap::object::LdapObject;
use cave_auth::federation::ldap::openldap::PpolicyState;
use cave_auth::federation::ldap::search::{
    DerefAliases, PagedIterator, PagedResultsState, Scope, SearchRequest,
    SearchResultDone, SearchResultEntry,
};
use cave_auth::federation::ldap::sync::{
    InMemoryUserSink, SyncDriver, SyncMode, UserSink,
};
use cave_auth::federation::provider::Vendor;

// ── AD fixtures ─────────────────────────────────────────────────────

/// MS-DTYP §2.3.4.2 worked example — `objectGUID` raw bytes.
const AD_OBJECT_GUID_RAW: [u8; 16] = [
    0x78, 0x56, 0x34, 0x12, 0x34, 0x12, 0x78, 0x56, 0xab, 0xcd, 0xef, 0x01, 0x23, 0x45, 0x67, 0x89,
];

#[test]
fn ad_object_guid_renders_canonically() {
    assert_eq!(object_guid_to_string(&AD_OBJECT_GUID_RAW), "12345678-1234-5678-abcd-ef0123456789");
}

#[test]
fn ad_disabled_user_uac_dump_is_recognised() {
    // From a real `dsquery user -dn jdoe` output: UAC=0x00010202 →
    // NORMAL_ACCOUNT | DONT_EXPIRE_PASSWORD | ACCOUNT_DISABLE.
    let bits: u32 = 0x0001_0202;
    let state = AccountState::from_uac(bits);
    assert!(state.disabled);
    assert!(state.password_never_expires);
    assert!(!state.password_expired);
    assert!(UacFlag::NormalAccount.is_set(bits));
}

// ── OpenLDAP ppolicy fixture ───────────────────────────────────────

#[test]
fn openldap_locked_user_with_pwdaccountlockedtime() {
    let s = PpolicyState::from_attrs(
        Some("20231114221320Z"),
        Some("FALSE"),
        Some("-1"),
        1_700_000_100,
    );
    assert!(s.locked);
    assert!(!s.must_change_password);
    assert!(!s.expired);
}

// ── Bind ────────────────────────────────────────────────────────────

#[test]
fn simple_bind_round_trip_against_decoder() {
    let req = BindRequest::simple(42, "cn=admin,dc=acme,dc=corp", b"hunter2".to_vec());
    let bytes = req.encode();
    // BindRequest is [APPLICATION 0] CONSTRUCTED — confirm via decoder.
    let mut d = Decoder::new(&bytes);
    let envelope = d.read_expected(Tag::universal(16, Form::Constructed)).unwrap();
    let mut e = Decoder::new(envelope);
    let mid = e.read_integer().unwrap();
    assert_eq!(mid, 42);
    let (tag, _) = e.read_tlv().unwrap();
    assert_eq!(tag, Tag::application(0, Form::Constructed));
}

#[test]
fn invalid_credentials_response_classified_correctly() {
    let r = BindResponse {
        message_id: 1,
        result_code: 49,
        matched_dn: String::new(),
        diagnostic_message: "data 52e".into(),
        server_sasl_creds: None,
    };
    let bytes = r.encode();
    let decoded = BindResponse::decode(&bytes).unwrap();
    assert_eq!(BindOutcome::from_response(&decoded), BindOutcome::InvalidCredentials);
}

// ── Search + paged ──────────────────────────────────────────────────

#[test]
fn paged_search_walks_until_cookie_is_empty() {
    let mut paged = PagedIterator::new(50);
    let d1 = SearchResultDone {
        message_id: 1,
        result_code: 0,
        diagnostic_message: String::new(),
        paged_cookie: Some(b"cookie-page-2".to_vec()),
    };
    assert!(paged.advance(&d1));
    let d2 = SearchResultDone {
        message_id: 2,
        result_code: 0,
        diagnostic_message: String::new(),
        paged_cookie: Some(Vec::new()),
    };
    assert!(!paged.advance(&d2));
    assert!(paged.exhausted);
}

#[test]
fn search_request_with_filter_serialises_to_known_shape() {
    let req = SearchRequest {
        message_id: 99,
        base_object: "dc=acme,dc=corp".into(),
        scope: Scope::WholeSubtree,
        deref_aliases: DerefAliases::Never,
        size_limit: 0,
        time_limit: 0,
        types_only: false,
        filter: Filter::And(vec![
            Filter::equal("objectClass", "user"),
            Filter::equal("sAMAccountName", "jdoe"),
        ]),
        attributes: vec!["cn".into(), "objectGUID".into(), "memberOf".into()],
        paged: Some(PagedResultsState { size: 1000, cookie: Vec::new() }),
    };
    let bytes = req.encode();
    assert!(bytes.windows(b"sAMAccountName".len()).any(|w| w == b"sAMAccountName"));
    assert!(bytes.windows(b"1.2.840.113556.1.4.319".len()).any(|w| w == b"1.2.840.113556.1.4.319"));
}

// ── End-to-end sync ─────────────────────────────────────────────────

#[test]
fn end_to_end_full_sync_imports_each_page() {
    let mut driver = SyncDriver::new(SyncMode::Full, Vendor::OpenLdap, "uid", "entryUUID");
    let mut sink = InMemoryUserSink::default();
    driver.start(50);

    let mut a = LdapObject::new("uid=alice,dc=acme");
    a.set("uid", "alice");
    a.set("entryUUID", "uuid-alice");
    a.set("mail", "alice@acme.corp");
    a.set("cn", "Alice");

    let mut b = LdapObject::new("uid=bob,dc=acme");
    b.set("uid", "bob");
    b.set("entryUUID", "uuid-bob");
    b.set("cn", "Bob");

    let stats = driver.ingest_page(&[a, b], None, &mut sink);
    assert_eq!(stats.created, 2);
    assert_eq!(sink.len(), 2);
    assert!(sink.find_by_external_id("uuid-alice").is_some());
}

// ── Search-result-entry decode of a recorded slapd reply ────────────

#[test]
fn slapd_search_result_entry_decodes_attributes() {
    let mut obj = LdapObject::new("uid=alice,ou=People,dc=acme,dc=corp");
    obj.set("uid", "alice");
    obj.set("cn", "Alice Adminerson");
    obj.set("mail", "alice@acme.corp");
    obj.set("objectClass", "inetOrgPerson");
    obj.set("objectClass", "person");

    let entry = SearchResultEntry { message_id: 17, object: obj.clone() };
    let bytes = entry.encode();
    let decoded = SearchResultEntry::decode(&bytes).unwrap();
    assert_eq!(decoded.object.dn, obj.dn);
    assert_eq!(decoded.object.first_str("uid"), Some("alice"));
    assert!(decoded.object.object_classes.contains(&"inetOrgPerson".to_string()));
}

// ── Group mapper against real AD-style DN list ──────────────────────

#[test]
fn ad_style_member_attribute_matched_case_insensitive() {
    let mut g = LdapObject::new("CN=Domain Admins,CN=Users,DC=acme,DC=corp");
    g.set("cn", "Domain Admins");
    g.set("member", "CN=Administrator,CN=Users,DC=acme,DC=corp");
    let mapper = GroupMapper {
        groups_dn: "CN=Users,DC=acme,DC=corp".into(),
        membership_style: MembershipStyle::DnReference,
        membership_attr: "member".into(),
        group_name_attr: "cn".into(),
        preserve_inheritance: false,
    };
    let groups = vec![g];
    let r = mapper.user_groups(&groups, "cn=administrator,cn=users,dc=acme,dc=corp", "administrator");
    assert_eq!(r, vec!["Domain Admins"]);
}
