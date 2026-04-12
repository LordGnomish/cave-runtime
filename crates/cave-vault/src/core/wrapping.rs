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
    pub fn wrap(&mut self, data: Value, ttl_secs: i64, creation_path: &str) -> VaultResult<WrapInfo> {
        let rng = SystemRandom::new();
        let mut token_bytes = vec![0u8; 16];
        rng.fill(&mut token_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
        let token = format!("hvs.{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&token_bytes));
        let mut acc_bytes = vec![0u8; 8];
        rng.fill(&mut acc_bytes).map_err(|_| VaultError::Crypto("rng failure".into()))?;
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

        self.tokens.insert(token.clone(), WrappedResponse {
            token,
            accessor,
            ttl: ttl_secs,
            creation_time,
            creation_path: creation_path.to_string(),
            data,
        });

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
