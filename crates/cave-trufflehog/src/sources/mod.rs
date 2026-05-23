// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Source connectors — port of `pkg/sources/*`. Each connector emits
//! `Chunk`s into the engine pipeline.
//!
//! The connector trait is intentionally synchronous so it's easy to test;
//! the engine wraps each invocation in a `tokio::task::spawn_blocking`
//! when running under the live HTTP scheduler.

use crate::error::Result;
use crate::models::Chunk;

pub mod bitbucket;
pub mod confluence;
pub mod db;
pub mod docker;
pub mod filesystem;
pub mod gcs;
pub mod git;
pub mod github;
pub mod gitlab;
pub mod jira;
pub mod s3;
pub mod slack;
pub mod stdin;

pub trait Source: Send + Sync {
    fn name(&self) -> &str;
    fn chunks(&self) -> Result<Vec<Chunk>>;
}
