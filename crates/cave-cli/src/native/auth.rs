// SPDX-License-Identifier: AGPL-3.0-or-later
//! `cavectl auth` — authentication & authorization CLI subcommand.
//!
//! STAGED MODULE — ready to drop into `crates/cave-cli/src/cmd/auth.rs`
//! once the cave-runtime workspace re-mounts. Wire-up steps in cave-cli/main.rs:
//!
//!     mod auth;
//!     use auth::AuthCmd;
//!
//!     #[derive(Subcommand)]
//!     enum Commands {
//!         // …existing variants…
//!         /// Authentication & authorization
//!         Auth { #[command(subcommand)] cmd: AuthCmd },
//!     }
//!
//! Then in the dispatch match arm:
//!
//!     Commands::Auth { cmd } => auth::run(cmd, &client).await?,
//!
//! Backend assumed: cave-auth crate's keycloak module from
//! `feat/cave-auth-keycloak-mvp` (commit 33992e4..722fed9). Endpoints used:
//!   POST /realms/{realm}/protocol/openid-connect/token
//!   POST /realms/{realm}/protocol/openid-connect/logout
//!   GET  /realms/{realm}/protocol/openid-connect/userinfo
//!   GET  /admin/realms/{realm}
//!   GET  /admin/realms/{realm}/users
//!   POST/DELETE per the AdminRoot.java parity from cave-auth/keycloak/admin.rs

use clap::{Args, Subcommand, ValueEnum};

// ── Top-level subcommand ────────────────────────────────────────────────────

/// Authentication & authorization commands.
#[derive(Debug, Subcommand)]
pub enum AuthCmd {
    /// Acquire an access token via the chosen method.
    Login(LoginArgs),
    /// Revoke the current access + refresh tokens and clear the local cache.
    Logout(LogoutArgs),
    /// Print the current identity, roles, and token expiry.
    Whoami(WhoamiArgs),
    /// Token administration.
    #[command(subcommand)]
    Token(TokenCmd),
    /// Realm (tenant) inspection.
    #[command(subcommand)]
    Realm(RealmCmd),
    /// User management (admin scope required).
    #[command(subcommand)]
    Users(UsersCmd),
}

// ── login ───────────────────────────────────────────────────────────────────

/// Authentication method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum LoginMethod {
    /// Pass an existing PAT / service token via `--token`.
    Token,
    /// Username + password (Direct Access Grant, RFC 6749 §4.3).
    Userpass,
    /// OIDC authorization-code + PKCE (browser flow).
    Oidc,
    /// Mutual-TLS client certificate (RFC 8705).
    Cert,
}

#[derive(Debug, Args)]
pub struct LoginArgs {
    /// Authentication method.
    #[arg(long, value_enum, default_value_t = LoginMethod::Oidc)]
    pub method: LoginMethod,

    /// Realm (tenant). If omitted, falls back to $CAVE_REALM, then "master".
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,

    /// OAuth2 client_id. Defaults to "cavectl".
    #[arg(long, default_value = "cavectl")]
    pub client_id: String,

    /// PAT or pre-issued bearer (used with --method=token).
    #[arg(long, env = "CAVE_TOKEN")]
    pub token: Option<String>,

    /// Username (used with --method=userpass).
    #[arg(long, short = 'u')]
    pub username: Option<String>,

    /// Password — prompted interactively if omitted.
    #[arg(long, short = 'p', env = "CAVE_PASSWORD", hide_env_values = true)]
    pub password: Option<String>,

    /// Client certificate path (PEM). Used with --method=cert.
    #[arg(long, value_name = "FILE")]
    pub cert: Option<std::path::PathBuf>,

    /// Private-key path (PEM). Used with --method=cert.
    #[arg(long, value_name = "FILE")]
    pub key: Option<std::path::PathBuf>,

    /// Open the browser automatically (--method=oidc).
    #[arg(long, default_value_t = true)]
    pub open_browser: bool,

    /// Bypass the browser and print the URL (--method=oidc).
    #[arg(long, conflicts_with = "open_browser")]
    pub no_browser: bool,
}

// ── logout ──────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct LogoutArgs {
    /// Realm to log out of (defaults to current cached session realm).
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,
    /// Skip the server-side revoke call; just clear the local cache.
    #[arg(long)]
    pub local_only: bool,
}

// ── whoami ──────────────────────────────────────────────────────────────────

#[derive(Debug, Args)]
pub struct WhoamiArgs {
    /// Output format.
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
    /// Resolve roles + groups via /userinfo (extra round-trip).
    #[arg(long)]
    pub resolve: bool,
}

// ── token ───────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum TokenCmd {
    /// List active tokens (admin scope required).
    List(TokenListArgs),
    /// Revoke a specific token by id (jti).
    Revoke(TokenRevokeArgs),
}

#[derive(Debug, Args)]
pub struct TokenListArgs {
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,
    /// Filter by username.
    #[arg(long, short = 'u')]
    pub user: Option<String>,
    /// Show only tokens expiring within N seconds.
    #[arg(long)]
    pub expiring_within: Option<i64>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct TokenRevokeArgs {
    /// Token id (jti claim).
    pub id: String,
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,
    /// Suppress the confirmation prompt.
    #[arg(long, short = 'y')]
    pub yes: bool,
}

// ── realm ───────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum RealmCmd {
    /// Dump the realm configuration via the admin API.
    Info(RealmInfoArgs),
}

#[derive(Debug, Args)]
pub struct RealmInfoArgs {
    /// Realm name (overrides cached default).
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,
    /// Include client + role counts (extra admin round-trips).
    #[arg(long)]
    pub stats: bool,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

// ── users ───────────────────────────────────────────────────────────────────

#[derive(Debug, Subcommand)]
pub enum UsersCmd {
    /// List users in the realm (admin scope required).
    List(UsersListArgs),
    /// Show a single user by id or username.
    Get(UsersGetArgs),
}

#[derive(Debug, Args)]
pub struct UsersListArgs {
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,
    /// Substring filter on username / email.
    #[arg(long, short = 'q')]
    pub query: Option<String>,
    /// Page size (admin pagination).
    #[arg(long, default_value_t = 50)]
    pub limit: u32,
    #[arg(long, value_enum, default_value_t = OutputFormat::Table)]
    pub output: OutputFormat,
}

#[derive(Debug, Args)]
pub struct UsersGetArgs {
    /// User id OR username (auto-detected).
    pub id_or_username: String,
    #[arg(long, env = "CAVE_REALM")]
    pub realm: Option<String>,
    #[arg(long, value_enum, default_value_t = OutputFormat::Text)]
    pub output: OutputFormat,
}

// ── shared types ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Text,
    Table,
    Json,
    Yaml,
}

// ── Runtime dispatch ────────────────────────────────────────────────────────
//
// `run` translates each AuthCmd variant into a `PreparedRequest` (HTTP verb +
// path + JSON body) and lets the caller execute it. This mirrors the rest of
// the native verbs (cave-cli/src/native/get.rs, deploy.rs, …) so tests can
// assert request shape without spinning up an HTTP server.
//
// The actual HTTP hop is delegated to the supplied `client::ApiClient` (the
// same one main.rs already uses for every other Commands variant). Endpoints
// follow the cave-runtime convention `/api/auth/...` — cave-auth's keycloak
// module fronts the underlying `/realms/{realm}/protocol/openid-connect/...`
// surface, so the CLI never talks to Keycloak directly. Token caching for
// the browser-less flows lives at `~/.cave/auth.json`.
//
// Hard-error variants:
//   * Login{method=Cert} — mTLS via reqwest needs the client builder to be
//     reconfigured per-call with a rustls cert + key; that's outside the
//     scope of this dispatcher. Tracked as a follow-up.

use anyhow::{Result, anyhow};
use std::path::PathBuf;

use super::request::{HttpVerb, PreparedRequest};

const REALM_DEFAULT: &str = "master";

fn cache_path() -> Result<PathBuf> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| anyhow!("HOME is not set — cannot locate ~/.cave/auth.json"))?;
    Ok(PathBuf::from(home).join(".cave").join("auth.json"))
}

fn realm_or_default(realm: Option<&str>) -> &str {
    realm.unwrap_or(REALM_DEFAULT)
}

/// Persist the issued bearer token + realm to `~/.cave/auth.json`. Used by
/// `login --method=token` and the post-token-grant path of userpass/oidc.
fn write_session(realm: &str, token: &str) -> Result<()> {
    let path = cache_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let body = serde_json::json!({
        "realm": realm,
        "access_token": token,
        "issued_at": chrono::Utc::now().to_rfc3339(),
    });
    std::fs::write(&path, serde_json::to_vec_pretty(&body)?)?;
    Ok(())
}

/// Remove the local session cache. Idempotent.
fn clear_session() -> Result<()> {
    let path = cache_path()?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

impl AuthCmd {
    /// Translate the parsed command into the HTTP request the runtime
    /// expects. Returns `Ok(None)` for purely-local commands (e.g. `login
    /// --method=token` only stashes credentials, never hits the wire) and
    /// `Err(...)` for combinations the dispatcher cannot fulfil.
    pub fn prepare(&self) -> Result<Option<PreparedRequest>> {
        match self {
            AuthCmd::Login(args) => {
                match args.method {
                    LoginMethod::Token => {
                        let token = args
                            .token
                            .clone()
                            .ok_or_else(|| anyhow!("--token is required with --method=token"))?;
                        let realm = realm_or_default(args.realm.as_deref()).to_string();
                        write_session(&realm, &token)?;
                        Ok(None)
                    }
                    LoginMethod::Userpass => {
                        let username = args.username.as_deref().ok_or_else(|| {
                            anyhow!("--username is required with --method=userpass")
                        })?;
                        let password = args.password.as_deref().ok_or_else(|| {
                            anyhow!(
                                "--password is required with --method=userpass \
                                 (or set CAVE_PASSWORD)"
                            )
                        })?;
                        let realm = realm_or_default(args.realm.as_deref());
                        let body = serde_json::json!({
                            "grant_type": "password",
                            "client_id": args.client_id,
                            "username": username,
                            "password": password,
                        });
                        Ok(Some(
                            PreparedRequest::new(
                                HttpVerb::Post,
                                format!("/api/auth/realms/{realm}/protocol/openid-connect/token"),
                            )
                            .with_body(body),
                        ))
                    }
                    LoginMethod::Oidc => {
                        // The PKCE/auth-code dance has to happen out-of-band: the
                        // browser hits the IdP, redirects back with a code, and a
                        // separate `cavectl auth login --method=token --token=…`
                        // call seals the session. Print the authorization URL so
                        // the operator can paste it into a browser; no server hop
                        // happens here, hence no PreparedRequest.
                        let realm = realm_or_default(args.realm.as_deref());
                        let server = std::env::var("CAVE_SERVER")
                            .unwrap_or_else(|_| "http://localhost:3000".into());
                        let url = format!(
                            "{server}/api/auth/realms/{realm}/protocol/openid-connect/auth\
                         ?client_id={cid}&response_type=code\
                         &scope=openid+profile+email\
                         &redirect_uri=http://localhost:8765/callback",
                            cid = args.client_id,
                        );
                        eprintln!("OIDC authorization URL — open in a browser:\n  {url}");
                        eprintln!(
                            "After the IdP redirects with ?code=…&state=…, exchange via:\n  \
                         cavectl auth login --method=token --token <bearer-from-callback> \
                         --realm {realm}"
                        );
                        Ok(None)
                    }
                    LoginMethod::Cert => {
                        let cert = args.cert.as_deref().ok_or_else(|| {
                            anyhow!("--cert PEM path is required with --method=cert")
                        })?;
                        let _key = args.key.as_deref().ok_or_else(|| {
                            anyhow!("--key PEM path is required with --method=cert")
                        })?;
                        if !cert.exists() {
                            return Err(anyhow!("--cert path does not exist: {}", cert.display()));
                        }
                        Err(anyhow!(
                            "--method=cert (mTLS) requires reqwest to be reconstructed with a \
                         rustls Identity per call — not yet wired in cave-cli's shared \
                         ApiClient. Use --method=token with a cert-bound bearer for now."
                        ))
                    }
                }
            }

            AuthCmd::Logout(args) => {
                let realm = realm_or_default(args.realm.as_deref());
                if args.local_only {
                    clear_session()?;
                    return Ok(None);
                }
                Ok(Some(PreparedRequest::new(
                    HttpVerb::Post,
                    format!("/api/auth/realms/{realm}/protocol/openid-connect/logout"),
                )))
            }

            AuthCmd::Whoami(_args) => Ok(Some(PreparedRequest::new(
                HttpVerb::Get,
                "/api/auth/userinfo".to_string(),
            ))),

            AuthCmd::Token(TokenCmd::List(a)) => {
                let realm = realm_or_default(a.realm.as_deref());
                let mut path = format!("/api/auth/realms/{realm}/admin/tokens");
                let mut q: Vec<String> = vec![];
                if let Some(u) = &a.user {
                    q.push(format!("user={}", urlencode(u)));
                }
                if let Some(s) = a.expiring_within {
                    q.push(format!("expiring_within={s}"));
                }
                if !q.is_empty() {
                    path.push('?');
                    path.push_str(&q.join("&"));
                }
                Ok(Some(PreparedRequest::new(HttpVerb::Get, path)))
            }
            AuthCmd::Token(TokenCmd::Revoke(a)) => {
                let realm = realm_or_default(a.realm.as_deref());
                Ok(Some(PreparedRequest::new(
                    HttpVerb::Delete,
                    format!("/api/auth/realms/{realm}/admin/tokens/{id}", id = a.id),
                )))
            }

            AuthCmd::Realm(RealmCmd::Info(a)) => {
                let realm = realm_or_default(a.realm.as_deref());
                let path = if a.stats {
                    format!("/api/auth/realms/{realm}?stats=true")
                } else {
                    format!("/api/auth/realms/{realm}")
                };
                Ok(Some(PreparedRequest::new(HttpVerb::Get, path)))
            }

            AuthCmd::Users(UsersCmd::List(a)) => {
                let realm = realm_or_default(a.realm.as_deref());
                let mut path = format!("/api/auth/realms/{realm}/users?limit={lim}", lim = a.limit);
                if let Some(q) = &a.query {
                    path.push_str(&format!("&q={}", urlencode(q)));
                }
                Ok(Some(PreparedRequest::new(HttpVerb::Get, path)))
            }
            AuthCmd::Users(UsersCmd::Get(a)) => {
                let realm = realm_or_default(a.realm.as_deref());
                Ok(Some(PreparedRequest::new(
                    HttpVerb::Get,
                    format!(
                        "/api/auth/realms/{realm}/users/{id}",
                        id = urlencode(&a.id_or_username)
                    ),
                )))
            }
        }
    }

    /// Convenience wrapper: prepare + execute via the supplied client.
    /// Most-common dispatch entry point — main.rs calls this directly.
    pub async fn run(&self, client: &crate::client::ApiClient) -> Result<()> {
        match self.prepare()? {
            None => Ok(()),
            Some(req) => match req.verb {
                HttpVerb::Get => client.get(&req.path).await,
                HttpVerb::Post => {
                    let body = req
                        .body
                        .unwrap_or_else(|| serde_json::Value::Object(Default::default()));
                    let res = client.post(&req.path, body).await;
                    if matches!(self, AuthCmd::Logout(a) if !a.local_only) {
                        let _ = clear_session();
                    }
                    res
                }
                HttpVerb::Delete => client.delete(&req.path).await,
                HttpVerb::Put | HttpVerb::Patch => Err(anyhow!(
                    "auth dispatcher emitted {:?} but ApiClient does not support it yet",
                    req.verb
                )),
            },
        }
    }
}

/// Minimal RFC 3986 percent-encoding for query/path segments. Avoids pulling
/// the full `url` crate just for two callsites.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ────────────────────────────────────────────────────────────────────────────
// Tests — clap parse tests against a synthetic root command. Drop these into
// `crates/cave-cli/src/cmd/auth.rs` `#[cfg(test)] mod tests` block as-is.
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    /// Synthetic root used to drive the parser in isolation.
    #[derive(Parser, Debug)]
    #[command(name = "cavectl")]
    struct Root {
        #[command(subcommand)]
        cmd: AuthCmd,
    }

    fn parse(argv: &[&str]) -> Root {
        let mut full = vec!["cavectl"];
        full.extend_from_slice(argv);
        Root::try_parse_from(full).expect("parse must succeed")
    }

    fn try_parse(argv: &[&str]) -> Result<Root, clap::Error> {
        let mut full = vec!["cavectl"];
        full.extend_from_slice(argv);
        Root::try_parse_from(full)
    }

    // ── login parse tests ────────────────────────────────────────────────

    #[test]
    fn login_method_defaults_to_oidc() {
        let r = parse(&["login"]);
        match r.cmd {
            AuthCmd::Login(args) => assert_eq!(args.method, LoginMethod::Oidc),
            other => panic!("expected Login, got {other:?}"),
        }
    }

    #[test]
    fn login_token_flow_parses() {
        let r = parse(&["login", "--method", "token", "--token", "PAT123"]);
        match r.cmd {
            AuthCmd::Login(a) => {
                assert_eq!(a.method, LoginMethod::Token);
                assert_eq!(a.token.as_deref(), Some("PAT123"));
            }
            other => panic!("expected Login, got {other:?}"),
        }
    }

    #[test]
    fn login_userpass_with_short_flags() {
        let r = parse(&[
            "login", "--method", "userpass", "-u", "alice", "-p", "hunter2",
        ]);
        match r.cmd {
            AuthCmd::Login(a) => {
                assert_eq!(a.method, LoginMethod::Userpass);
                assert_eq!(a.username.as_deref(), Some("alice"));
                assert_eq!(a.password.as_deref(), Some("hunter2"));
            }
            other => panic!("expected Login, got {other:?}"),
        }
    }

    #[test]
    fn login_cert_method_takes_pem_paths() {
        let r = parse(&[
            "login",
            "--method",
            "cert",
            "--cert",
            "/etc/cave/client.pem",
            "--key",
            "/etc/cave/client.key",
            "--realm",
            "acme",
        ]);
        match r.cmd {
            AuthCmd::Login(a) => {
                assert_eq!(a.method, LoginMethod::Cert);
                assert_eq!(
                    a.cert.as_deref().and_then(|p| p.to_str()),
                    Some("/etc/cave/client.pem")
                );
                assert_eq!(
                    a.key.as_deref().and_then(|p| p.to_str()),
                    Some("/etc/cave/client.key")
                );
                assert_eq!(a.realm.as_deref(), Some("acme"));
            }
            other => panic!("expected Login, got {other:?}"),
        }
    }

    #[test]
    fn login_oidc_no_browser_conflicts_with_open_browser_default() {
        // --no-browser conflicts with --open-browser.
        // The default value for open_browser is true, but conflict only fires
        // when user explicitly passes BOTH flags.
        let ok = try_parse(&["login", "--method", "oidc", "--no-browser"]);
        assert!(ok.is_ok(), "no-browser alone parses");

        let bad = try_parse(&[
            "login",
            "--method",
            "oidc",
            "--open-browser",
            "--no-browser",
        ]);
        assert!(bad.is_err(), "explicit both flags must conflict");
    }

    // ── logout / whoami parse tests ──────────────────────────────────────

    #[test]
    fn logout_local_only_flag() {
        let r = parse(&["logout", "--local-only"]);
        match r.cmd {
            AuthCmd::Logout(a) => assert!(a.local_only),
            other => panic!("expected Logout, got {other:?}"),
        }
    }

    #[test]
    fn whoami_resolve_and_format() {
        let r = parse(&["whoami", "--resolve", "--output", "json"]);
        match r.cmd {
            AuthCmd::Whoami(a) => {
                assert!(a.resolve);
                assert_eq!(a.output, OutputFormat::Json);
            }
            other => panic!("expected Whoami, got {other:?}"),
        }
    }

    // ── token / realm / users parse tests ────────────────────────────────

    #[test]
    fn token_list_with_filter() {
        let r = parse(&["token", "list", "--user", "bob", "--expiring-within", "300"]);
        match r.cmd {
            AuthCmd::Token(TokenCmd::List(a)) => {
                assert_eq!(a.user.as_deref(), Some("bob"));
                assert_eq!(a.expiring_within, Some(300));
            }
            other => panic!("expected token list, got {other:?}"),
        }
    }

    #[test]
    fn token_revoke_requires_id() {
        let r = parse(&["token", "revoke", "abc123", "-y"]);
        match r.cmd {
            AuthCmd::Token(TokenCmd::Revoke(a)) => {
                assert_eq!(a.id, "abc123");
                assert!(a.yes);
            }
            other => panic!("expected token revoke, got {other:?}"),
        }
        // Missing positional id must fail.
        let bad = try_parse(&["token", "revoke"]);
        assert!(bad.is_err(), "missing id must error");
    }

    #[test]
    fn realm_info_with_stats() {
        let r = parse(&["realm", "info", "--stats", "--realm", "acme"]);
        match r.cmd {
            AuthCmd::Realm(RealmCmd::Info(a)) => {
                assert!(a.stats);
                assert_eq!(a.realm.as_deref(), Some("acme"));
            }
            other => panic!("expected realm info, got {other:?}"),
        }
    }

    #[test]
    fn users_list_with_query_and_limit() {
        let r = parse(&["users", "list", "-q", "alice", "--limit", "10"]);
        match r.cmd {
            AuthCmd::Users(UsersCmd::List(a)) => {
                assert_eq!(a.query.as_deref(), Some("alice"));
                assert_eq!(a.limit, 10);
            }
            other => panic!("expected users list, got {other:?}"),
        }
    }

    #[test]
    fn users_get_positional_id_or_username() {
        let r = parse(&["users", "get", "alice"]);
        match r.cmd {
            AuthCmd::Users(UsersCmd::Get(a)) => {
                assert_eq!(a.id_or_username, "alice");
            }
            other => panic!("expected users get, got {other:?}"),
        }
    }

    // ── unknown subcommand rejected ──────────────────────────────────────

    #[test]
    fn unknown_subcommand_rejected() {
        let bad = try_parse(&["wat"]);
        assert!(bad.is_err());
    }
}
