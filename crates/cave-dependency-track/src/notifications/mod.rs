// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Notification rules + publishers.
//!
//! Mirrors `org.dependencytrack.notification.{publisher,vo}` +
//! `model/NotificationRule`.

pub mod publishers;
pub mod rules;

pub use publishers::{
    Publisher, render_email, render_jira_issue, render_mattermost, render_slack, render_teams,
    render_webhook,
};
pub use rules::{
    NotificationLevel, NotificationRule, NotificationRuleStore, NotificationScope, NotificationTrigger,
    PublisherKind,
};
