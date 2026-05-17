// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/protocol/wsfed/WSFedLoginProtocolService.java

//! Axum endpoints for `/protocol/wsfed/{realm}`. Mirrors Keycloak's
//! `WSFedLoginProtocolService` — one handler that dispatches on the
//! `wa` query parameter.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Router,
};

use super::protocol::{PassiveRst, Rstr};
use super::saml11_assertion::Saml11Assertion;
use super::signing::Saml11SignedAssertion;
use super::WsAction;

/// Shared state for the WS-Fed surface.
#[derive(Clone)]
pub struct WsFedState {
    /// IdP entity URI ("Issuer" on emitted assertions).
    pub issuer: String,
    /// PKCS#8-DER encoded RSA signing key. `None` disables signing
    /// (test mode only).
    pub signing_key_pkcs8_der: Option<Arc<Vec<u8>>>,
    /// Authentication callback — given the request, returns the
    /// authenticated subject + attributes, or `None` if the user is not
    /// yet authenticated (the handler then has to redirect to a login
    /// page; here we return 401 as a placeholder).
    pub authenticator: Arc<dyn Authenticator>,
}

/// Trait the host application implements to plug session state into the
/// WS-Fed endpoint. Returning `None` means "not yet signed in".
pub trait Authenticator: Send + Sync + std::fmt::Debug {
    fn authenticate(&self, realm: &str) -> Option<AuthenticatedSubject>;
}

/// Subject returned by [`Authenticator::authenticate`].
#[derive(Debug, Clone)]
pub struct AuthenticatedSubject {
    /// `<saml:NameIdentifier>` value (typically email).
    pub name_id: String,
    /// Flattened attribute map (e.g. `name`, `groups`).
    pub attributes: BTreeMap<String, Vec<String>>,
}

/// Build the WS-Fed router. Mount at `/protocol/wsfed`.
pub fn router(state: WsFedState) -> Router {
    Router::new()
        .route("/{realm}", get(handle_wsfed).post(handle_wsfed_post))
        .with_state(state)
}

async fn handle_wsfed(
    State(st): State<WsFedState>,
    Path(realm): Path<String>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    handle(&st, &realm, q)
}

async fn handle_wsfed_post(
    State(st): State<WsFedState>,
    Path(realm): Path<String>,
    Query(q): Query<BTreeMap<String, String>>,
) -> Response {
    handle(&st, &realm, q)
}

fn handle(st: &WsFedState, realm: &str, q: BTreeMap<String, String>) -> Response {
    let rst = match PassiveRst::from_query(&q) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("wsfed: {e}")).into_response(),
    };
    let action = match WsAction::from_str(&rst.wa) {
        Ok(a) => a,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("wsfed: {e}")).into_response(),
    };
    match action {
        WsAction::Signin => handle_signin(st, realm, &rst),
        WsAction::Signout => handle_signout(&rst),
        WsAction::SignoutCleanup => handle_signout_cleanup(),
    }
}

fn handle_signin(st: &WsFedState, realm: &str, rst: &PassiveRst) -> Response {
    let subject = match st.authenticator.authenticate(realm) {
        Some(s) => s,
        None => return (StatusCode::UNAUTHORIZED, "wsfed: not authenticated").into_response(),
    };
    let wtrealm = match &rst.wtrealm {
        Some(r) => r.clone(),
        None => return (StatusCode::BAD_REQUEST, "wsfed: wtrealm missing").into_response(),
    };
    let wreply = rst.wreply.clone().unwrap_or_else(|| wtrealm.clone());

    let mut assertion = Saml11Assertion::new(&st.issuer, &subject.name_id);
    assertion.audience = Some(wtrealm.clone());
    for (k, v) in &subject.attributes {
        assertion.add_attribute(k.clone(), v.clone());
    }
    let assertion_id = assertion.assertion_id.clone();
    let assertion_xml = match assertion.to_xml() {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("wsfed: {e}")).into_response(),
    };

    // Sign if a key is configured.
    let final_xml = if let Some(key) = &st.signing_key_pkcs8_der {
        let s = Saml11SignedAssertion::new(assertion_xml, &assertion_id);
        match s.sign(key) {
            Ok(x) => x,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("wsfed: sign: {e}")).into_response(),
        }
    } else {
        assertion_xml
    };

    let now = chrono::Utc::now();
    let rstr = Rstr {
        assertion_xml: final_xml,
        created: now,
        expires: now + chrono::Duration::minutes(5),
        applies_to: wtrealm.clone(),
    };
    let rstr_xml = match rstr.to_xml() {
        Ok(x) => x,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("wsfed: {e}")).into_response(),
    };

    // Build an HTML auto-POST form that submits wresult to wreply.
    let wctx = rst.wctx.as_deref().unwrap_or("");
    let html = format!(
        "<!DOCTYPE html><html><body onload=\"document.forms[0].submit()\">\
         <form method=\"POST\" action=\"{reply}\">\
         <input type=\"hidden\" name=\"wa\" value=\"wsignin1.0\"/>\
         <input type=\"hidden\" name=\"wresult\" value=\"{w}\"/>\
         <input type=\"hidden\" name=\"wctx\" value=\"{ctx}\"/>\
         <noscript><button type=\"submit\">Continue</button></noscript>\
         </form></body></html>",
        reply = html_escape_attr(&wreply),
        w = html_escape_attr(&rstr_xml),
        ctx = html_escape_attr(wctx),
    );
    Html(html).into_response()
}

fn handle_signout(rst: &PassiveRst) -> Response {
    let reply = rst.wreply.clone().unwrap_or_else(|| "/".into());
    Redirect::to(&reply).into_response()
}

fn handle_signout_cleanup() -> Response {
    // 1x1 transparent GIF — what AD FS expects in response to a
    // `wsignoutcleanup1.0` ping.
    static GIF: &[u8] = &[
        0x47, 0x49, 0x46, 0x38, 0x39, 0x61, 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0xFF, 0xFF,
        0xFF, 0x00, 0x00, 0x00, 0x21, 0xF9, 0x04, 0x01, 0x00, 0x00, 0x00, 0x00, 0x2C, 0x00, 0x00,
        0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02, 0x02, 0x44, 0x01, 0x00, 0x3B,
    ];
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "image/gif")],
        GIF,
    )
        .into_response()
}

fn html_escape_attr(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
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
        let req = Request::builder()
            .uri("/r1?wa=wsignin1.0&wtrealm=urn:rp")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_wa_returns_400() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wtrealm=urn:rp")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn unknown_wa_returns_400() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wattr1.0")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signin_missing_wtrealm_returns_400() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignin1.0")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn signin_returns_html_autopost_form() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignin1.0&wtrealm=urn:rp&wreply=https://rp.example/callback&wctx=opaque")
            .body(Body::empty())
            .unwrap();
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
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        // Signature block appears (HTML-escaped) in the wresult attribute.
        assert!(html.contains("&lt;ds:SignatureValue&gt;"));
    }

    #[tokio::test]
    async fn signout_redirects_to_wreply() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignout1.0&wreply=https://rp.example/post-signout")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        // 303 See Other or 307 / 308 acceptable; axum Redirect::to() uses 303.
        assert!(resp.status().is_redirection());
        let loc = resp.headers().get("location").unwrap().to_str().unwrap();
        assert_eq!(loc, "https://rp.example/post-signout");
    }

    #[tokio::test]
    async fn signout_cleanup_returns_gif() {
        let app = router(state(Some(alice())));
        let req = Request::builder()
            .uri("/r1?wa=wsignoutcleanup1.0")
            .body(Body::empty())
            .unwrap();
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
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000).await.unwrap();
        let html = std::str::from_utf8(&body).unwrap();
        assert!(html.contains("value=\"token123\""));
    }
}
