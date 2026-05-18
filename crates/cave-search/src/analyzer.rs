// SPDX-License-Identifier: AGPL-3.0-or-later
//! Text analyzer: tokenization + stop-word filtering.
//! upstream: opensearch v3.0/server/src/main/java/org/opensearch/index/analysis/

use crate::tenant::TenantId;

pub fn tokenize(_text: &str, _tenant_id: &TenantId) -> Vec<String> {
    unimplemented!("cave-search::analyzer::tokenize")
}

pub fn filter_stop_words<'a>(_tokens: Vec<&'a str>, _tenant_id: &TenantId) -> Vec<&'a str> {
    unimplemented!("cave-search::analyzer::filter_stop_words")
}
