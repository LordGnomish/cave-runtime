//! HTTP scaler (KEDA HTTP add-on) — scales on inbound HTTP request rate.
//! upstream: kedacore/http-add-on v0.x

#[derive(Default)]
pub struct HttpScaler {
    pub tenant_id: String,
    pub host: String,
    pub target_pending_requests: Option<i32>,
}

impl HttpScaler {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::http_scaler::HttpScaler::new")
    }
}
