//! Azure Shared Access Signature (Service-SAS + Account-SAS).
//!
//! Reference: <https://learn.microsoft.com/en-us/rest/api/storageservices/create-account-sas>.
//!
//! Service-SAS string-to-sign for blobs (api-version 2020-12-06+):
//!
//! ```text
//! signedPermissions + "\n" + signedStart + "\n" + signedExpiry + "\n" +
//! canonicalizedResource + "\n" + signedIdentifier + "\n" + signedIP + "\n" +
//! signedProtocol + "\n" + signedVersion + "\n" + signedResource + "\n" +
//! signedSnapshotTime + "\n" + signedEncryptionScope + "\n" +
//! rscc + "\n" + rscd + "\n" + rsce + "\n" + rscl + "\n" + rsct
//! ```
//!
//! The signature = `Base64(HMAC-SHA256(key, string_to_sign_utf8))`.

use base64::Engine;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, Copy, Default)]
pub struct SasPermissions {
    pub read: bool,
    pub add: bool,
    pub create: bool,
    pub write: bool,
    pub delete: bool,
    pub deletev: bool,
    pub list: bool,
    pub tag: bool,
    pub move_: bool,
    pub execute: bool,
}

impl SasPermissions {
    pub fn read_only() -> Self {
        Self {
            read: true,
            ..Default::default()
        }
    }

    pub fn read_write() -> Self {
        Self {
            read: true,
            write: true,
            create: true,
            add: true,
            ..Default::default()
        }
    }

    pub fn to_string(&self) -> String {
        // Order matches the Azure spec: r,a,c,w,d,x,l,t,m,e
        let mut s = String::new();
        if self.read { s.push('r'); }
        if self.add { s.push('a'); }
        if self.create { s.push('c'); }
        if self.write { s.push('w'); }
        if self.delete { s.push('d'); }
        if self.deletev { s.push('x'); }
        if self.list { s.push('l'); }
        if self.tag { s.push('t'); }
        if self.move_ { s.push('m'); }
        if self.execute { s.push('e'); }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SasToken {
    pub query_string: String,
    pub expires_at: DateTime<Utc>,
}

pub fn build_blob_sas(
    account: &str,
    container: &str,
    blob: &str,
    permissions: SasPermissions,
    starts: DateTime<Utc>,
    expires: DateTime<Utc>,
    account_key_b64: &str,
    api_version: &str,
) -> SasToken {
    let perms = permissions.to_string();
    let start_str = starts.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let expiry_str = expires.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let canonical_resource = format!("/blob/{}/{}/{}", account, container, blob);
    let signed_resource = "b"; // blob
    let signed_protocol = "https";

    let string_to_sign = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}",
        perms,
        start_str,
        expiry_str,
        canonical_resource,
        "",                    // signedIdentifier
        "",                    // signedIP
        signed_protocol,
        api_version,
        signed_resource,
        "",                    // signedSnapshotTime
        "",                    // signedEncryptionScope
        "",                    // rscc
        "",                    // rscd
        "",                    // rsce
        "",                    // rscl
        ""                     // rsct
    );

    let key = base64::engine::general_purpose::STANDARD
        .decode(account_key_b64)
        .unwrap_or_else(|_| account_key_b64.as_bytes().to_vec());
    let sig = hmac_sha256(&key, string_to_sign.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(sig);
    let qs = format!(
        "sv={}&ss=b&srt=o&sp={}&se={}&st={}&spr={}&sig={}",
        api_version,
        urlencode(&perms),
        urlencode(&expiry_str),
        urlencode(&start_str),
        signed_protocol,
        urlencode(&signature),
    );
    SasToken {
        query_string: qs,
        expires_at: expires,
    }
}

pub fn build_account_sas(
    account: &str,
    services: &str,
    resource_types: &str,
    permissions: SasPermissions,
    expires: DateTime<Utc>,
    account_key_b64: &str,
    api_version: &str,
) -> SasToken {
    let perms = permissions.to_string();
    let start_str = "";
    let expiry_str = expires.format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let signed_protocol = "https";
    // string-to-sign for account-SAS:
    let string_to_sign = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n{}\n",
        account, perms, services, resource_types, start_str, expiry_str, "",
        signed_protocol, api_version, "", ""
    );
    let key = base64::engine::general_purpose::STANDARD
        .decode(account_key_b64)
        .unwrap_or_else(|_| account_key_b64.as_bytes().to_vec());
    let sig = hmac_sha256(&key, string_to_sign.as_bytes());
    let signature = base64::engine::general_purpose::STANDARD.encode(sig);
    let qs = format!(
        "sv={}&ss={}&srt={}&sp={}&se={}&spr={}&sig={}",
        api_version,
        services,
        resource_types,
        urlencode(&perms),
        urlencode(&expiry_str),
        signed_protocol,
        urlencode(&signature),
    );
    SasToken {
        query_string: qs,
        expires_at: expires,
    }
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        let unreserved = b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~';
        if unreserved {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

fn hmac_sha256(key: &[u8], msg: &[u8]) -> Vec<u8> {
    const BLOCK: usize = 64;
    let mut k = if key.len() > BLOCK {
        let mut o = vec![0u8; BLOCK];
        let mut h = Sha256::new();
        h.update(key);
        let h = h.finalize();
        o[..32].copy_from_slice(&h);
        o
    } else {
        let mut o = vec![0u8; BLOCK];
        o[..key.len()].copy_from_slice(key);
        o
    };
    let mut o_pad = vec![0x5cu8; BLOCK];
    let mut i_pad = vec![0x36u8; BLOCK];
    for i in 0..BLOCK {
        o_pad[i] ^= k[i];
        i_pad[i] ^= k[i];
    }
    k.fill(0);
    let mut inner = Sha256::new();
    inner.update(&i_pad);
    inner.update(msg);
    let inner_h = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(&o_pad);
    outer.update(inner_h);
    outer.finalize().to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t(y: i32, m: u32, d: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, 0, 0, 0).unwrap()
    }

    #[test]
    fn permissions_string_in_canonical_order() {
        let p = SasPermissions {
            read: true,
            write: true,
            create: true,
            ..Default::default()
        };
        // canonical Azure order: r,a,c,w,d,x,l,t,m,e
        assert_eq!(p.to_string(), "rcw");
    }

    #[test]
    fn permissions_read_only_helper() {
        assert_eq!(SasPermissions::read_only().to_string(), "r");
    }

    #[test]
    fn permissions_read_write_helper() {
        assert_eq!(SasPermissions::read_write().to_string(), "racw");
    }

    #[test]
    fn permissions_empty_when_none_set() {
        assert_eq!(SasPermissions::default().to_string(), "");
    }

    #[test]
    fn permissions_all_set() {
        let p = SasPermissions {
            read: true,
            add: true,
            create: true,
            write: true,
            delete: true,
            deletev: true,
            list: true,
            tag: true,
            move_: true,
            execute: true,
        };
        assert_eq!(p.to_string(), "racwdxltme");
    }

    #[test]
    fn build_blob_sas_includes_signature() {
        let token = build_blob_sas(
            "myaccount",
            "mycontainer",
            "myblob.txt",
            SasPermissions::read_only(),
            t(2025, 1, 1),
            t(2025, 1, 2),
            "AAAAAAAA",
            "2020-12-06",
        );
        assert!(token.query_string.contains("sig="));
        assert!(token.query_string.contains("sv=2020-12-06"));
        assert!(token.query_string.contains("sp=r"));
        assert!(token.query_string.contains("spr=https"));
    }

    #[test]
    fn build_blob_sas_expires_at_matches() {
        let exp = t(2025, 6, 15);
        let token = build_blob_sas(
            "a", "c", "b", SasPermissions::read_only(), t(2025, 1, 1), exp, "AAAA", "2020-12-06",
        );
        assert_eq!(token.expires_at, exp);
    }

    #[test]
    fn build_blob_sas_signature_changes_with_key() {
        let t1 = build_blob_sas(
            "a", "c", "b", SasPermissions::read_only(), t(2025, 1, 1), t(2025, 1, 2), "AAAA", "2020-12-06",
        );
        let t2 = build_blob_sas(
            "a", "c", "b", SasPermissions::read_only(), t(2025, 1, 1), t(2025, 1, 2), "BBBB", "2020-12-06",
        );
        assert_ne!(t1.query_string, t2.query_string);
    }

    #[test]
    fn build_blob_sas_signature_changes_with_resource() {
        let t1 = build_blob_sas(
            "a", "c", "b1", SasPermissions::read_only(), t(2025, 1, 1), t(2025, 1, 2), "AAAA", "2020-12-06",
        );
        let t2 = build_blob_sas(
            "a", "c", "b2", SasPermissions::read_only(), t(2025, 1, 1), t(2025, 1, 2), "AAAA", "2020-12-06",
        );
        assert_ne!(t1.query_string, t2.query_string);
    }

    #[test]
    fn account_sas_includes_services_and_resource_types() {
        let token = build_account_sas(
            "myaccount",
            "b",
            "co",
            SasPermissions::read_only(),
            t(2025, 1, 2),
            "AAAA",
            "2020-12-06",
        );
        assert!(token.query_string.contains("ss=b"));
        assert!(token.query_string.contains("srt=co"));
    }

    #[test]
    fn urlencode_alpha_unchanged() {
        assert_eq!(urlencode("abc"), "abc");
    }

    #[test]
    fn urlencode_special_to_pct() {
        assert_eq!(urlencode("a+b"), "a%2Bb");
    }
}
