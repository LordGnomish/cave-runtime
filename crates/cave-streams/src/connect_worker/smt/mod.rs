// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//   connect/transforms/src/main/java/org/apache/kafka/connect/transforms/Transformation.java
//
//! Single Message Transforms (SMTs) — per-record mutation chain.
//!
//! Mirrors upstream `connect/transforms/` `Transformation<R>` and the
//! six built-ins covered here. A SMT receives a record, returns a
//! transformed record (or `None` to drop it). Chains compose
//! left-to-right; the `transforms` connector config lists them by
//! alias and the alias-prefixed config block (`transforms.<alias>.*`)
//! configures each.
//!
//! ## Honest scope
//!
//! cave-streams ships six SMTs in this module:
//!
//! * [`cast::Cast`] — Cast field or value to a primitive type.
//! * [`extract_field::ExtractField`] — Pull a single named field up to
//!   the top-level value.
//! * [`filter::Filter`] — Drop or keep records by predicate.
//! * [`header_from::HeaderFrom`] — Copy/move record fields into the
//!   record's Kafka headers.
//! * [`insert_field::InsertField`] — Insert a literal or topic/offset
//!   metadata into a value.
//! * [`mask_field::MaskField`] — Replace a named field with a fixed
//!   placeholder.
//!
//! Records are represented as `RecordEnvelope` — a header map plus an
//! optional JSON-shaped value. Schemas (Avro/Protobuf/JSON-Schema) are
//! tracked, not in this module; SMTs operate on the value-side `Value`
//! ladder which is enough for upstream functional parity.

pub mod cast;
pub mod extract_field;
pub mod filter;
pub mod flatten;
pub mod header_from;
pub mod insert_field;
pub mod mask_field;
pub mod regex_router;
pub mod replace_field;
pub mod timestamp_router;

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, RwLock};

use crate::error::{StreamsError, StreamsResult};

/// SMT-level value ladder. Avoids leaking serde_json across the
/// trait surface — the SMT chain stays std-only and the SMT
/// instances are object-safe.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Array(Vec<Value>),
    Object(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_object(&self) -> Option<&BTreeMap<String, Value>> {
        if let Value::Object(m) = self {
            Some(m)
        } else {
            None
        }
    }
    pub fn as_object_mut(&mut self) -> Option<&mut BTreeMap<String, Value>> {
        if let Value::Object(m) = self {
            Some(m)
        } else {
            None
        }
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Object(_))
    }
}

/// A record headed into the SMT chain. Mirrors upstream's
/// `ConnectRecord<R>` projection at SMT time: topic + partition +
/// offset for metadata-using SMTs, kafka headers + value for the
/// mutation surface.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordEnvelope {
    pub topic: String,
    pub partition: Option<i32>,
    pub kafka_offset: Option<u64>,
    pub timestamp_ms: Option<i64>,
    pub headers: BTreeMap<String, Vec<u8>>,
    pub key: Value,
    pub value: Value,
}

impl RecordEnvelope {
    pub fn new(topic: impl Into<String>, value: Value) -> Self {
        Self {
            topic: topic.into(),
            partition: None,
            kafka_offset: None,
            timestamp_ms: None,
            headers: BTreeMap::new(),
            key: Value::Null,
            value,
        }
    }
}

/// One SMT — the Connect "Transformation" interface. `apply`
/// receives the record and returns the new record, or `Ok(None)` to
/// drop it ("filter"). Implementors are `Send + Sync + 'static` so
/// chains can be installed by the runtime.
pub trait Smt: Send + Sync + std::fmt::Debug + 'static {
    /// Stable name (matches `transforms.<alias>.type` value in
    /// upstream Connect configs — e.g. `"org.apache.kafka.connect.
    /// transforms.MaskField$Value"`).
    fn name(&self) -> &'static str;

    /// One transform step. `Ok(None)` drops the record; `Ok(Some(r))`
    /// passes `r` to the next SMT; `Err(_)` fails the task.
    fn apply(&self, record: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>>;
}

/// A chain of SMTs applied left-to-right.
#[derive(Debug, Clone, Default)]
pub struct SmtChain {
    transforms: Vec<Arc<dyn Smt>>,
}

impl SmtChain {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_transforms(transforms: Vec<Arc<dyn Smt>>) -> Self {
        Self { transforms }
    }

    pub fn push(&mut self, t: Arc<dyn Smt>) -> &mut Self {
        self.transforms.push(t);
        self
    }

    pub fn len(&self) -> usize {
        self.transforms.len()
    }

    pub fn is_empty(&self) -> bool {
        self.transforms.is_empty()
    }

    /// Names of each SMT in order — used by the admin UI to show
    /// the configured chain.
    pub fn names(&self) -> Vec<&'static str> {
        self.transforms.iter().map(|t| t.name()).collect()
    }

    /// Apply the chain. Returns the final record or `Ok(None)` if any
    /// SMT in the chain dropped it. The first `Err` short-circuits.
    pub fn apply(&self, record: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
        let mut cur = record;
        for t in &self.transforms {
            match t.apply(cur)? {
                Some(next) => cur = next,
                None => return Ok(None),
            }
        }
        Ok(Some(cur))
    }
}

/// Builder closure registered for a SMT alias. Receives the
/// `transforms.<alias>.*` (already stripped) config map.
pub type SmtBuilder = fn(&BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>>;

/// Static registry — maps SMT type strings to builders. Each SMT
/// module registers exactly one builder under its upstream type
/// name (`"org.apache.kafka.connect.transforms.Cast$Value"`, etc.)
/// plus a short alias (`"Cast$Value"`).
///
/// This is the Rust analogue of upstream's classpath-discovered SMT
/// catalogue. We use an explicit `register_defaults()` rather than
/// linkme/inventory to avoid the workspace dep weight; in practice
/// the only call sites that need built-ins are
/// [`Self::with_defaults`] and the integration tests.
pub struct SmtRegistry {
    builders: RwLock<BTreeMap<String, SmtBuilder>>,
}

impl Default for SmtRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl SmtRegistry {
    pub fn new() -> Self {
        Self {
            builders: RwLock::new(BTreeMap::new()),
        }
    }

    /// Register `builder` under `name`. Replaces any previous entry
    /// (call once at boot; the load-test confirms idempotence).
    pub fn register(&self, name: impl Into<String>, builder: SmtBuilder) {
        let mut g = self.builders.write().expect("poisoned");
        g.insert(name.into(), builder);
    }

    /// Look up a builder by SMT type string. Returns `None` if the
    /// SMT is unknown — the Connect REST layer must surface a
    /// validation error rather than failing at task start.
    pub fn builder(&self, name: &str) -> Option<SmtBuilder> {
        self.builders.read().expect("poisoned").get(name).copied()
    }

    pub fn names(&self) -> Vec<String> {
        let g = self.builders.read().expect("poisoned");
        g.keys().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.builders.read().expect("poisoned").len()
    }

    /// Construct a SmtChain from a Connect-style config map. Reads
    /// `transforms` (comma-separated aliases), then for each alias
    /// looks up `transforms.<alias>.type` in the registry.
    pub fn build_chain(
        &self,
        config: &BTreeMap<String, String>,
    ) -> StreamsResult<SmtChain> {
        let aliases = match config.get("transforms") {
            None => return Ok(SmtChain::new()),
            Some(s) if s.is_empty() => return Ok(SmtChain::new()),
            Some(s) => s
                .split(',')
                .map(|x| x.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>(),
        };
        let mut chain = SmtChain::new();
        for alias in aliases {
            let type_key = format!("transforms.{alias}.type");
            let type_value = config.get(&type_key).ok_or_else(|| {
                StreamsError::Internal(format!("missing {type_key}"))
            })?;
            let builder = self.builder(type_value).ok_or_else(|| {
                StreamsError::Internal(format!("unknown SMT type: {type_value}"))
            })?;
            // Slice out transforms.<alias>.* into a fresh map with
            // the prefix removed.
            let prefix = format!("transforms.{alias}.");
            let scoped: BTreeMap<String, String> = config
                .iter()
                .filter_map(|(k, v)| {
                    k.strip_prefix(&prefix)
                        .filter(|k2| *k2 != "type")
                        .map(|k2| (k2.to_string(), v.clone()))
                })
                .collect();
            chain.push(builder(&scoped)?);
        }
        Ok(chain)
    }

    /// Pre-loaded registry with the ten built-in SMTs.
    pub fn with_defaults() -> Self {
        let me = Self::new();
        cast::Cast::register(&me);
        extract_field::ExtractField::register(&me);
        filter::Filter::register(&me);
        flatten::Flatten::register(&me);
        header_from::HeaderFrom::register(&me);
        insert_field::InsertField::register(&me);
        mask_field::MaskField::register(&me);
        regex_router::RegexRouter::register(&me);
        replace_field::ReplaceField::register(&me);
        timestamp_router::TimestampRouter::register(&me);
        me
    }
}

/// Process-wide default registry — cave-streams' `route::create_router`
/// uses this so the REST surface can resolve SMT aliases without each
/// handler threading a registry through.
pub fn global_smt_registry() -> &'static SmtRegistry {
    static G: OnceLock<SmtRegistry> = OnceLock::new();
    G.get_or_init(SmtRegistry::with_defaults)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct PassThru;
    impl Smt for PassThru {
        fn name(&self) -> &'static str {
            "test.PassThru"
        }
        fn apply(&self, r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
            Ok(Some(r))
        }
    }

    #[derive(Debug)]
    struct DropAll;
    impl Smt for DropAll {
        fn name(&self) -> &'static str {
            "test.DropAll"
        }
        fn apply(&self, _r: RecordEnvelope) -> StreamsResult<Option<RecordEnvelope>> {
            Ok(None)
        }
    }

    fn rec() -> RecordEnvelope {
        RecordEnvelope::new("orders", Value::Object(BTreeMap::new()))
    }

    #[test]
    fn empty_chain_returns_input_unchanged() {
        let chain = SmtChain::new();
        let r = chain.apply(rec()).unwrap().unwrap();
        assert_eq!(r.topic, "orders");
    }

    #[test]
    fn chain_runs_in_order() {
        let mut chain = SmtChain::new();
        chain.push(Arc::new(PassThru));
        chain.push(Arc::new(PassThru));
        assert_eq!(chain.len(), 2);
        let r = chain.apply(rec()).unwrap();
        assert!(r.is_some());
    }

    #[test]
    fn drop_short_circuits_remaining_smts() {
        let mut chain = SmtChain::new();
        chain.push(Arc::new(DropAll));
        chain.push(Arc::new(PassThru));
        let r = chain.apply(rec()).unwrap();
        assert!(r.is_none());
    }

    #[test]
    fn registry_with_defaults_has_six_builtins() {
        let reg = SmtRegistry::with_defaults();
        assert!(reg.len() >= 6, "expected ≥6 built-ins, got {}", reg.len());
    }

    #[test]
    fn registry_lookup_resolves_known() {
        let reg = SmtRegistry::with_defaults();
        assert!(reg.builder("org.apache.kafka.connect.transforms.Cast$Value").is_some());
        assert!(reg.builder("org.apache.kafka.connect.transforms.MaskField$Value").is_some());
    }

    #[test]
    fn registry_lookup_unknown_returns_none() {
        let reg = SmtRegistry::new();
        assert!(reg.builder("does-not-exist").is_none());
    }

    #[test]
    fn build_chain_empty_config_yields_empty_chain() {
        let reg = SmtRegistry::with_defaults();
        let chain = reg.build_chain(&BTreeMap::new()).unwrap();
        assert!(chain.is_empty());
    }

    #[test]
    fn build_chain_resolves_aliases_in_order() {
        let reg = SmtRegistry::with_defaults();
        let mut cfg = BTreeMap::new();
        cfg.insert("transforms".into(), "mask,extract".into());
        cfg.insert(
            "transforms.mask.type".into(),
            "org.apache.kafka.connect.transforms.MaskField$Value".into(),
        );
        cfg.insert("transforms.mask.fields".into(), "password".into());
        cfg.insert(
            "transforms.extract.type".into(),
            "org.apache.kafka.connect.transforms.ExtractField$Value".into(),
        );
        cfg.insert("transforms.extract.field".into(), "payload".into());
        let chain = reg.build_chain(&cfg).unwrap();
        assert_eq!(chain.len(), 2);
        assert_eq!(
            chain.names(),
            vec![
                "org.apache.kafka.connect.transforms.MaskField$Value",
                "org.apache.kafka.connect.transforms.ExtractField$Value",
            ]
        );
    }

    #[test]
    fn build_chain_unknown_type_errors() {
        let reg = SmtRegistry::with_defaults();
        let mut cfg = BTreeMap::new();
        cfg.insert("transforms".into(), "bogus".into());
        cfg.insert("transforms.bogus.type".into(), "not.a.real.smt".into());
        assert!(reg.build_chain(&cfg).is_err());
    }

    #[test]
    fn build_chain_missing_type_errors() {
        let reg = SmtRegistry::with_defaults();
        let mut cfg = BTreeMap::new();
        cfg.insert("transforms".into(), "x".into());
        // No transforms.x.type — should fail.
        assert!(reg.build_chain(&cfg).is_err());
    }

    #[test]
    fn registry_register_is_idempotent() {
        let reg = SmtRegistry::new();
        fn b(_: &BTreeMap<String, String>) -> StreamsResult<Arc<dyn Smt>> {
            Ok(Arc::new(PassThru))
        }
        reg.register("x", b);
        reg.register("x", b);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn global_registry_persists_across_calls() {
        let a = global_smt_registry();
        let b = global_smt_registry();
        assert!(std::ptr::eq(a, b));
    }
}
