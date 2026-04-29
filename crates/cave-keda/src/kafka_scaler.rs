//! Kafka scaler — scales on Kafka consumer-group lag.
//! upstream: kedacore/keda v2.x — pkg/scalers/kafka_scaler.go

#[derive(Default)]
pub struct KafkaScaler {
    pub tenant_id: String,
    pub bootstrap_servers: Vec<String>,
    pub consumer_group: String,
    pub topic: String,
    pub lag_threshold: Option<i64>,
}

impl KafkaScaler {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::kafka_scaler::KafkaScaler::new")
    }
}
