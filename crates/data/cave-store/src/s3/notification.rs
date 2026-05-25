// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! S3 event notification dispatch.

use crate::s3::types::{NotificationConfiguration, S3Event};
use tokio::sync::broadcast;
use tracing::debug;

/// Dispatch an S3 event to all matching notification subscribers.
pub async fn dispatch(
    config: &NotificationConfiguration,
    event: &S3Event,
    tx: &broadcast::Sender<S3Event>,
) {
    // Queue configurations
    for qc in &config.queue_configurations {
        if qc
            .events
            .iter()
            .any(|e| event_matches(e, &event.event_name))
            && event.matches_filter(&qc.filter)
        {
            debug!(
                queue = %qc.queue_arn,
                event = %event.event_name,
                key = %event.key,
                "Dispatching event notification"
            );
            // In a real system, push to SQS/webhook; here we just broadcast.
            let _ = tx.send(event.clone());
        }
    }

    // Topic configurations
    for tc in &config.topic_configurations {
        if tc
            .events
            .iter()
            .any(|e| event_matches(e, &event.event_name))
            && event.matches_filter(&tc.filter)
        {
            debug!(
                topic = %tc.topic_arn,
                event = %event.event_name,
                "Dispatching topic notification"
            );
            let _ = tx.send(event.clone());
        }
    }

    // Lambda configurations
    for lc in &config.lambda_function_configurations {
        if lc
            .events
            .iter()
            .any(|e| event_matches(e, &event.event_name))
            && event.matches_filter(&lc.filter)
        {
            debug!(
                lambda = %lc.lambda_function_arn,
                event = %event.event_name,
                "Dispatching lambda notification"
            );
            let _ = tx.send(event.clone());
        }
    }
}

/// Match an event pattern against an event name.
/// Patterns use `*` as a wildcard, e.g. "s3:ObjectCreated:*".
fn event_matches(pattern: &str, name: &str) -> bool {
    if pattern == "*" || pattern == name {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    false
}
