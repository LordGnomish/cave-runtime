// SPDX-License-Identifier: AGPL-3.0-or-later
use std::collections::HashMap;

/// Simple in-memory logical storage backend.
/// Paths are forward-slash delimited strings.
#[derive(Default)]
pub struct StorageBackend {
    data: HashMap<String, Vec<u8>>,
}

impl StorageBackend {
    pub fn get(&self, path: &str) -> Option<Vec<u8>> {
        self.data.get(path).cloned()
    }

    pub fn put(&mut self, path: &str, value: Vec<u8>) {
        self.data.insert(path.to_string(), value);
    }

    pub fn delete(&mut self, path: &str) {
        self.data.remove(path);
    }

    /// List all keys directly under prefix (one level deep, like a directory listing).
    pub fn list(&self, prefix: &str) -> Vec<String> {
        let prefix = if prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
        let mut keys = std::collections::BTreeSet::new();
        for key in self.data.keys() {
            if let Some(rest) = key.strip_prefix(&prefix) {
                let part = rest.split('/').next().unwrap_or(rest);
                if rest.contains('/') {
                    keys.insert(format!("{}/", part));
                } else {
                    keys.insert(part.to_string());
                }
            }
        }
        keys.into_iter().collect()
    }

    pub fn exists(&self, path: &str) -> bool {
        self.data.contains_key(path)
    }
}
