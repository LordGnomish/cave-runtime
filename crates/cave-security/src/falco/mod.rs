//! Falco runtime security subsystem.
//!
//! Full parity with Falco 0.38:
//!   - YAML rule/macro/list loading
//!   - Complete condition language (all operators + boolean + exceptions)
//!   - Syscall, k8s-audit, and cloud-trail event sources
//!   - Output formatting (text, JSON, gRPC envelope, HTTP webhook)

pub mod condition;
pub mod engine;
pub mod fields;
pub mod output;
pub mod rule;

pub use engine::{Alert, RuleEngine, RuleStore};
pub use fields::EventContext;
pub use rule::{FalcoList, FalcoMacro, FalcoRule, Priority, RuleSet};
