//! cave-knative: Knative Serverless reimpl (scaffold — impl pending).
//!
//! upstream: knative/serving v1.18.x

#![allow(non_snake_case)]

pub mod meta;
pub mod ksvc;
pub mod revision;
pub mod configuration;
pub mod route;
pub mod eventing;

pub use ksvc::Ksvc;
pub use revision::Revision;
pub use configuration::Configuration;
pub use route::Route;
pub use eventing::{EventingSource, EventingSink};
pub use meta::{ObjectMeta, TrafficTarget, RevisionTemplateSpec, PodSpec, Container, EnvVar};
