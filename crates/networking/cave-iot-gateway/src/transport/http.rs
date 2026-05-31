// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! HTTP transport codec — ThingsBoard HTTP device API path router.
//! (RED: router + key parser pending.)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_telemetry_post() {
        let r = route("POST", "/api/v1/MYTOKEN/telemetry").unwrap();
        assert_eq!(r.token, "MYTOKEN");
        assert_eq!(r.kind, HttpEndpointKind::Telemetry);
    }

    #[test]
    fn routes_attributes_post_and_get() {
        let post = route("POST", "/api/v1/T/attributes").unwrap();
        assert_eq!(post.kind, HttpEndpointKind::PostAttributes);
        let get = route("GET", "/api/v1/T/attributes").unwrap();
        assert_eq!(get.kind, HttpEndpointKind::GetAttributes);
    }

    #[test]
    fn routes_rpc_and_claim() {
        assert_eq!(
            route("POST", "/api/v1/T/rpc").unwrap().kind,
            HttpEndpointKind::Rpc
        );
        assert_eq!(
            route("POST", "/api/v1/T/claim").unwrap().kind,
            HttpEndpointKind::Claim
        );
    }

    #[test]
    fn routes_provision_without_token() {
        let r = route("POST", "/api/v1/provision").unwrap();
        assert_eq!(r.token, "");
        assert_eq!(r.kind, HttpEndpointKind::Provision);
    }

    #[test]
    fn rejects_unknown_path_and_method() {
        assert!(route("POST", "/api/v2/T/telemetry").is_err());
        assert!(route("DELETE", "/api/v1/T/telemetry").is_err());
        assert!(route("POST", "/nope").is_err());
    }

    #[test]
    fn parses_attribute_key_query() {
        let (client, shared) = parse_attribute_keys("clientKeys=a,b&sharedKeys=c");
        assert_eq!(client, vec!["a", "b"]);
        assert_eq!(shared, vec!["c"]);
        let (c2, s2) = parse_attribute_keys("sharedKeys=x");
        assert!(c2.is_empty());
        assert_eq!(s2, vec!["x"]);
    }

    #[test]
    fn parses_body_telemetry() {
        let kv = parse_body(br#"{"speed":55}"#).unwrap();
        assert_eq!(kv.get("speed"), Some(&crate::KvValue::Long(55)));
    }
}
