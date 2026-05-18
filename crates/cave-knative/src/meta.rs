// SPDX-License-Identifier: AGPL-3.0-or-later
//! Shared metadata + spec primitives used across Knative resources.
//! upstream: knative/serving v1.18.x

use std::collections::HashMap;

pub const ANNOTATION_CREATOR: &str = "knative.dev/creator";
pub const ANNOTATION_LAST_MODIFIER: &str = "knative.dev/lastModifier";
pub const ANNOTATION_AUTOSCALER_CLASS: &str = "autoscaling.knative.dev/class";
pub const ANNOTATION_MIN_SCALE: &str = "autoscaling.knative.dev/minScale";
pub const ANNOTATION_MAX_SCALE: &str = "autoscaling.knative.dev/maxScale";
pub const ANNOTATION_TARGET: &str = "autoscaling.knative.dev/target";
pub const ANNOTATION_METRIC: &str = "autoscaling.knative.dev/metric";

#[derive(Default, Debug, Clone)]
pub struct ObjectMeta {
    pub annotations: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub name: String,
    pub namespace: String,
    pub generation: i64,
}

impl ObjectMeta {
    pub fn with_creator(tenant_id: &str) -> Self {
        let mut m = ObjectMeta::default();
        m.annotations.insert(ANNOTATION_CREATOR.to_string(), tenant_id.to_string());
        m.annotations.insert(ANNOTATION_LAST_MODIFIER.to_string(), tenant_id.to_string());
        m
    }

    pub fn creator(&self) -> Option<&String> {
        self.annotations.get(ANNOTATION_CREATOR)
    }
}

#[derive(Default, Debug, Clone)]
pub struct TrafficTarget {
    pub revision_name: Option<String>,
    pub configuration_name: Option<String>,
    pub latest_revision: Option<bool>,
    pub percent: Option<i32>,
    pub tag: Option<String>,
}

#[derive(Default, Debug, Clone)]
pub struct RevisionTemplateSpec {
    pub metadata: ObjectMeta,
    pub spec: PodSpec,
}

#[derive(Default, Debug, Clone)]
pub struct PodSpec {
    pub containers: Vec<Container>,
}

#[derive(Default, Debug, Clone)]
pub struct Container {
    pub name: String,
    pub image: String,
    pub env: Vec<EnvVar>,
}

#[derive(Default, Debug, Clone)]
pub struct EnvVar {
    pub name: String,
    pub value: Option<String>,
}

/// Validate a traffic split: percentages must sum to 100.
pub fn validate_traffic(targets: &[TrafficTarget]) -> Result<(), String> {
    if targets.is_empty() {
        return Err("traffic split is empty".to_string());
    }
    let sum: i32 = targets.iter().filter_map(|t| t.percent).sum();
    if sum != 100 {
        return Err(format!("traffic percentages must sum to 100 (got {sum})"));
    }
    for t in targets {
        if t.revision_name.is_none() && t.configuration_name.is_none() && t.latest_revision != Some(true) {
            return Err("traffic target must reference a revision, configuration, or latestRevision=true".to_string());
        }
    }
    Ok(())
}

/// Validate a RevisionTemplateSpec — a revision template must have at least one container with an image.
pub fn validate_template(template: &RevisionTemplateSpec) -> Result<(), String> {
    if template.spec.containers.is_empty() {
        return Err("revision template must have at least one container".to_string());
    }
    for c in &template.spec.containers {
        if c.image.is_empty() {
            return Err("container image must not be empty".to_string());
        }
    }
    Ok(())
}
