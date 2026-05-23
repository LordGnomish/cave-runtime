// SPDX-License-Identifier: AGPL-3.0-or-later
//! Consumer-side helpers — lookup by username or custom_id, tag filters.

use crate::error::{AGwError, AGwResult};
use crate::models::Consumer;
use crate::store::GwStore;
use std::sync::Arc;

pub struct ConsumerLookup { pub store: Arc<GwStore> }

impl ConsumerLookup {
    pub fn new(store: Arc<GwStore>) -> Self { Self { store } }

    pub fn by_username(&self, name: &str) -> AGwResult<Consumer> {
        self.store.list_consumers().into_iter().find(|c| c.username == name)
            .ok_or_else(|| AGwError::ConsumerNotFound(name.into()))
    }
    pub fn by_custom_id(&self, custom_id: &str) -> AGwResult<Consumer> {
        self.store.list_consumers().into_iter().find(|c| c.custom_id.as_deref() == Some(custom_id))
            .ok_or_else(|| AGwError::ConsumerNotFound(custom_id.into()))
    }
    pub fn with_tag(&self, tag: &str) -> Vec<Consumer> {
        self.store.list_consumers().into_iter().filter(|c| c.tags.iter().any(|t| t == tag)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Arc<GwStore> {
        let s = Arc::new(GwStore::new());
        s.upsert_consumer(Consumer { tags: vec!["staff".into()], custom_id: Some("alice@corp".into()), ..Consumer::new("alice") }).unwrap();
        s.upsert_consumer(Consumer { tags: vec![], ..Consumer::new("bob") }).unwrap();
        s
    }
    #[test] fn by_username() {
        let l = ConsumerLookup::new(store());
        assert_eq!(l.by_username("alice").unwrap().username, "alice");
    }
    #[test] fn by_username_missing() {
        let l = ConsumerLookup::new(store());
        assert!(l.by_username("nope").is_err());
    }
    #[test] fn by_custom_id() {
        let l = ConsumerLookup::new(store());
        assert_eq!(l.by_custom_id("alice@corp").unwrap().username, "alice");
    }
    #[test] fn with_tag_match() {
        let l = ConsumerLookup::new(store());
        assert_eq!(l.with_tag("staff").len(), 1);
    }
    #[test] fn with_tag_none() {
        let l = ConsumerLookup::new(store());
        assert_eq!(l.with_tag("nope").len(), 0);
    }
}
