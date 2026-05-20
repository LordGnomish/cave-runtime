// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Token management and storage for the Vault runtime.
//!
//! This module provides structures and logic for creating, storing,
//! renewing, and revoking Vault tokens. It includes utilities for
//! parsing TTL durations and generating secure token identifiers.

use crate::error::{VaultError, VaultResult};
use crate::response::AuthInfo;
use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents the type of a Vault token.
///
/// Tokens can be of type Service, Batch, or Default.
/// The Default variant is displayed as "service" for compatibility.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TokenType {
    Service,
    Batch,
    Default,
}

/// Implements the Display trait for TokenType.
///
/// Converts the token type enum variant into its string representation.
impl std::fmt::Display for TokenType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TokenType::Service => write!(f, "service"),
            TokenType::Batch => write!(f, "batch"),
            TokenType::Default => write!(f, "service"),
        }
    }
}

/// Represents a Vault token with all its associated properties.
///
/// This struct holds the identifier, accessor, policies, metadata,
/// TTL information, and other attributes relevant to a specific token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultToken {
    pub id: String,
    pub accessor: String,
    pub policies: Vec<String>,
    pub metadata: HashMap<String, String>,
    pub ttl: i64,
    pub max_ttl: i64,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub renewable: bool,
    pub orphan: bool,
    pub display_name: String,
    pub num_uses: i64,
    pub uses_remaining: i64,
    pub token_type: TokenType,
    pub parent: Option<String>,
    pub entity_id: String,
    pub is_root: bool,
    pub period: Option<i64>,
    pub explicit_max_ttl: i64,
    pub bound_cidrs: Vec<String>,
    pub role_name: Option<String>,
}

/// Implementation of methods for VaultToken.
///
/// Provides utility functions to check expiration, calculate remaining
/// time-to-live, and convert the token into an AuthInfo response.
impl VaultToken {
    /// Checks if the token has expired based on its expires_at timestamp.
    ///
    /// Returns true if the current UTC time is past the expiration time.
    pub fn is_expired(&self) -> bool {
        match &self.expires_at {
            Some(exp) => Utc::now() > *exp,
            None => false,
        }
    }

    /// Calculates the remaining time-to-live in seconds.
    ///
    /// Returns the number of seconds until expiration, or 0 if expired
    /// or if there is no expiration time set.
    pub fn remaining_ttl(&self) -> i64 {
        match &self.expires_at {
            Some(exp) => (*exp - Utc::now()).num_seconds().max(0),
            None => 0,
        }
    }

    /// Converts the token into an AuthInfo structure.
    ///
    /// Maps token fields to the corresponding fields in AuthInfo,
    /// including policies, metadata, and TTL information.
    pub fn to_auth_info(&self) -> AuthInfo {
        AuthInfo {
            client_token: self.id.clone(),
            accessor: self.accessor.clone(),
            policies: self.policies.clone(),
            token_policies: self.policies.clone(),
            metadata: self.metadata.clone(),
            lease_duration: self.remaining_ttl(),
            renewable: self.renewable,
            entity_id: self.entity_id.clone(),
            token_type: self.token_type.to_string(),
            orphan: self.orphan,
        }
    }
}

/// Parameters for creating a new Vault token.
///
/// Contains optional fields that customize the properties of the
/// newly created token, such as policies, TTL, and metadata.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CreateTokenParams {
    pub id: Option<String>,
    pub policies: Option<Vec<String>>,
    pub metadata: Option<HashMap<String, String>>,
    pub no_parent: Option<bool>,
    pub no_default_policy: Option<bool>,
    pub renewable: Option<bool>,
    pub ttl: Option<String>,
    pub explicit_max_ttl: Option<String>,
    pub display_name: Option<String>,
    pub num_uses: Option<i64>,
    pub period: Option<String>,
    pub role_name: Option<String>,
    pub token_type: Option<String>,
    pub entity_alias: Option<String>,
    pub bound_cidrs: Option<Vec<String>>,
}

/// Parses a duration string into seconds.
///
/// Supports numeric values (interpreted as seconds) and suffixed values
/// like 's', 'm', 'h', 'd' for seconds, minutes, hours, and days.
pub fn parse_duration(s: &str) -> i64 {
    if s.is_empty() {
        return 0;
    }
    let s = s.trim();
    if let Ok(n) = s.parse::<i64>() {
        return n;
    }
    let idx = s.find(|c: char| c.is_alphabetic());
    let (num_str, unit) = match idx {
        Some(u) => (&s[..u], &s[u..]),
        None => return 0,
    };
    let num: i64 = num_str.parse().unwrap_or(0);
    match unit {
        "s" => num,
        "m" => num * 60,
        "h" => num * 3600,
        "d" => num * 86400,
        _ => 0,
    }
}

/// Stores and manages Vault tokens.
///
/// This struct maintains the mapping between token IDs, accessors,
/// and token objects, as well as parent-child relationships.
pub struct TokenStore {
    tokens: HashMap<String, VaultToken>,
    accessors: HashMap<String, String>,
    children: HashMap<String, Vec<String>>,
}

/// Default implementation for TokenStore.
///
/// Initializes an empty store with empty HashMaps for tokens,
/// accessors, and children.
impl Default for TokenStore {
    fn default() -> Self {
        Self {
            tokens: HashMap::new(),
            accessors: HashMap::new(),
            children: HashMap::new(),
        }
    }
}

/// Implementation of methods for TokenStore.
///
/// Provides functionality to create, lookup, renew, and revoke tokens,
/// as well as generate secure token IDs and accessors.
impl TokenStore {
    /// Generates a secure random token ID.
    ///
    /// Creates a 16-byte random sequence and encodes it using
    /// URL-safe Base64 without padding, prefixed with "hvs.".
    fn gen_token_id() -> VaultResult<String> {
        let rng = SystemRandom::new();
        let mut bytes = vec![0u8; 16];
        rng.fill(&mut bytes)
            .map_err(|_| VaultError::Crypto("rng failure".into()))?;
        Ok(format!(
            "hvs.{}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&bytes)
        ))
    }

    /// Generates a secure random accessor string.
    ///
    /// Creates a 12-byte random sequence and encodes it as a hex string.
    fn gen_accessor() -> VaultResult<String> {
        let rng = SystemRandom::new();
        let mut bytes = vec![0u8; 12];
        rng.fill(&mut bytes)
            .map_err(|_| VaultError::Crypto("rng failure".into()))?;
        Ok(hex::encode(&bytes))
    }

    /// Creates a new token based on the provided parameters.
    ///
    /// Handles policy inheritance, TTL calculation, and storage of the
    /// new token in the store. Returns the created VaultToken.
    pub fn create(
        &mut self,
        params: &CreateTokenParams,
        parent: Option<&VaultToken>,
    ) -> VaultResult<VaultToken> {
        let id = if let Some(ref custom_id) = params.id {
            custom_id.clone()
        } else {
            Self::gen_token_id()?
        };
        let accessor = Self::gen_accessor()?;

        let mut policies = params.policies.clone().unwrap_or_default();
        if !params.no_default_policy.unwrap_or(false) && !policies.contains(&"default".to_string())
        {
            policies.push("default".to_string());
        }
        if policies.is_empty() {
            if let Some(p) = parent {
                policies = p.policies.clone();
            }
        }

        let ttl_secs = params
            .ttl
            .as_deref()
            .map(parse_duration)
            .filter(|&t| t > 0)
            .unwrap_or(3600);

        let max_ttl_secs = params
            .explicit_max_ttl
            .as_deref()
            .map(parse_duration)
            .filter(|&t| t > 0)
            .unwrap_or_else(|| ttl_secs * 8);

        let now = Utc::now();
        let expires_at = if ttl_secs > 0 {
            Some(now + Duration::seconds(ttl_secs))
        } else {
            None
        };

        let orphan = params.no_parent.unwrap_or(false) || parent.is_none();
        let parent_id = if !orphan {
            parent.map(|p| p.id.clone())
        } else {
            None
        };

        let token = VaultToken {
            id: id.clone(),
            accessor: accessor.clone(),
            policies,
            metadata: params.metadata.clone().unwrap_or_default(),
            ttl: ttl_secs,
            max_ttl: max_ttl_secs,
            created_at: now,
            expires_at,
            renewable: params.renewable.unwrap_or(true),
            orphan,
            display_name: params
                .display_name
                .clone()
                .unwrap_or_else(|| "token".to_string()),
            num_uses: params.num_uses.unwrap_or(0),
            uses_remaining: params.num_uses.unwrap_or(0),
            token_type: match params.token_type.as_deref() {
                Some("batch") => TokenType::Batch,
                _ => TokenType::Service,
            },
            parent: parent_id.clone(),
            entity_id: String::new(),
            is_root: false,
            period: params
                .period
                .as_deref()
                .map(parse_duration)
                .filter(|&p| p > 0),
            explicit_max_ttl: max_ttl_secs,
            bound_cidrs: params.bound_cidrs.clone().unwrap_or_default(),
            role_name: params.role_name.clone(),
        };

        if let Some(pid) = &parent_id {
            self.children
                .entry(pid.clone())
                .or_default()
                .push(id.clone());
        }
        self.accessors.insert(accessor, id.clone());
        self.tokens.insert(id, token.clone());
        Ok(token)
    }

    /// Creates a root token.
    ///
    /// Initializes a token with root privileges, no expiration, and
    /// specific policies. Inserts it into the store.
    pub fn create_root(&mut self, root_token_id: &str) -> VaultToken {
        let token = VaultToken {
            id: root_token_id.to_string(),
            accessor: "root-accessor".to_string(),
            policies: vec!["root".to_string()],
            metadata: HashMap::new(),
            ttl: 0,
            max_ttl: 0,
            created_at: Utc::now(),
            expires_at: None,
            renewable: false,
            orphan: true,
            display_name: "root".to_string(),
            num_uses: 0,
            uses_remaining: 0,
            token_type: TokenType::Service,
            parent: None,
            entity_id: String::new(),
            is_root: true,
            period: None,
            explicit_max_ttl: 0,
            bound_cidrs: Vec::new(),
            role_name: None,
        };
        self.accessors
            .insert("root-accessor".to_string(), root_token_id.to_string());
        self.tokens.insert(root_token_id.to_string(), token.clone());
        token
    }

    /// Looks up a token by its ID.
    ///
    /// Returns the token if found and not expired, otherwise None.
    pub fn lookup(&self, id: &str) -> Option<&VaultToken> {
        self.tokens.get(id).filter(|t| !t.is_expired())
    }

    /// Looks up a token by its accessor.
    ///
    /// Resolves the accessor to a token ID and returns the token
    /// if found and not expired, otherwise None.
    pub fn lookup_by_accessor(&self, accessor: &str) -> Option<&VaultToken> {
        self.accessors
            .get(accessor)
            .and_then(|id| self.tokens.get(id))
            .filter(|t| !t.is_expired())
    }

    /// Renews a token by extending its expiration time.
    ///
    /// Adds the specified increment to the current expiration time,
    /// respecting the max TTL limit. Returns the renewed token.
    pub fn renew(&mut self, id: &str, increment_secs: i64) -> VaultResult<&VaultToken> {
        let token = self.tokens.get_mut(id).ok_or(VaultError::TokenNotFound)?;
        if !token.renewable {
            return Err(VaultError::InvalidRequest("token is not renewable".into()));
        }
        let increment = if token.max_ttl > 0 {
            increment_secs.min(token.max_ttl)
        } else {
            increment_secs
        };
        token.expires_at = Some(Utc::now() + Duration::seconds(increment));
        token.ttl = increment;
        Ok(self.tokens.get(id).unwrap())
    }

    /// Revokes a token by ID.
    ///
    /// Removes the token and its accessor from the store.
    /// Returns true if the token was found and removed.
    pub fn revoke(&mut self, id: &str) -> bool {
        if let Some(token) = self.tokens.remove(id) {
            self.accessors.remove(&token.accessor);
            true
        } else {
            false
        }
    }

    /// Revokes a token and all its descendants.
    ///
    /// Recursively revokes child tokens before revoking the specified token.
    pub fn revoke_tree(&mut self, id: &str) {
        let children: Vec<String> = self.children.remove(id).unwrap_or_default();
        for child in children {
            self.revoke_tree(&child.clone());
        }
        self.revoke(id);
    }

    /// Lists all accessor strings in the store.
    ///
    /// Returns a vector of accessor strings currently stored.
    pub fn list_accessors(&self) -> Vec<String> {
        self.accessors.keys().cloned().collect()
    }

    /// Inserts a token directly into the store.
    ///
    /// Bypasses creation logic and directly adds the token and its
    /// accessor to the respective maps.
    pub fn insert_direct(&mut self, token: VaultToken) {
        self.accessors
            .insert(token.accessor.clone(), token.id.clone());
        self.tokens.insert(token.id.clone(), token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_lookup_token() {
        let mut store = TokenStore::default();
        let params = CreateTokenParams {
            policies: Some(vec!["default".to_string()]),
            ttl: Some("1h".to_string()),
            ..Default::default()
        };
        let token = store.create(&params, None).unwrap();
        assert!(token.id.starts_with("hvs."));
        let found = store.lookup(&token.id).unwrap();
        assert_eq!(found.id, token.id);
    }

    #[test]
    fn test_renew_token() {
        let mut store = TokenStore::default();
        let params = CreateTokenParams {
            ttl: Some("1h".to_string()),
            renewable: Some(true),
            ..Default::default()
        };
        let token = store.create(&params, None).unwrap();
        let renewed = store.renew(&token.id, 7200).unwrap();
        assert!(renewed.remaining_ttl() > 3000);
    }

    #[test]
    fn test_revoke_token() {
        let mut store = TokenStore::default();
        let params = CreateTokenParams {
            ttl: Some("1h".to_string()),
            ..Default::default()
        };
        let token = store.create(&params, None).unwrap();
        let id = token.id.clone();
        assert!(store.revoke(&id));
        assert!(store.lookup(&id).is_none());
    }

    #[test]
    fn test_root_token() {
        let mut store = TokenStore::default();
        let root = store.create_root("hvs.root-test-token");
        assert!(root.is_root);
        assert_eq!(root.policies, vec!["root"]);
        assert!(root.expires_at.is_none());
    }

    #[test]
    fn test_parse_duration() {
        assert_eq!(parse_duration("1h"), 3600);
        assert_eq!(parse_duration("30m"), 1800);
        assert_eq!(parse_duration("7d"), 604800);
        assert_eq!(parse_duration("3600"), 3600);
    }
}
