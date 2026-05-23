// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Slack source — channels + messages enumeration via the Slack REST API.

use crate::error::Result;
use crate::models::SourceKind;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SlackOptions {
    pub channels: Vec<String>,
    pub include_dms: bool,
    pub include_threads: bool,
    pub since_ts: Option<f64>,
}

pub struct SlackSource {
    pub options: SlackOptions,
    pub bot_token: Option<String>,
}

impl SlackSource {
    pub fn new(options: SlackOptions) -> Self {
        Self {
            options,
            bot_token: None,
        }
    }

    pub fn with_token(mut self, t: impl Into<String>) -> Self {
        self.bot_token = Some(t.into());
        self
    }

    pub fn name(&self) -> &str {
        "slack"
    }
    pub fn kind(&self) -> SourceKind {
        SourceKind::Slack
    }

    pub fn chunks(&self) -> Result<Vec<crate::models::Chunk>> {
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn store_token() {
        let s = SlackSource::new(SlackOptions::default()).with_token("xoxb-x");
        assert_eq!(s.bot_token.as_deref(), Some("xoxb-x"));
    }

    #[test]
    fn kind_is_slack() {
        let s = SlackSource::new(SlackOptions::default());
        assert_eq!(s.kind(), SourceKind::Slack);
    }

    #[test]
    fn options_round_trip() {
        let opts = SlackOptions {
            channels: vec!["general".into()],
            include_dms: true,
            ..Default::default()
        };
        let json = serde_json::to_string(&opts).unwrap();
        let back: SlackOptions = serde_json::from_str(&json).unwrap();
        assert_eq!(back.channels, vec!["general".to_string()]);
        assert!(back.include_dms);
    }
}
