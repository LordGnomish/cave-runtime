// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Conversation memory — an append-only turn log with a windowed view, a
//! token-budget eviction policy that pins `System` turns, and keyword recall.
//!
//! OpenJarvis upstream: `jarvis/memory/conversation.py`. The on-device variant
//! keeps the whole transcript in process; embedding-backed semantic recall is
//! scope-cut to cave-search, so recall here is a deterministic substring scan.

use serde::{Deserialize, Serialize};

/// Who produced a turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// The fixed system / persona prompt. Pinned across eviction.
    System,
    /// An end-user message.
    User,
    /// A model response.
    Assistant,
    /// A tool-result message folded back into context.
    Tool,
}

/// A single conversation turn.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    /// Monotonic 0-based sequence number assigned at append time.
    pub seq: i64,
    /// Producer of the turn.
    pub role: Role,
    /// The textual content.
    pub content: String,
}

/// An ordered, append-only conversation transcript.
#[derive(Default)]
pub struct ConversationMemory {
    turns: Vec<Turn>,
    next_seq: i64,
}

impl ConversationMemory {
    /// An empty transcript.
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a turn, assigning it the next monotonic sequence number.
    pub fn append(&mut self, role: Role, content: impl Into<String>) {
        self.turns.push(Turn {
            seq: self.next_seq,
            role,
            content: content.into(),
        });
        self.next_seq += 1;
    }

    /// All turns in append order.
    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    /// Number of turns currently retained.
    pub fn len(&self) -> usize {
        self.turns.len()
    }

    /// Whether the transcript is empty.
    pub fn is_empty(&self) -> bool {
        self.turns.is_empty()
    }

    /// The last `n` turns, in order. Fewer if the history is shorter.
    pub fn window(&self, n: usize) -> Vec<&Turn> {
        let start = self.turns.len().saturating_sub(n);
        self.turns[start..].iter().collect()
    }

    /// A cheap token estimate: total content characters divided by four,
    /// the standard BPE rule-of-thumb.
    pub fn token_estimate(&self) -> usize {
        self.turns
            .iter()
            .map(|t| t.content.chars().count())
            .sum::<usize>()
            / 4
    }

    /// Evict the oldest non-`System` turns until the estimate fits `max_tokens`.
    /// `System` turns are never removed. Returns the number of turns evicted.
    pub fn evict_to_budget(&mut self, max_tokens: usize) -> usize {
        let mut evicted = 0;
        while self.token_estimate() > max_tokens {
            // Find the oldest non-system turn.
            let Some(idx) = self.turns.iter().position(|t| t.role != Role::System) else {
                break; // only system turns remain
            };
            self.turns.remove(idx);
            evicted += 1;
        }
        evicted
    }

    /// All turns whose content contains `keyword`, case-insensitively.
    pub fn recall(&self, keyword: &str) -> Vec<&Turn> {
        let needle = keyword.to_lowercase();
        self.turns
            .iter()
            .filter(|t| t.content.to_lowercase().contains(&needle))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_estimate_is_zero() {
        assert_eq!(ConversationMemory::new().token_estimate(), 0);
    }

    #[test]
    fn evict_on_already_small_is_noop() {
        let mut m = ConversationMemory::new();
        m.append(Role::User, "hi");
        assert_eq!(m.evict_to_budget(1000), 0);
        assert_eq!(m.len(), 1);
    }
}
