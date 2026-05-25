// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Built-in cave-flavored providers — minimal functional stubs that operate
//! in-process (no real cluster IO). Live providers route through cave-apiserver
//! in Phase 2.

pub mod helm;
pub mod kubernetes;
