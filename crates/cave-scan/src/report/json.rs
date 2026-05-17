// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors

//! Plain JSON serializer — passes through the [`super::Report`] struct verbatim.

use super::Report;

pub fn to_string(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string(report)
}

pub fn to_string_pretty(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}
