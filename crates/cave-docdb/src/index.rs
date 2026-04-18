//! Index metadata management.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Index {
    pub name: String,
    pub keys: BTreeMap<String, i32>, // field -> 1 (asc) or -1 (desc)
    pub unique: bool,
}

impl Index {
    pub fn new(name: String, keys: BTreeMap<String, i32>, unique: bool) -> Self {
        Self { name, keys, unique }
    }
}
