// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::error::{VaultError, VaultResult};
use crate::response::WrapInfo;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde_json::Value;
use std::collections::HashMap;

pub struct WrappedResponse {
    pub token: String,
    pub accessor: String,
    pub ttl: i64,
    pub creation_time: DateTime<Utc>,
    pub creation_path: String,
    pub data: Value,
}

#[derive(Default)]
pub struct WrapStore {
    tokens: HashMap<String, WrappedResponse>,
}

impl WrapStore {
    pub fn wrap(
        &mut self,
        data: Value,
        ttl_secs: i64,
        creation_path: &str,
    ) -> VaultResult<WrapInfo> {
        let rng = SystemRandom::new();
        let mut token_bytes = vec![0u8; 16];
        rng.fill(&mut token_bytes)
            .map_err(|_| VaultError::Crypto("rng failure".into()))?;
        let token = format!(
            "hvs.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&token_bytes)
        );
        let mut acc_bytes = vec![0u8; 8];
        rng.fill(&mut acc_bytes)
            .map_err(|_| VaultError::Crypto("rng failure".into()))?;
        let accessor = hex::encode(&acc_bytes);
        let creation_time = Utc::now();

        let info = WrapInfo {
            token: token.clone(),
            accessor: accessor.clone(),
            ttl: ttl_secs,
            creation_time: creation_time.to_rfc3339(),
            creation_path: creation_path.to_string(),
            wrapped_accessor: accessor.clone(),
        };

        self.tokens.insert(
            token.clone(),
            WrappedResponse {
                token,
                accessor,
                ttl: ttl_secs,
                creation_time,
                creation_path: creation_path.to_string(),
                data,
            },
        );

        Ok(info)
    }

    pub fn unwrap(&mut self, token: &str) -> VaultResult<Value> {
        let entry = self.tokens.remove(token).ok_or(VaultError::WrapNotFound)?;
        let expires_at = entry.creation_time + Duration::seconds(entry.ttl);
        if Utc::now() > expires_at {
            return Err(VaultError::WrapNotFound);
        }
        Ok(entry.data)
    }

    pub fn lookup(&self, token: &str) -> VaultResult<WrapInfo> {
        let entry = self.tokens.get(token).ok_or(VaultError::WrapNotFound)?;
        let expires_at = entry.creation_time + Duration::seconds(entry.ttl);
        if Utc::now() > expires_at {
            return Err(VaultError::WrapNotFound);
        }
        Ok(WrapInfo {
            token: entry.token.clone(),
            accessor: entry.accessor.clone(),
            ttl: entry.ttl,
            creation_time: entry.creation_time.to_rfc3339(),
            creation_path: entry.creation_path.clone(),
            wrapped_accessor: entry.accessor.clone(),
        })
    }

    pub fn rewrap(&mut self, token: &str, new_ttl: i64) -> VaultResult<WrapInfo> {
        let entry = self.tokens.remove(token).ok_or(VaultError::WrapNotFound)?;
        let expires_at = entry.creation_time + Duration::seconds(entry.ttl);
        if Utc::now() > expires_at {
            return Err(VaultError::WrapNotFound);
        }
        self.wrap(entry.data, new_ttl, &entry.creation_path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_wrap_returns_token_with_hvs_prefix() {
        let mut s = WrapStore::default();
        let info = s.wrap(json!({"k": "v"}), 60, "/v1/secret/data/x").unwrap();
        assert!(info.token.starts_with("hvs."));
        assert_eq!(info.ttl, 60);
        assert_eq!(info.creation_path, "/v1/secret/data/x");
    }

    #[test]
    fn test_wrap_unwrap_roundtrip() {
        let mut s = WrapStore::default();
        let payload = json!({"username": "alice", "secret": "hunter2"});
        let info = s.wrap(payload.clone(), 60, "/p").unwrap();
        let unwrapped = s.unwrap(&info.token).unwrap();
        assert_eq!(unwrapped, payload);
    }

    #[test]
    fn test_unwrap_consumes_token_single_use() {
        let mut s = WrapStore::default();
        let info = s.wrap(json!(1), 60, "/p").unwrap();
        assert!(s.unwrap(&info.token).is_ok());
        // Second unwrap must fail — wrap tokens are single-use.
        assert!(s.unwrap(&info.token).is_err());
    }

    #[test]
    fn test_lookup_does_not_consume() {
        let mut s = WrapStore::default();
        let info = s.wrap(json!("data"), 60, "/p").unwrap();
        assert!(s.lookup(&info.token).is_ok());
        // Token still unwrappable after lookup.
        assert!(s.unwrap(&info.token).is_ok());
    }

    #[test]
    fn test_unwrap_unknown_token_fails() {
        let mut s = WrapStore::default();
        assert!(s.unwrap("hvs.bogus").is_err());
    }
}
