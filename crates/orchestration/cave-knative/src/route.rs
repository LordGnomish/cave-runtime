// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Knative Route — traffic routing to Revisions.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/route_types.go

use crate::meta::{ObjectMeta, TrafficTarget, validate_traffic};

#[derive(Default, Debug, Clone)]
pub struct Route {
    pub metadata: ObjectMeta,
    pub spec: RouteSpec,
    pub status: RouteStatus,
}

#[derive(Default, Debug, Clone)]
pub struct RouteSpec {
    pub traffic: Vec<TrafficTarget>,
}

#[derive(Default, Debug, Clone)]
pub struct RouteStatus {
    pub traffic: Vec<TrafficTarget>,
    pub url: Option<String>,
    pub observed_generation: i64,
}

impl Route {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            metadata: ObjectMeta::with_creator(tenant_id),
            spec: RouteSpec::default(),
            status: RouteStatus::default(),
        }
    }

    /// Zero out traffic — every target gets percent=0.
    pub fn scale_to_zero(&mut self) {
        for t in &mut self.status.traffic {
            t.percent = Some(0);
        }
    }

    /// Validate the route's traffic split (must sum to 100, must reference targets).
    pub fn validate(&self) -> Result<(), String> {
        if self.spec.traffic.is_empty() {
            return Ok(()); // empty = will get a default route via Configuration
        }
        validate_traffic(&self.spec.traffic)
    }

    /// Resolve which revision a percent should route to (deterministic by-percent ordering).
    /// Returns the revision name responsible for the percentile, or None if no traffic configured.
    pub fn resolve_revision(&self, percentile: i32) -> Option<&str> {
        if self.status.traffic.is_empty() {
            return None;
        }
        let mut cursor = 0i32;
        let p = percentile.clamp(0, 99);
        for t in &self.status.traffic {
            cursor += t.percent.unwrap_or(0);
            if p < cursor {
                return t.revision_name.as_deref();
            }
        }
        self.status
            .traffic
            .last()
            .and_then(|t| t.revision_name.as_deref())
    }

    /// Promote a revision to 100% traffic.
    pub fn promote(&mut self, revision_name: &str) {
        self.spec.traffic = vec![TrafficTarget {
            revision_name: Some(revision_name.to_string()),
            percent: Some(100),
            ..Default::default()
        }];
    }

    /// Tag a revision (subroute access via traffic[].tag).
    pub fn tag(&mut self, revision_name: &str, tag: &str) {
        self.spec.traffic.push(TrafficTarget {
            revision_name: Some(revision_name.to_string()),
            tag: Some(tag.to_string()),
            percent: Some(0),
            ..Default::default()
        });
    }
}
