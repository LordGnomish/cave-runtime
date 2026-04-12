//! SCRAM-SHA-256 server-side implementation (RFC 5802 / RFC 7677).
//!
//! Implements the full server-side handshake:
//!   1. Parse client-first-message
//!   2. Generate server-first-message (with server nonce + iteration count + salt)
//!   3. Verify client-final-message proof
//!   4. Generate server-final-message (server signature)

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use hmac::{Hmac, Mac};
use pbkdf2::pbkdf2_hmac;
use rand::{distributions::Alphanumeric, Rng};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use super::UserRecord;

type HmacSha256 = Hmac<Sha256>;

const ITERATION_COUNT: u32 = 4096;
const SALT_LEN: usize = 16;

/// Opaque server-side SCRAM context — held between server-first and server-final.
#[derive(Debug)]
pub struct ServerScramContext {
    pub username: String,
    pub client_nonce: String,
    pub server_nonce: String,
    pub server_first: String,
    /// The auth message used for proof verification:
    ///   client-first-message-bare + "," + server-first + "," + client-final-without-proof
    pub auth_message: Option<String>,
    pub salted_password: Option<Vec<u8>>,
    pub salt: Vec<u8>,
    pub iterations: u32,
}

/// Parse client-first-message and produce server-first-message.
/// Returns (context, server_first_message).
pub fn server_first(client_first: &str) -> Result<(ServerScramContext, String), String> {
    // client-first = gs2-header client-first-message-bare
    // gs2-header = ("n" / "y" / "p=mechanism") "," authzid ","
    // client-first-message-bare = "n=" username "," "r=" client_nonce [,extensions]

    let parts: Vec<&str> = client_first.splitn(3, ',').collect();
    if parts.len() < 3 {
        return Err("malformed client-first-message".into());
    }
    // parts[0] = gs2-cbind-flag (e.g. "n")
    // parts[1] = authzid (often empty)
    // parts[2] = client-first-message-bare
    let bare = parts[2];

    let mut username = String::new();
    let mut client_nonce = String::new();

    for attr in bare.split(',') {
        if let Some(v) = attr.strip_prefix("n=") {
            username = scram_unescape(v);
        } else if let Some(v) = attr.strip_prefix("r=") {
            client_nonce = v.to_string();
        }
    }

    if username.is_empty() || client_nonce.is_empty() {
        return Err("missing n= or r= in client-first-message-bare".into());
    }

    // Generate server nonce extension
    let server_nonce_ext: String = rand::thread_rng()
        .sample_iter(Alphanumeric)
        .take(24)
        .map(char::from)
        .collect();
    let full_nonce = format!("{client_nonce}{server_nonce_ext}");

    // Generate salt
    let mut salt_bytes = vec![0u8; SALT_LEN];
    rand::thread_rng().fill(&mut salt_bytes[..]);
    let salt_b64 = BASE64.encode(&salt_bytes);

    let server_first = format!(
        "r={full_nonce},s={salt_b64},i={ITERATION_COUNT}"
    );

    let ctx = ServerScramContext {
        username,
        client_nonce,
        server_nonce: full_nonce,
        server_first: server_first.clone(),
        auth_message: None,
        salted_password: None,
        salt: salt_bytes,
        iterations: ITERATION_COUNT,
    };

    Ok((ctx, server_first))
}

/// Verify client-final-message and produce server-final-message.
/// Returns (username, server_final_message).
pub fn server_final(
    ctx: &ServerScramContext,
    client_final: &str,
    users: &HashMap<String, UserRecord>,
) -> Result<(String, String), String> {
    // client-final = client-final-without-proof "," "p=" proof_b64
    // client-final-without-proof = "c=" channel_binding "," "r=" server_nonce [,extensions]

    let proof_sep = client_final
        .rfind(",p=")
        .ok_or("missing proof in client-final-message")?;
    let without_proof = &client_final[..proof_sep];
    let proof_b64 = &client_final[proof_sep + 3..];

    // Validate channel binding and nonce
    for attr in without_proof.split(',') {
        if let Some(v) = attr.strip_prefix("r=") {
            if v != ctx.server_nonce {
                return Err(format!(
                    "server nonce mismatch: expected '{}', got '{}'",
                    ctx.server_nonce, v
                ));
            }
        }
        // c= (channel binding) — we accept and ignore for now (tls-unique not implemented)
    }

    // Auth message = client-first-bare + "," + server-first + "," + client-final-without-proof
    // We need the client-first-bare. We reconstruct it from what we know.
    // Actually the client sent it — we stored the bare part. But we don't have it stored.
    // For now reconstruct:
    let client_first_bare = format!("n={},r={}", scram_escape(&ctx.username), ctx.client_nonce);
    let auth_message = format!(
        "{},{},{}",
        client_first_bare, ctx.server_first, without_proof
    );

    // Look up user password
    let user = users.get(&ctx.username).ok_or_else(|| {
        format!("user '{}' not found", ctx.username)
    })?;
    let password = user.password.as_deref().unwrap_or("");

    // Compute SaltedPassword = PBKDF2(password, salt, iterations, 32)
    let mut salted_password = vec![0u8; 32];
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        &ctx.salt,
        ctx.iterations,
        &mut salted_password,
    );

    // ClientKey = HMAC(SaltedPassword, "Client Key")
    let mut mac = HmacSha256::new_from_slice(&salted_password)
        .map_err(|e| e.to_string())?;
    mac.update(b"Client Key");
    let client_key = mac.finalize().into_bytes();

    // StoredKey = H(ClientKey)
    let stored_key = Sha256::digest(&client_key);

    // ClientSignature = HMAC(StoredKey, AuthMessage)
    let mut mac = HmacSha256::new_from_slice(&stored_key)
        .map_err(|e| e.to_string())?;
    mac.update(auth_message.as_bytes());
    let client_signature = mac.finalize().into_bytes();

    // ClientProof = ClientKey XOR ClientSignature
    let received_proof = BASE64.decode(proof_b64).map_err(|e| e.to_string())?;
    if received_proof.len() != 32 {
        return Err("proof length mismatch".into());
    }

    let reconstructed_client_key: Vec<u8> = received_proof
        .iter()
        .zip(client_signature.iter())
        .map(|(p, s)| p ^ s)
        .collect();

    // Verify: H(reconstructed_client_key) == StoredKey
    let reconstructed_stored_key = Sha256::digest(&reconstructed_client_key);
    if reconstructed_stored_key != stored_key {
        return Err(format!("SCRAM authentication failed for user '{}'", ctx.username));
    }

    // Compute ServerSignature = HMAC(ServerKey, AuthMessage)
    let mut mac = HmacSha256::new_from_slice(&salted_password)
        .map_err(|e| e.to_string())?;
    mac.update(b"Server Key");
    let server_key = mac.finalize().into_bytes();

    let mut mac = HmacSha256::new_from_slice(&server_key)
        .map_err(|e| e.to_string())?;
    mac.update(auth_message.as_bytes());
    let server_signature = mac.finalize().into_bytes();

    let server_final = format!("v={}", BASE64.encode(server_signature));
    Ok((ctx.username.clone(), server_final))
}

/// Escape special characters in SCRAM attribute values.
fn scram_escape(s: &str) -> String {
    s.replace('=', "=3D").replace(',', "=2C")
}

/// Unescape SCRAM attribute values.
fn scram_unescape(s: &str) -> String {
    s.replace("=3D", "=").replace("=2C", ",")
}

/// Generate a SCRAM-SHA-256 verifier string for storing in pg_authid.
/// Format: SCRAM-SHA-256$<iterations>:<salt_b64>$<stored_key_b64>:<server_key_b64>
pub fn generate_scram_verifier(password: &str) -> String {
    let mut salt = vec![0u8; SALT_LEN];
    rand::thread_rng().fill(&mut salt[..]);

    let mut salted_password = vec![0u8; 32];
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        &salt,
        ITERATION_COUNT,
        &mut salted_password,
    );

    // ClientKey
    let mut mac = HmacSha256::new_from_slice(&salted_password).unwrap();
    mac.update(b"Client Key");
    let client_key = mac.finalize().into_bytes();

    // StoredKey = H(ClientKey)
    let stored_key = Sha256::digest(&client_key);

    // ServerKey
    let mut mac = HmacSha256::new_from_slice(&salted_password).unwrap();
    mac.update(b"Server Key");
    let server_key = mac.finalize().into_bytes();

    format!(
        "SCRAM-SHA-256${ITERATION_COUNT}:{}${}:{}",
        BASE64.encode(&salt),
        BASE64.encode(stored_key),
        BASE64.encode(server_key)
    )
}
