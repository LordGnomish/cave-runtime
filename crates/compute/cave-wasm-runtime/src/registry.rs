//! In-process content-addressed module registry.
//!
//! Stores wasm module blobs keyed by their `sha256:<hex>` digest and maps
//! human references (`name:tag`) onto digests — the local-store half of a
//! Spin/OCI-style module registry. The distributed (remote push/pull over a
//! transport) half is tracked honestly as out-of-scope in the parity manifest.

use crate::digest::sha256_hex;
use std::collections::HashMap;

/// A parsed `name:tag` module reference (tag defaults to `latest`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleRef {
    pub name: String,
    pub tag: String,
}

impl ModuleRef {
    pub fn parse(reference: &str) -> ModuleRef {
        match reference.rsplit_once(':') {
            Some((name, tag)) if !name.is_empty() && !tag.is_empty() => ModuleRef {
                name: name.to_string(),
                tag: tag.to_string(),
            },
            _ => ModuleRef {
                name: reference.to_string(),
                tag: "latest".to_string(),
            },
        }
    }

    /// Canonical `name:tag` string.
    pub fn canonical(&self) -> String {
        format!("{}:{}", self.name, self.tag)
    }
}

/// A registry catalogue entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryEntry {
    pub reference: String,
    pub digest: String,
    pub size: usize,
}

/// Content-addressed local module store.
#[derive(Debug, Default)]
pub struct ModuleRegistry {
    blobs: HashMap<String, Vec<u8>>,
    tags: HashMap<String, String>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        ModuleRegistry::default()
    }

    /// Store `bytes` and bind `reference` to its digest. Returns the
    /// `sha256:<hex>` digest. Content-addressed: identical bytes collapse to a
    /// single blob shared by every reference.
    pub fn push(&mut self, reference: &str, bytes: Vec<u8>) -> String {
        let digest = format!("sha256:{}", sha256_hex(&bytes));
        self.blobs.entry(digest.clone()).or_insert(bytes);
        self.tags
            .insert(ModuleRef::parse(reference).canonical(), digest.clone());
        digest
    }

    /// Fetch the bytes bound to a reference.
    pub fn pull(&self, reference: &str) -> Option<&[u8]> {
        let digest = self.tags.get(&ModuleRef::parse(reference).canonical())?;
        self.blobs.get(digest).map(|b| b.as_slice())
    }

    /// Fetch bytes by digest directly.
    pub fn get_by_digest(&self, digest: &str) -> Option<&[u8]> {
        self.blobs.get(digest).map(|b| b.as_slice())
    }

    /// Resolve a reference to its digest.
    pub fn resolve(&self, reference: &str) -> Option<String> {
        self.tags
            .get(&ModuleRef::parse(reference).canonical())
            .cloned()
    }

    /// All catalogue entries, sorted by reference.
    pub fn list(&self) -> Vec<RegistryEntry> {
        let mut out: Vec<RegistryEntry> = self
            .tags
            .iter()
            .map(|(reference, digest)| RegistryEntry {
                reference: reference.clone(),
                digest: digest.clone(),
                size: self.blobs.get(digest).map(|b| b.len()).unwrap_or(0),
            })
            .collect();
        out.sort_by(|a, b| a.reference.cmp(&b.reference));
        out
    }

    /// Remove a tag binding. Returns whether it existed. The underlying blob is
    /// retained (it may be shared by other references).
    pub fn remove(&mut self, reference: &str) -> bool {
        self.tags
            .remove(&ModuleRef::parse(reference).canonical())
            .is_some()
    }

    /// Number of distinct stored blobs.
    pub fn blob_count(&self) -> usize {
        self.blobs.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ref_parsing_defaults_tag() {
        assert_eq!(ModuleRef::parse("greet").canonical(), "greet:latest");
        assert_eq!(ModuleRef::parse("greet:v2").canonical(), "greet:v2");
    }

    #[test]
    fn push_then_pull_roundtrips() {
        let mut reg = ModuleRegistry::new();
        let digest = reg.push("greet:v1", b"\0asm\x01\0\0\0".to_vec());
        assert!(digest.starts_with("sha256:"));
        assert_eq!(reg.pull("greet:v1"), Some(&b"\0asm\x01\0\0\0"[..]));
    }

    #[test]
    fn identical_bytes_share_digest() {
        let mut reg = ModuleRegistry::new();
        let d1 = reg.push("a:1", b"same".to_vec());
        let d2 = reg.push("b:1", b"same".to_vec());
        assert_eq!(d1, d2);
        // content-addressed: one blob, two tags
        assert_eq!(reg.blob_count(), 1);
        assert_eq!(reg.get_by_digest(&d1), Some(&b"same"[..]));
    }

    #[test]
    fn resolve_and_list() {
        let mut reg = ModuleRegistry::new();
        let d = reg.push("mod:latest", b"xyz".to_vec());
        assert_eq!(reg.resolve("mod:latest"), Some(d.clone()));
        let list = reg.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].reference, "mod:latest");
        assert_eq!(list[0].digest, d);
        assert_eq!(list[0].size, 3);
    }

    #[test]
    fn remove_tag() {
        let mut reg = ModuleRegistry::new();
        reg.push("gone:1", b"data".to_vec());
        assert!(reg.remove("gone:1"));
        assert!(!reg.remove("gone:1"));
        assert_eq!(reg.pull("gone:1"), None);
    }
}
