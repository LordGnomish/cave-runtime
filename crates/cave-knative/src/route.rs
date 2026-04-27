//! Knative Route — traffic routing to Revisions.
//! upstream: knative/serving v1.18.x — pkg/apis/serving/v1/route_types.go

use crate::meta::{ObjectMeta, TrafficTarget};

#[derive(Default)]
pub struct Route {
    pub metadata: ObjectMeta,
    pub spec: RouteSpec,
    pub status: RouteStatus,
}

#[derive(Default)]
pub struct RouteSpec {
    pub traffic: Vec<TrafficTarget>,
}

#[derive(Default)]
pub struct RouteStatus {
    pub traffic: Vec<TrafficTarget>,
    pub url: Option<String>,
}

impl Route {
    pub fn new(_tenant_id: &str) -> Self {
        unimplemented!("cave-knative::route::Route::new")
    }

    pub fn scale_to_zero(&mut self) {
        unimplemented!("cave-knative::route::Route::scale_to_zero")
    }
}
