// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Text analyzer: stub.

use crate::tenant::TenantId;

pub fn normalize_token(_token: &str) -> String { String::new() }
pub fn tokenize(_text: &str, _tenant_id: &TenantId) -> Vec<String> { Vec::new() }
pub fn filter_stop_words<'a>(tokens: Vec<&'a str>, _tenant_id: &TenantId) -> Vec<&'a str> { tokens }
pub fn stem(_word: &str) -> String { String::new() }
