//! TriggerAuthentication CRD — credentials for trigger sources.
//! upstream: kedacore/keda v2.x — apis/keda/v1alpha1/triggerauthentication_types.go

#[derive(Default)]
pub struct TriggerAuthentication {
    pub tenant_id: String,
    pub secret_target_ref: Vec<String>,
}

impl TriggerAuthentication {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-keda::trigger_authentication::TriggerAuthentication::new")
    }
}
