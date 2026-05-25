// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@v22.0.0 services/src/main/java/org/keycloak/federation/kerberos/impl/SPNEGOAuthenticator.java + libgssapi @ crates.io 0.7 (https://github.com/estokes/libgssapi) + RFC 4178/2743/2744

//! libgssapi server-side wiring. The pure-Rust ASN.1 layer in
//! [`super::gssapi`] / [`super::spnego`] decodes the wire format;
//! once we have the inner mech-specific blob, this module hands
//! it to the system GSSAPI implementation for cryptographic
//! ticket validation and principal extraction.
//!
//! ## Feature flag
//!
//! * `kerberos-gssapi` (off by default) — pulls in the
//!   `libgssapi` crate. Without it, [`accept_security_context`]
//!   returns [`GssapiError::FeatureDisabled`] so callers can
//!   short-circuit to the "GSSAPI not available" 401 response.
//!
//! ## Platform notes
//!
//! See [`PLATFORM_NOTES`].
//!
//! ## What this module covers
//!
//! * [`accept_security_context`] — the production path. Takes
//!   an AP-REQ (raw Kerberos v5 bytes, *not* the GSS wrapper —
//!   the caller strips the wrapper with
//!   [`super::gssapi::InitialContextToken::parse`] first) and
//!   returns either a fully-established context with the peer
//!   principal name, or a "continue-needed" output token that
//!   the caller must relay back to the client in a
//!   `WWW-Authenticate: Negotiate <b64-reply>` 401.
//! * [`init_security_context`] — client-side helper. Wraps the
//!   libgssapi `ClientCtx`. Production callers don't need this
//!   (cave-auth is a server); it exists for round-trip tests
//!   and for the rare case where cave acts as a Kerberos
//!   service client.
//!
//! ## Honest scope
//!
//! * **No Channel Bindings** — RFC 5929 channel bindings are
//!   ignored; if the deployment needs them, the caller passes
//!   the channel-binding token through a sidecar.
//! * **No InquireSecContext** — we extract only the peer
//!   principal name, not the full attribute bundle (lifetime /
//!   mech / locally-issued flag). The caller usually only
//!   needs the principal.
//! * **No credential delegation** — `GSS_C_DELEG_FLAG` is not
//!   requested. Cave doesn't impersonate the upstream client.

use std::path::{Path, PathBuf};

use thiserror::Error;

/// Documentation string surfaced via the binary and tests so
/// operators don't have to re-read this comment block. Keep
/// it short — the Cargo.toml comment carries the full rationale.
pub const PLATFORM_NOTES: &str = "\
libgssapi-sys bindgen targets MIT krb5. macOS Heimdal exposes a different\n\
symbol set (no gss_localname, gss_store_cred, gss_wrap_iov, gss_unwrap_iov);\n\
linking fails on Darwin. The `kerberos-gssapi` feature defaults to off so\n\
cargo build works everywhere. On Linux, install `libgssapi-krb5-2-dev` (apt)\n\
or `krb5-devel` (yum) then build with `--features kerberos-gssapi`.\n\
The pure-Rust ASN.1 modules (gssapi.rs / spnego.rs / negotiate.rs / keytab.rs)\n\
work on every host without the feature.\n";

/// Surface error type. Keeps the per-call failure mode visible
/// — `Gssapi(_)` is what a real libgssapi call returns;
/// `FeatureDisabled` lets the caller deliver a structured 401
/// when the binary was built without the feature.
#[derive(Debug, Error)]
pub enum GssapiError {
    #[error(
        "GSSAPI feature 'kerberos-gssapi' is disabled — rebuild with --features kerberos-gssapi to enable"
    )]
    FeatureDisabled,
    #[error("empty GSSAPI token")]
    EmptyToken,
    #[error("GSSAPI token exceeds 64 KiB limit ({0} bytes)")]
    OversizedToken(usize),
    #[error("keytab path does not exist or is unreadable: {0}")]
    KeytabUnavailable(PathBuf),
    #[error("GSSAPI: {0}")]
    Gssapi(String),
    #[error("KRB_AP_ERR: {0}")]
    Kerberos(String),
}

/// Output of a successful [`accept_security_context`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcceptedContext {
    /// Canonical Kerberos principal — e.g. `alice@EXAMPLE.COM`.
    pub peer_principal: String,
    /// Reply token to send back to the client. Populated when
    /// the client requested mutual auth (rare); usually `None`.
    pub output_token: Option<Vec<u8>>,
    /// `true` once the security context is fully established.
    /// `false` is returned via [`AcceptOutcome::ContinueNeeded`]
    /// — see that variant instead.
    pub complete: bool,
}

/// What [`accept_security_context`] returns. Modelled on the
/// libgssapi `Result<Option<Buf>, Error>` shape but with named
/// fields so the caller doesn't have to remember which `Option`
/// means "incomplete".
#[derive(Debug, Clone)]
pub enum AcceptOutcome {
    /// Handshake done — `peer_principal` is authoritative.
    Established(AcceptedContext),
    /// `gss_accept_sec_context` returned `GSS_S_CONTINUE_NEEDED`.
    /// Caller must base64-encode `output_token` and reply with
    /// `WWW-Authenticate: Negotiate <b64>` 401 so the client
    /// can produce the next token.
    ContinueNeeded { output_token: Vec<u8> },
}

/// Output of a successful [`init_security_context`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InitiatedContext {
    pub target_name: String,
    /// Token to send to the server.
    pub output_token: Vec<u8>,
    pub complete: bool,
}

/// Maximum GSSAPI token size. Keycloak's `KerberosUtil` uses
/// the same 64 KiB ceiling.
pub const MAX_TOKEN_SIZE: usize = 64 * 1024;

/// Accept-side of the GSSAPI handshake. The `input_token` must
/// be the raw mech-specific bytes (Kerberos AP-REQ), not the
/// `0x60 …` GSS wrapper; the caller strips that wrapper with
/// [`super::gssapi::InitialContextToken::parse`] / SPNEGO
/// [`super::spnego::NegTokenInit::parse`] beforehand.
///
/// `keytab_path` is the absolute path to the keytab holding
/// the server's principal credentials. Pass `None` to use the
/// default (`$KRB5_KTNAME` env-var, falling back to
/// `/etc/krb5.keytab`).
pub fn accept_security_context(
    input_token: &[u8],
    keytab_path: Option<&Path>,
) -> Result<AcceptOutcome, GssapiError> {
    if input_token.is_empty() {
        return Err(GssapiError::EmptyToken);
    }
    if input_token.len() > MAX_TOKEN_SIZE {
        return Err(GssapiError::OversizedToken(input_token.len()));
    }
    if let Some(p) = keytab_path {
        if !p.exists() {
            return Err(GssapiError::KeytabUnavailable(p.to_path_buf()));
        }
    }

    #[cfg(feature = "kerberos-gssapi")]
    {
        accept_security_context_impl(input_token, keytab_path)
    }
    #[cfg(not(feature = "kerberos-gssapi"))]
    {
        let _ = (input_token, keytab_path);
        Err(GssapiError::FeatureDisabled)
    }
}

/// Initiate-side helper. Cave is normally a server, so this is
/// mostly a test convenience. `target_name` is in canonical
/// Kerberos form — `service/host@REALM`.
pub fn init_security_context(target_name: &str) -> Result<InitiatedContext, GssapiError> {
    if target_name.is_empty() {
        return Err(GssapiError::Gssapi("empty target_name".into()));
    }

    #[cfg(feature = "kerberos-gssapi")]
    {
        init_security_context_impl(target_name)
    }
    #[cfg(not(feature = "kerberos-gssapi"))]
    {
        let _ = target_name;
        Err(GssapiError::FeatureDisabled)
    }
}

// ─── Real libgssapi implementation — only compiled when the system library is available ──

#[cfg(feature = "kerberos-gssapi")]
fn accept_security_context_impl(
    input_token: &[u8],
    keytab_path: Option<&Path>,
) -> Result<AcceptOutcome, GssapiError> {
    use libgssapi::context::{SecurityContext, ServerCtx};
    use libgssapi::credential::{Cred, CredUsage};
    use libgssapi::oid::{GSS_MECH_KRB5, OidSet};

    // Optional keytab override — set KRB5_KTNAME before acquiring credentials.
    let _guard = keytab_path.map(KrbKtnameGuard::set);

    let mech_set = OidSet::new().map_err(|e| GssapiError::Gssapi(format!("OidSet::new: {e}")))?;
    mech_set
        .add(&GSS_MECH_KRB5)
        .map_err(|e| GssapiError::Gssapi(format!("OidSet::add krb5: {e}")))?;

    // Server credentials — name=None means "use the default keytab principal".
    let cred = Cred::acquire(None, None, CredUsage::Accept, Some(&mech_set))
        .map_err(|e| GssapiError::Gssapi(format!("Cred::acquire (accept): {e}")))?;

    let mut server = ServerCtx::new(Some(cred));
    let out = server
        .step(input_token)
        .map_err(|e| GssapiError::Gssapi(format!("ServerCtx::step: {e}")))?;

    if server.is_complete() {
        let peer = server
            .source_name()
            .map_err(|e| GssapiError::Gssapi(format!("source_name: {e}")))?;
        let principal = format!("{}", peer);
        Ok(AcceptOutcome::Established(AcceptedContext {
            peer_principal: principal,
            output_token: out.map(|b| (*b).to_vec()),
            complete: true,
        }))
    } else {
        let out_token = out
            .map(|b| (*b).to_vec())
            .ok_or_else(|| GssapiError::Gssapi("CONTINUE_NEEDED but no output token".into()))?;
        Ok(AcceptOutcome::ContinueNeeded {
            output_token: out_token,
        })
    }
}

#[cfg(feature = "kerberos-gssapi")]
fn init_security_context_impl(target_name: &str) -> Result<InitiatedContext, GssapiError> {
    use libgssapi::context::{ClientCtx, CtxFlags, SecurityContext};
    use libgssapi::credential::{Cred, CredUsage};
    use libgssapi::name::Name;
    use libgssapi::oid::{GSS_MECH_KRB5, GSS_NT_KRB5_PRINCIPAL, OidSet};

    let name = Name::new(target_name.as_bytes(), Some(&GSS_NT_KRB5_PRINCIPAL))
        .map_err(|e| GssapiError::Gssapi(format!("Name::new: {e}")))?;
    let canonical = name
        .canonicalize(Some(&GSS_MECH_KRB5))
        .map_err(|e| GssapiError::Gssapi(format!("Name::canonicalize: {e}")))?;

    let mech_set = OidSet::new().map_err(|e| GssapiError::Gssapi(format!("OidSet::new: {e}")))?;
    mech_set
        .add(&GSS_MECH_KRB5)
        .map_err(|e| GssapiError::Gssapi(format!("OidSet::add krb5: {e}")))?;

    let cred = Cred::acquire(None, None, CredUsage::Initiate, Some(&mech_set))
        .map_err(|e| GssapiError::Gssapi(format!("Cred::acquire (initiate): {e}")))?;

    let mut client = ClientCtx::new(
        Some(cred),
        canonical,
        CtxFlags::GSS_C_MUTUAL_FLAG | CtxFlags::GSS_C_SEQUENCE_FLAG,
        Some(&GSS_MECH_KRB5),
    );
    let out = client
        .step(None, None)
        .map_err(|e| GssapiError::Gssapi(format!("ClientCtx::step: {e}")))?
        .ok_or_else(|| {
            GssapiError::Gssapi("ClientCtx::step returned no token on initiate".into())
        })?;

    Ok(InitiatedContext {
        target_name: target_name.to_string(),
        output_token: (*out).to_vec(),
        complete: client.is_complete(),
    })
}

/// RAII guard — sets `KRB5_KTNAME` while the GSSAPI call runs and
/// restores the previous value afterwards. libgssapi has no
/// "pass-keytab-path" API; `Cred::acquire(None, …)` reads
/// `KRB5_KTNAME` from the environment, so we override it for
/// the duration of the call.
#[cfg(feature = "kerberos-gssapi")]
struct KrbKtnameGuard {
    prev: Option<std::ffi::OsString>,
}

#[cfg(feature = "kerberos-gssapi")]
impl KrbKtnameGuard {
    fn set(path: &Path) -> Self {
        let prev = std::env::var_os("KRB5_KTNAME");
        // `FILE:` prefix is how MIT krb5 names a keytab type.
        let val = format!("FILE:{}", path.display());
        // SAFETY: tests are single-threaded by default and the env
        // is restored on Drop. Real callers should serialize calls
        // when overriding the keytab path.
        unsafe {
            std::env::set_var("KRB5_KTNAME", val);
        }
        Self { prev }
    }
}

#[cfg(feature = "kerberos-gssapi")]
impl Drop for KrbKtnameGuard {
    fn drop(&mut self) {
        // SAFETY: same single-threaded caveat as `set`.
        unsafe {
            match self.prev.take() {
                Some(v) => std::env::set_var("KRB5_KTNAME", v),
                None => std::env::remove_var("KRB5_KTNAME"),
            }
        }
    }
}

// ─── Unit tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_token_returns_empty_token_error() {
        let err = accept_security_context(&[], None).unwrap_err();
        assert!(matches!(err, GssapiError::EmptyToken), "got {err:?}");
    }

    #[test]
    fn oversized_token_returns_oversized_token_error() {
        let big = vec![0u8; MAX_TOKEN_SIZE + 1];
        let err = accept_security_context(&big, None).unwrap_err();
        assert!(matches!(err, GssapiError::OversizedToken(_)), "got {err:?}");
    }

    #[test]
    fn missing_keytab_returns_keytab_unavailable() {
        let kt = Path::new("/nonexistent/path/to/cave.keytab");
        let err = accept_security_context(&[0x01, 0x02], Some(kt)).unwrap_err();
        assert!(
            matches!(err, GssapiError::KeytabUnavailable(_)),
            "got {err:?}"
        );
    }

    #[test]
    fn feature_disabled_returns_feature_disabled_when_off() {
        if cfg!(not(feature = "kerberos-gssapi")) {
            let err = accept_security_context(&[0x01, 0x02], None).unwrap_err();
            assert!(matches!(err, GssapiError::FeatureDisabled));
        }
    }

    #[test]
    fn init_security_context_empty_target_rejected() {
        let err = init_security_context("").unwrap_err();
        match err {
            GssapiError::Gssapi(msg) => assert!(msg.contains("empty target_name")),
            GssapiError::FeatureDisabled => {} // also acceptable when feature off
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn platform_notes_constant_is_non_empty() {
        assert!(!PLATFORM_NOTES.is_empty());
        assert!(PLATFORM_NOTES.contains("libgssapi"));
    }

    #[test]
    fn accepted_context_struct_can_be_constructed() {
        let c = AcceptedContext {
            peer_principal: "alice@EXAMPLE.COM".into(),
            output_token: None,
            complete: true,
        };
        assert!(c.complete);
        assert_eq!(c.peer_principal, "alice@EXAMPLE.COM");
    }

    #[test]
    fn accept_outcome_continue_needed_carries_output() {
        let oc = AcceptOutcome::ContinueNeeded {
            output_token: vec![0xaa, 0xbb],
        };
        match oc {
            AcceptOutcome::ContinueNeeded { output_token } => {
                assert_eq!(output_token, vec![0xaa, 0xbb])
            }
            AcceptOutcome::Established(_) => panic!("wrong variant"),
        }
    }
}
