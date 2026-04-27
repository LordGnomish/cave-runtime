//! Deep, sans-IO state machines for the streams crate. Mirrors the Kafka
//! KIPs and Pulsar PIPs that govern exactly-once semantics, idempotent
//! producer state, transaction coordinator state, and geo-replication.

pub mod eos;
pub mod geo_rep;
pub mod idempotent_producer;
pub mod tenant;
pub mod txn_coordinator;
