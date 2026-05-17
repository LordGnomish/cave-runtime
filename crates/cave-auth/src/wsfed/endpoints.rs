// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/wsfed/WSFedLoginProtocolService.java

//! WS-Fed endpoints — RED phase: tests authored, implementation lands in GREEN.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

#[derive(Clone)]
pub struct WsFedState {
    pub issuer: String,
    pub signing_key_pkcs8_der: Option<Arc<Vec<u8>>>,
    pub authenticator: Arc<dyn Authenticator>,
}

pub trait Authenticator: Send + Sync + std::fmt::Debug {
    fn authenticate(&self, realm: &str) -> Option<AuthenticatedSubject>;
}

#[derive(Debug, Clone)]
pub struct AuthenticatedSubject {
    pub name_id: String,
    pub attributes: BTreeMap<String, Vec<String>>,
}

pub fn router(state: WsFedState) -> Router {
    Router::new()
        .route("/{realm}", get(stub_get).post(stub_post))
        .with_state(state)
}

async fn stub_get(
    State(_st): State<WsFedState>,
    Path(_realm): Path<String>,
    Query(_q): Query<BTreeMap<String, String>>,
) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "RED-phase stub").into_response()
}
async fn stub_post(
    State(_st): State<WsFedState>,
    Path(_realm): Path<String>,
    Query(_q): Query<BTreeMap<String, String>>,
) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, "RED-phase stub").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[derive(Debug, Clone)]
    struct StubAuth(Option<AuthenticatedSubject>);
    impl Authenticator for StubAuth {
        fn authenticate(&self, _realm: &str) -> Option<AuthenticatedSubject> {
            self.0.clone()
        }
    }

    fn state(subject: Option<AuthenticatedSubject>) -> WsFedState {
        WsFedState {
            issuer: "https://idp.example/wsfed".into(),
            signing_key_pkcs8_der: None,
            authenticator: Arc::new(StubAuth(subject)),
        }
    }

    fn signed_state(subject: AuthenticatedSubject) -> WsFedState {
        use base64::engine::general_purpose::STANDARD as B64;
        use base64::Engine;
        const KEY_B64: &str = "MIIEvwIBADANBgkqhkiG9w0BAQEFAASCBKkwggSlAgEAAoIBAQCPegzMZl+1jHVMT0PW68K/qcIYqbBkkO6ooVUmxuDLFq0NIQmuteQ30RM06txbzpJdtBO/vAxOfcUBQ+jmKwixHC0JUcW6jixFfTOwFKIdeByzIRNoi1i/ZbrhhknLKZ3U2IQz4VwroyKbL2mFg5dPDA1oj1cJG4QODWLqbcjngRmExdM8remq+c6HGiI2TS0aldg3/wGBI5C+IyOeniVjzaFN/Z3GCqq9uC7Ij8spDoGBZPpskH8ehFLb6RsoxvVWJKJmB7LSNkabWXVLD+a+oqVO9ozMlV1R6qZZ4IUV7+lNS4BQp4Vla3RIKajjj2YKzGIl9UUEyH/A3SOlkqrrAgMBAAECggEANzZ8nlv3EOJQcWE/dgGcHC2zp9IFM24iqXoMTrPR5dWAGsFP/I+6l1A51+9ZhWrlIHIf93TiN4Jmwankgk6lNaLmIeP592Sm3MblkSkfib+jK7vawCx/pof7drY6x5foSPRZS625zoEk3BtOvDZ7j8vPjSE8GSEhnFbCbfx5h7yu4RqjBVEAz7feOMade++Qjn/IyfoNJ2Wq7oq/w7lXVYUNVIS7Ulj9cdTXIF6QFf+84B46d+YTsYiZRGMb/eZYk5IyXdv0vDg+qCD2mV+JYs1PD2qZOKemCxLjYs0OMYy1fKxbYVra4g0gOtTcnUYTJFixuyFifnfOyKKNrpbpgQKBgQDKRtc94O2t+Bah/bU4+90RNB4/lVmCRB0ExkMzJ/djT9TLYYFtnjX6DympmQ6ACzO8cqsArB2nEgbsXCV2lcwCldVY5/I9SuplyqtGKfPRdXlU3GopFNjZ/bdi1GF7MgRpD59yWWRHNN55HV94Eef/LumDOvTBtVu28jRWGJXYmwKBgQC1lUsoBAyBCnXkddsVIm5bqoi84CvcNC+nVRTcn8+x+GYb8o35RSSOymMQzlNd/b1YHzhi2b0R3vikSU/r3LtMdrWgdoIV6ElKgAwcbaoqIb/Zovh3qUXimIZvB8krR6a60QqJVw/1lTRnSuU82zV3ZncCSOJo64TZXmEdm47T8QKBgQCO/smG6w3bYHjPh8WnRRYg5VFE7dXbKz/AclBrR6Oxx2vNY17WGXRbFIEFbjg7+K9YV0/gJ8zGoQ3X5cRuMrOIWFf8g+xRvDY8Q6wU6+97caqWfUNnS1+Jq70K1s0bBF7tzqePdPZZCF0GDefBwBbb5VQa+4Cvt//gMxUgkDzOZQKBgQCRrjQ853qssJrC7vcUrqoBawEHH4awxUGSK0Vwd9qm+xXYyDG1Ug6xbJgsLIxf9SnKoEmZrPzucIflLlgrb8zo3Lh9A3b8Yn8igTa2PBlwceE8l25memzyDdKVE5cG3RZb/UhJxYqtScZgNItT1r6/i3phX94dtQ7BYeHiYiIl0QKBgQCCGN21FfQalMH2duGu7UQnZ03To0uDyn3zoaxxVK7M+9xB8bQ5rFq23ZOuGy1qYE7CitzGkCLf9goiJaNCowwUIKVsj+Joufxg1K9usyThr/OpWwQYNu1TOXpzBmKY1AnK+JVpUsRppc0BzpaPiDcnfi1Ch0ds0gVgPLUfflmX/A==";
        WsFedState {
            issuer: "https://idp.example/wsfed".into(),
            signing_key_pkcs8_der: Some(Arc::new(B64.decode(KEY_B64).unwrap())),
            authenticator: Arc::new(StubAuth(Some(subject))),
        }
    }

    fn alice() -> AuthenticatedSubject {
        let mut attrs = BTreeMap::new();
        attrs.insert("name".to_string(), vec!["Alice".to_string()]);
        attrs.insert("groups".to_string(), vec!["admins".to_string()]);
        AuthenticatedSubject {
            name_id: "alice@example.com".into(),
            attributes: attrs,
        }
    }

    #[tokio::test]
    async fn unauthenticated_returns_401() {
        let app = router(state(None));
        let req = Request::builder().uri("/r1?wa=wsignin1.0&wtrealm=urn:rp").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_wa_returns_400() {
        let app = router(state(Some(alice())));
        let req = Request::builder().uri("/r1?wtrealm=urn:rp").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_wa_returns_400() {
        let app = router(state(Some(alice())));
        let req = Request::builder().uri("/r1?wa=wattr1.0").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signin_missing_wtrealm_returns_400() {
        let app = router(state(Some(alice())));
        let req = Request::builder().uri("/r1?wa=wsignin1.0").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signin_returns_html_autopost_form() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignin1.0&wtrealm=urn:rp&wreply=https://rp.example/callback&wctx=opaque")
            .body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("action=\"https://rp.example/callback\""));
        assert!(html.contains("name=\"wresult\""));
        assert!(html.contains("name=\"wctx\""));
        assert!(html.contains("value=\"opaque\""));
        assert!(html.contains("wsignin1.0"));
    }

    #[tokio::test]
    async fn signin_form_carries_signed_assertion_when_key_configured() {
        let app = router(signed_state(alice()));
        let req = Request::builder()
            .uri("/r1?wa=wsignin1.0&wtrealm=urn:rp&wreply=https://rp/cb")
            .body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("&lt;ds:SignatureValue&gt;"));
    }

    #[tokio::test]
    async fn signout_redirects_to_wreply() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignout1.0&wreply=https://rp.example/post-signout")
            .body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert!(resp.status().is_redirection());
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(loc, "https://rp.example/post-signout");
    }

    #[tokio::test]
    async fn signout_cleanup_returns_gif() {
        let app = router(state(Some(alice())));
        let req = Request::builder().uri("/r1?wa=wsignoutcleanup1.0").body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get("content-type").unwrap().to_str().unwrap();
        assert_eq!(ct, "image/gif");
    }

    #[tokio::test]
    async fn signin_preserves_wctx_value() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignin1.0&wtrealm=urn:rp&wreply=https://rp.example/cb&wctx=token123")
            .body(Body::empty()).unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("value=\"token123\""));
    }
}
