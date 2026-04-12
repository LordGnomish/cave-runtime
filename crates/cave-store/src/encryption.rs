use ring::digest::{digest, SHA256};

pub struct EncryptionEngine;

fn derive_key(key_id: &str) -> Vec<u8> {
    let hash = digest(&SHA256, key_id.as_bytes());
    hash.as_ref().to_vec()
}

fn xor_encrypt(data: &[u8], key: &[u8]) -> Vec<u8> {
    if key.is_empty() {
        return data.to_vec();
    }
    data.iter()
        .enumerate()
        .map(|(i, &b)| b ^ key[i % key.len()])
        .collect()
}

impl EncryptionEngine {
    /// SSE-S3: XOR with a key derived from key_id (simplified, not real AES).
    pub fn encrypt_sse_s3(data: &[u8], key_id: &str) -> Vec<u8> {
        let key = derive_key(key_id);
        xor_encrypt(data, &key)
    }

    pub fn decrypt_sse_s3(data: &[u8], key_id: &str) -> Vec<u8> {
        // XOR is symmetric
        let key = derive_key(key_id);
        xor_encrypt(data, &key)
    }

    /// SSE-C: XOR with customer-provided key.
    pub fn encrypt_sse_c(data: &[u8], customer_key: &[u8]) -> Vec<u8> {
        xor_encrypt(data, customer_key)
    }

    pub fn decrypt_sse_c(data: &[u8], customer_key: &[u8]) -> Vec<u8> {
        xor_encrypt(data, customer_key)
    }
}
