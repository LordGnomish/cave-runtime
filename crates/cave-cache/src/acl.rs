// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! ACL (Access Control List) system — Redis 6+ compatible.

use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub struct AclUser {
    pub name: String,
    pub enabled: bool,
    pub passwords: Vec<String>, // SHA-256 hex hashes
    pub flags: UserFlags,
    pub allowed_commands: CommandPermissions,
    pub allowed_keys: KeyPermissions,
    pub allowed_channels: ChannelPermissions,
}

impl AclUser {
    pub fn default_user() -> Self {
        AclUser {
            name: "default".into(),
            enabled: true,
            passwords: vec![],
            flags: UserFlags { no_pass: true, no_touch_keys: false, reset_keys: false },
            allowed_commands: CommandPermissions::All,
            allowed_keys: KeyPermissions::All,
            allowed_channels: ChannelPermissions::All,
        }
    }

    pub fn can_execute(&self, cmd: &str) -> bool {
        if !self.enabled {
            return false;
        }
        self.allowed_commands.allows(cmd)
    }

    pub fn can_access_key(&self, key: &[u8]) -> bool {
        self.allowed_keys.allows(key)
    }

    pub fn authenticate(&self, password: &str) -> bool {
        if self.flags.no_pass {
            return true;
        }
        let hash = sha256_hex(password.as_bytes());
        self.passwords.iter().any(|p| p == &hash)
    }
}

#[derive(Debug, Clone)]
pub struct UserFlags {
    pub no_pass: bool,
    pub no_touch_keys: bool,
    pub reset_keys: bool,
}

#[derive(Debug, Clone)]
pub enum CommandPermissions {
    All,
    None,
    Specific {
        allowed: HashSet<String>,
        denied: HashSet<String>,
    },
}

impl CommandPermissions {
    pub fn allows(&self, cmd: &str) -> bool {
        let cmd = cmd.to_ascii_lowercase();
        match self {
            CommandPermissions::All => true,
            CommandPermissions::None => false,
            CommandPermissions::Specific { allowed, denied } => {
                if denied.contains(&cmd) {
                    return false;
                }
                allowed.contains(&cmd) || allowed.contains("all")
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum KeyPermissions {
    All,
    None,
    Patterns(Vec<Vec<u8>>),
}

impl KeyPermissions {
    pub fn allows(&self, key: &[u8]) -> bool {
        match self {
            KeyPermissions::All => true,
            KeyPermissions::None => false,
            KeyPermissions::Patterns(patterns) => {
                patterns.iter().any(|p| crate::db::glob_match(p, key))
            }
        }
    }
}

#[derive(Debug, Clone)]
pub enum ChannelPermissions {
    All,
    None,
    Patterns(Vec<Vec<u8>>),
}

impl ChannelPermissions {
    pub fn allows(&self, channel: &[u8]) -> bool {
        match self {
            ChannelPermissions::All => true,
            ChannelPermissions::None => false,
            ChannelPermissions::Patterns(patterns) => {
                patterns.iter().any(|p| crate::db::glob_match(p, channel))
            }
        }
    }
}

// ── ACL State ─────────────────────────────────────────────────────────────────

#[derive(Debug, Default)]
pub struct AclState {
    pub users: HashMap<String, AclUser>,
    /// ACL log: recent auth failures
    pub log: Vec<AclLogEntry>,
}

impl AclState {
    pub fn new() -> Self {
        let mut users = HashMap::new();
        users.insert("default".into(), AclUser::default_user());
        AclState { users, log: vec![] }
    }

    pub fn get_user(&self, name: &str) -> Option<&AclUser> {
        self.users.get(name)
    }

    pub fn authenticate(&self, username: &str, password: &str) -> bool {
        self.users.get(username).map(|u| u.authenticate(password)).unwrap_or(false)
    }

    pub fn list_users(&self) -> Vec<String> {
        self.users.keys().cloned().collect()
    }

    pub fn whoami(&self) -> &str {
        "default"
    }
}

#[derive(Debug, Clone)]
pub struct AclLogEntry {
    pub count: u64,
    pub reason: String,
    pub context: String,
    pub object: String,
    pub username: String,
    pub age_seconds: f64,
    pub client_info: String,
    pub entry_id: u64,
}

fn sha256_hex(data: &[u8]) -> String {
    use ring::digest;
    let d = digest::digest(&digest::SHA256, data);
    d.as_ref().iter().map(|b| format!("{:02x}", b)).collect()
}
