// SPDX-License-Identifier: AGPL-3.0-or-later
use ring::digest::{digest, SHA256};

fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

pub fn generate_version_id() -> String {
    use uuid::Uuid;
    Uuid::new_v4().to_string().replace('-', "").to_uppercase()
}

pub fn compute_etag(data: &[u8]) -> String {
    let hash = digest(&SHA256, data);
    to_hex(hash.as_ref())
}
