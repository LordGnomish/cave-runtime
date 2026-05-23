// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Built-in detector library — line-by-line ports of the highest-signal
//! upstream `pkg/detectors/*` plug-ins (the 16 providers we ship in the
//! MVP rule pack). Custom YAML-defined detectors live in
//! `super::custom_detectors`.

pub mod anthropic;
pub mod aws;
pub mod azure;
pub mod gcp;
pub mod generic_api_key;
pub mod github;
pub mod gitlab;
pub mod jwt;
pub mod mailgun;
pub mod npm;
pub mod openai;
pub mod private_key;
pub mod pypi;
pub mod sendgrid;
pub mod slack;
pub mod square;
pub mod stripe;
pub mod twilio;
