// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: keycloak/keycloak@v22.0.0 saml-core/src/main/java/org/keycloak/saml/processing/web/util/{RedirectBindingUtil,PostBindingUtil,ArtifactBindingUtil}.java
// SPDX-License-Identifier: AGPL-3.0-or-later
//
//! SAML 2.0 transport bindings — front-channel (Redirect, POST)
//! and back-channel (Artifact). This is the per-binding-as-a-module
//! factoring Keycloak ships in
//! `saml-core/.../processing/web/util/*BindingUtil.java`.
//!
//! ## Three bindings
//!
//! * [`http_redirect`] — DEFLATE + base64 + URL-encode. Used for
//!   AuthnRequests that fit inside a URL.
//! * [`http_post`]     — base64 only. Used for Responses that
//!   carry an assertion + signature.
//! * [`http_artifact`] — opaque 44-byte handle the receiver
//!   resolves via the back-channel ArtifactResolve SOAP call. The
//!   wire format itself is defined here; the actual SOAP resolution
//!   step lives in the broker (separate concern).
//!
//! The existing `saml::binding` module already implements the
//! first two as free functions; this module re-organises them
//! into per-binding sub-modules (matching upstream) and adds the
//! third. The existing free-function surface is preserved for
//! backwards compatibility — nothing here shadows it.

/// HTTP-Redirect binding (DEFLATE + base64).
pub mod http_redirect;

/// HTTP-POST binding (base64).
pub mod http_post;

/// HTTP-Artifact binding (back-channel handle).
pub mod http_artifact;

/// Spec URN for HTTP-Redirect.
pub const BINDING_REDIRECT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect";
/// Spec URN for HTTP-POST.
pub const BINDING_POST: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST";
/// Spec URN for HTTP-Artifact.
pub const BINDING_ARTIFACT: &str = "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Artifact";
/// Spec URN for SOAP back-channel (ArtifactResolve).
pub const BINDING_SOAP: &str = "urn:oasis:names:tc:SAML:2.0:bindings:SOAP";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn binding_urns_match_spec() {
        // Wire-format pins — if any of these change, every SP
        // integration breaks.
        assert_eq!(
            BINDING_REDIRECT,
            "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect"
        );
        assert_eq!(
            BINDING_POST,
            "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST"
        );
        assert_eq!(
            BINDING_ARTIFACT,
            "urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Artifact"
        );
        assert_eq!(BINDING_SOAP, "urn:oasis:names:tc:SAML:2.0:bindings:SOAP");
    }
}
