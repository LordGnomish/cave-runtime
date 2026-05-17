// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Scrape manager: scrapes Prometheus /metrics endpoints on configurable intervals.
//! Service discovery: static, file-based, and Kubernetes.

pub mod discovery;
pub mod manager;
pub mod target;

pub use manager::ScrapeManager;
pub use target::{ScrapeTarget, ScrapeConfig};
