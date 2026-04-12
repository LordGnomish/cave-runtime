//! Log ingestion subsystem — multiple protocol receivers.

pub mod fluentd;
pub mod loki_push;
pub mod otlp;
pub mod syslog;
