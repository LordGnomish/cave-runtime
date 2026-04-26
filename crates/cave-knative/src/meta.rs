//! Shared metadata + spec primitives used across Knative resources.
//! upstream: knative/serving v1.18.x

use std::collections::HashMap;

#[derive(Default)]
pub struct ObjectMeta {
    pub annotations: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub name: String,
    pub namespace: String,
}

#[derive(Default)]
pub struct TrafficTarget {
    pub revision_name: Option<String>,
    pub configuration_name: Option<String>,
    pub latest_revision: Option<bool>,
    pub percent: Option<i32>,
    pub tag: Option<String>,
}

#[derive(Default)]
pub struct RevisionTemplateSpec {
    pub metadata: ObjectMeta,
    pub spec: PodSpec,
}

#[derive(Default)]
pub struct PodSpec {
    pub containers: Vec<Container>,
}

#[derive(Default)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub env: Vec<EnvVar>,
}

#[derive(Default)]
pub struct EnvVar {
    pub name: String,
    pub value: Option<String>,
}
