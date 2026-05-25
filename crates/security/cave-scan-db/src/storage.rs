// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: aquasecurity/trivy-db@2034dd8 pkg/db/db.go
//! Sled-backed persistent store.
//!
//! Bucket layout (mirrors trivy-db's bbolt buckets):
//!
//! | tree           | key                          | value           |
//! |----------------|------------------------------|-----------------|
//! | `vulns`        | `<CVE-ID>`                   | `Vulnerability` |
//! | `advisories`   | `<ecosystem>:<pkg>:<vuln-id>`| `Advisory`      |
//! | `pkg_index`    | `<ecosystem>:<pkg>`          | list of vuln-ids|
//! | `iac_rules`    | `<rule-id>`                  | `IacRule`       |
//! | `iac_provider` | `<provider>:<rule-id>`       | (presence)      |
//!
//! Each value is JSON-encoded for simplicity and forward-compat with trivy
//! exports. `pkg_index` is maintained on `put_advisory` for O(1) lookups.

use crate::{
    Advisory, DbError, IacRule, IacRuleDb, LangAdvisoryDb, OsAdvisoryDb, Result, VulnDb,
    Vulnerability,
};
use sled::Db;
use std::path::Path;

const TREE_VULNS: &str = "vulns";
const TREE_ADVISORIES: &str = "advisories";
const TREE_PKG_INDEX: &str = "pkg_index";
const TREE_IAC_RULES: &str = "iac_rules";
const TREE_IAC_PROVIDER: &str = "iac_provider";

/// Sled-backed implementation of the full VulnDb / IacRuleDb surface.
pub struct SledStore {
    db: Db,
}

impl SledStore {
    /// Open or create a store at `path`. Sled creates the dir if absent.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let db = sled::open(path.as_ref())?;
        Ok(Self { db })
    }

    /// In-memory temporary store — handy for tests.
    pub fn temporary() -> Result<Self> {
        let db = sled::Config::new().temporary(true).open()?;
        Ok(Self { db })
    }

    fn tree(&self, name: &str) -> Result<sled::Tree> {
        Ok(self.db.open_tree(name)?)
    }

    fn advisory_key(eco: &str, pkg: &str, vuln_id: &str) -> String {
        format!("{eco}:{pkg}:{vuln_id}")
    }

    fn pkg_index_key(eco: &str, pkg: &str) -> String {
        format!("{eco}:{pkg}")
    }

    fn iac_provider_key(provider: &str, id: &str) -> String {
        format!("{provider}:{id}")
    }

    fn append_pkg_index(&self, eco: &str, pkg: &str, vuln_id: &str) -> Result<()> {
        let t = self.tree(TREE_PKG_INDEX)?;
        let k = Self::pkg_index_key(eco, pkg);
        let mut list: Vec<String> = match t.get(&k)? {
            Some(b) => serde_json::from_slice(&b)?,
            None => Vec::new(),
        };
        if !list.iter().any(|x| x == vuln_id) {
            list.push(vuln_id.to_string());
            t.insert(k.as_bytes(), serde_json::to_vec(&list)?)?;
        }
        Ok(())
    }
}

impl VulnDb for SledStore {
    fn put_vuln(&self, v: &Vulnerability) -> Result<()> {
        let t = self.tree(TREE_VULNS)?;
        t.insert(v.id.as_bytes(), serde_json::to_vec(v)?)?;
        Ok(())
    }

    fn get_vuln(&self, id: &str) -> Result<Option<Vulnerability>> {
        let t = self.tree(TREE_VULNS)?;
        match t.get(id.as_bytes())? {
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
            None => Ok(None),
        }
    }

    fn put_advisory(&self, a: &Advisory) -> Result<()> {
        let t = self.tree(TREE_ADVISORIES)?;
        let k = Self::advisory_key(&a.ecosystem, &a.package_name, &a.vulnerability_id);
        t.insert(k.as_bytes(), serde_json::to_vec(a)?)?;
        self.append_pkg_index(&a.ecosystem, &a.package_name, &a.vulnerability_id)?;
        Ok(())
    }

    fn count_vulns(&self) -> Result<usize> {
        Ok(self.tree(TREE_VULNS)?.len())
    }

    fn count_advisories(&self) -> Result<usize> {
        Ok(self.tree(TREE_ADVISORIES)?.len())
    }
}

fn advisories_via_index(store: &SledStore, eco: &str, pkg: &str) -> Result<Vec<Advisory>> {
    let idx = store.tree(TREE_PKG_INDEX)?;
    let adv = store.tree(TREE_ADVISORIES)?;
    let k = SledStore::pkg_index_key(eco, pkg);
    let ids: Vec<String> = match idx.get(&k)? {
        Some(b) => serde_json::from_slice(&b)?,
        None => return Ok(Vec::new()),
    };
    let mut out = Vec::with_capacity(ids.len());
    for vid in ids {
        let ak = SledStore::advisory_key(eco, pkg, &vid);
        if let Some(b) = adv.get(ak.as_bytes())? {
            out.push(serde_json::from_slice(&b)?);
        }
    }
    Ok(out)
}

impl OsAdvisoryDb for SledStore {
    fn advisories_for_pkg(&self, ecosystem: &str, package: &str) -> Result<Vec<Advisory>> {
        advisories_via_index(self, ecosystem, package)
    }
}

impl LangAdvisoryDb for SledStore {
    fn advisories_for_lang_pkg(&self, ecosystem: &str, package: &str) -> Result<Vec<Advisory>> {
        advisories_via_index(self, ecosystem, package)
    }
}

impl IacRuleDb for SledStore {
    fn put_rule(&self, r: &IacRule) -> Result<()> {
        let rules = self.tree(TREE_IAC_RULES)?;
        rules.insert(r.id.as_bytes(), serde_json::to_vec(r)?)?;
        let providers = self.tree(TREE_IAC_PROVIDER)?;
        providers.insert(
            Self::iac_provider_key(&r.provider, &r.id).as_bytes(),
            &[1u8],
        )?;
        Ok(())
    }

    fn get_rule(&self, id: &str) -> Result<Option<IacRule>> {
        let rules = self.tree(TREE_IAC_RULES)?;
        match rules.get(id.as_bytes())? {
            Some(b) => Ok(Some(serde_json::from_slice(&b)?)),
            None => Ok(None),
        }
    }

    fn rules_for_provider(&self, provider: &str) -> Result<Vec<IacRule>> {
        let providers = self.tree(TREE_IAC_PROVIDER)?;
        let rules = self.tree(TREE_IAC_RULES)?;
        let prefix = format!("{provider}:");
        let mut out = Vec::new();
        for kv in providers.scan_prefix(prefix.as_bytes()) {
            let (k, _) = kv?;
            let k_str =
                std::str::from_utf8(&k).map_err(|_| DbError::InvalidFeed("non-utf8 key".into()))?;
            let id = k_str.trim_start_matches(&prefix);
            if let Some(b) = rules.get(id.as_bytes())? {
                out.push(serde_json::from_slice(&b)?);
            }
        }
        Ok(out)
    }
}
