// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: webauthn4j@v0.24.0 + keycloak@v22.0.0 services/.../webauthn + W3C WebAuthn L2
//
// `cavectl auth webauthn {register-options,verify-attestation,assert-options,verify-assertion}`
//
// Parse-only stub — full HTTP wiring lands when the cave-portal A6 page is
// merged. The parse surface is exercised by tests below.

use std::fmt;

/// Parsed sub-command — what the user typed after `cavectl auth webauthn`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WebAuthnCmd {
    /// `cavectl auth webauthn register-options --user-id alice`
    RegisterOptions { user_id: String },
    /// `cavectl auth webauthn verify-attestation --challenge ABC --resp-file r.json`
    VerifyAttestation {
        challenge_b64: String,
        response_path: String,
    },
    /// `cavectl auth webauthn assert-options --user-id alice`
    AssertOptions { user_id: String },
    /// `cavectl auth webauthn verify-assertion --challenge ABC --resp-file r.json`
    VerifyAssertion {
        challenge_b64: String,
        response_path: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    MissingSubcommand,
    UnknownSubcommand(String),
    MissingFlag(&'static str),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingSubcommand => write!(f, "expected one of: register-options, verify-attestation, assert-options, verify-assertion"),
            Self::UnknownSubcommand(s) => write!(f, "unknown subcommand {s:?}"),
            Self::MissingFlag(flag) => write!(f, "missing required --{flag}"),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse `cavectl auth webauthn <subcommand> [flags…]` argv.  The first token
/// is the subcommand; subsequent tokens are flag pairs.  This is intentionally
/// minimal — the binary's clap dispatch will replace it once the surface
/// stabilises.
pub fn parse(argv: &[String]) -> Result<WebAuthnCmd, ParseError> {
    let mut iter = argv.iter();
    let sub = iter.next().ok_or(ParseError::MissingSubcommand)?;
    let mut user_id: Option<String> = None;
    let mut challenge: Option<String> = None;
    let mut resp_file: Option<String> = None;
    while let Some(flag) = iter.next() {
        match flag.as_str() {
            "--user-id" => user_id = iter.next().cloned(),
            "--challenge" => challenge = iter.next().cloned(),
            "--resp-file" => resp_file = iter.next().cloned(),
            _ => {}
        }
    }
    match sub.as_str() {
        "register-options" => Ok(WebAuthnCmd::RegisterOptions {
            user_id: user_id.ok_or(ParseError::MissingFlag("user-id"))?,
        }),
        "verify-attestation" => Ok(WebAuthnCmd::VerifyAttestation {
            challenge_b64: challenge.ok_or(ParseError::MissingFlag("challenge"))?,
            response_path: resp_file.ok_or(ParseError::MissingFlag("resp-file"))?,
        }),
        "assert-options" => Ok(WebAuthnCmd::AssertOptions {
            user_id: user_id.ok_or(ParseError::MissingFlag("user-id"))?,
        }),
        "verify-assertion" => Ok(WebAuthnCmd::VerifyAssertion {
            challenge_b64: challenge.ok_or(ParseError::MissingFlag("challenge"))?,
            response_path: resp_file.ok_or(ParseError::MissingFlag("resp-file"))?,
        }),
        other => Err(ParseError::UnknownSubcommand(other.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(toks: &[&str]) -> Vec<String> {
        toks.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parse_register_options_happy() {
        let cmd = parse(&args(&["register-options", "--user-id", "alice"])).unwrap();
        assert_eq!(
            cmd,
            WebAuthnCmd::RegisterOptions {
                user_id: "alice".into()
            }
        );
    }

    #[test]
    fn parse_verify_attestation_happy() {
        let cmd = parse(&args(&[
            "verify-attestation",
            "--challenge",
            "QUJD",
            "--resp-file",
            "/tmp/r.json",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            WebAuthnCmd::VerifyAttestation {
                challenge_b64: "QUJD".into(),
                response_path: "/tmp/r.json".into(),
            }
        );
    }

    #[test]
    fn parse_assert_then_verify_pair() {
        assert!(matches!(
            parse(&args(&["assert-options", "--user-id", "u"])).unwrap(),
            WebAuthnCmd::AssertOptions { .. }
        ));
        assert!(matches!(
            parse(&args(&[
                "verify-assertion",
                "--challenge",
                "AAAA",
                "--resp-file",
                "x"
            ]))
            .unwrap(),
            WebAuthnCmd::VerifyAssertion { .. }
        ));
    }

    #[test]
    fn parse_unknown_subcommand_errors() {
        let err = parse(&args(&["bogus"])).unwrap_err();
        assert!(matches!(err, ParseError::UnknownSubcommand(_)));
    }
}
