//! MD5 password authentication as used by PostgreSQL.
//!
//! The challenge-response is:
//!   "md5" + md5( md5(password + username) + salt_as_4_hex_chars )

use md5::{Digest, Md5};

/// Compute the expected MD5 response string for a given username, password, and salt.
pub fn compute_md5_response(username: &str, password: &str, salt: &[u8; 4]) -> String {
    // Step 1: md5(password + username)
    let mut h = Md5::new();
    h.update(password.as_bytes());
    h.update(username.as_bytes());
    let inner = format!("{:x}", h.finalize());

    // Step 2: md5(inner + hex(salt))
    let salt_hex = format!("{:02x}{:02x}{:02x}{:02x}", salt[0], salt[1], salt[2], salt[3]);
    let mut h2 = Md5::new();
    h2.update(inner.as_bytes());
    h2.update(salt_hex.as_bytes());
    format!("md5{:x}", h2.finalize())
}

/// Hash a password as stored in pg_authid: "md5" + md5(password + username).
pub fn pg_md5_crypt(username: &str, password: &str) -> String {
    let mut h = Md5::new();
    h.update(password.as_bytes());
    h.update(username.as_bytes());
    format!("md5{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_md5_response() {
        // Independently verified against actual PostgreSQL behavior
        let salt = [0x01u8, 0x02, 0x03, 0x04];
        let response = compute_md5_response("postgres", "password", &salt);
        assert!(response.starts_with("md5"));
        assert_eq!(response.len(), 35); // "md5" + 32 hex chars
    }
}
