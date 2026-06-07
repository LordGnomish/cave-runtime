// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! RabbitMQ scaler — scales on queue length or publish message-rate.
//! upstream: kedacore/keda v2.16.1 — pkg/scalers/rabbitmq_scaler.go

use crate::scaler::{ScalerTrait, replicas_from_metric};
use std::time::Duration;

// upstream rabbitmq_scaler.go const block (mode + operation names)
const RABBIT_MODE_QUEUE_LENGTH: &str = "QueueLength";
const RABBIT_MODE_MESSAGE_RATE: &str = "MessageRate";
const DEFAULT_RABBITMQ_QUEUE_LENGTH: f64 = 20.0;
const DEFAULT_RABBITMQ_PAGE_SIZE: i64 = 100;

/// `mode` trigger value — QueueLength counts messages, MessageRate watches the
/// broker publish rate. Unknown mirrors upstream's `default=Unknown`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RabbitMqMode {
    Unknown,
    QueueLength,
    MessageRate,
}

impl RabbitMqMode {
    fn as_str(self) -> &'static str {
        match self {
            RabbitMqMode::QueueLength => RABBIT_MODE_QUEUE_LENGTH,
            RabbitMqMode::MessageRate => RABBIT_MODE_MESSAGE_RATE,
            RabbitMqMode::Unknown => "Unknown",
        }
    }
}

/// `operation` — how regex-matched queues are folded into one metric.
/// Mirrors upstream sumOperation/avgOperation/maxOperation (default sum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RabbitMqOperation {
    Sum,
    Avg,
    Max,
}

/// Per-queue snapshot — mirrors upstream `queueInfo` (the fields the scaler
/// actually reads for a scaling decision).
#[derive(Debug, Clone, Copy, Default)]
pub struct QueueInfo {
    pub messages: i64,
    pub messages_ready: i64,
    pub messages_unacknowledged: i64,
    pub publish_rate: f64,
}

#[derive(Debug, Clone)]
pub struct RabbitMqScaler {
    pub tenant_id: String,
    pub queue_name: String,
    pub mode: RabbitMqMode,
    pub queue_length: f64,
    pub value: f64,
    pub activation_value: f64,
    pub exclude_unacknowledged: bool,
    pub use_regex: bool,
    pub operation: RabbitMqOperation,
    pub page_size: i64,
    pub protocol: String,
    pub vhost_name: String,
    queues: Vec<QueueInfo>,
}

impl RabbitMqScaler {
    pub fn new(tenant_id: &str) -> Self {
        Self {
            tenant_id: tenant_id.to_string(),
            queue_name: String::new(),
            mode: RabbitMqMode::Unknown,
            queue_length: DEFAULT_RABBITMQ_QUEUE_LENGTH,
            value: 0.0,
            activation_value: 0.0,
            exclude_unacknowledged: false,
            use_regex: false,
            operation: RabbitMqOperation::Sum,
            page_size: DEFAULT_RABBITMQ_PAGE_SIZE,
            protocol: "auto".to_string(),
            vhost_name: String::new(),
            queues: Vec::new(),
        }
    }

    /// Record a queue snapshot — for `use_regex` scalers, call once per
    /// matched queue; the composed view is folded with `operation`.
    pub fn record_queue(&mut self, info: QueueInfo) {
        self.queues.push(info);
    }

    /// Fold the recorded queues into a single composed `queueInfo`.
    /// Port of `getComposedQueue` + `getSum`/`getAverage`/`getMaximum`.
    pub fn composed(&self) -> QueueInfo {
        let q = &self.queues;
        if q.is_empty() {
            return QueueInfo::default();
        }
        match self.operation {
            RabbitMqOperation::Sum => {
                let mut sum = QueueInfo::default();
                for v in q {
                    sum.messages += v.messages;
                    sum.messages_ready += v.messages_ready;
                    sum.publish_rate += v.publish_rate;
                }
                sum
            }
            RabbitMqOperation::Avg => {
                let len = q.len() as i64;
                let mut sum = QueueInfo::default();
                for v in q {
                    sum.messages += v.messages;
                    sum.messages_ready += v.messages_ready;
                    sum.publish_rate += v.publish_rate;
                }
                QueueInfo {
                    messages: sum.messages / len,
                    messages_ready: sum.messages_ready / len,
                    messages_unacknowledged: 0,
                    publish_rate: sum.publish_rate / len as f64,
                }
            }
            RabbitMqOperation::Max => {
                let mut m = QueueInfo::default();
                for v in q {
                    if v.messages > m.messages {
                        m.messages = v.messages;
                    }
                    if v.messages_ready > m.messages_ready {
                        m.messages_ready = v.messages_ready;
                    }
                    if v.publish_rate > m.publish_rate {
                        m.publish_rate = v.publish_rate;
                    }
                }
                m
            }
        }
    }

    /// The message count the scaler acts on — `messages_ready` when
    /// `exclude_unacknowledged`, else total `messages`.
    /// Port of `getQueueInfoViaHTTP`'s return selection.
    pub fn message_count(&self) -> i64 {
        let info = self.composed();
        if self.exclude_unacknowledged {
            info.messages_ready
        } else {
            info.messages
        }
    }

    pub fn publish_rate(&self) -> f64 {
        self.composed().publish_rate
    }

    /// Replicas from the QueueLength target (ceil(messages / queueLength)).
    pub fn recommended_replicas(&self) -> i32 {
        replicas_from_metric(self.message_count() as f64, self.queue_length)
    }

    pub fn mode_str(&self) -> &'static str {
        self.mode.as_str()
    }
}

impl ScalerTrait for RabbitMqScaler {
    fn metric_value(&self) -> Option<f64> {
        match self.mode {
            RabbitMqMode::MessageRate => Some(self.publish_rate()),
            _ => Some(self.message_count() as f64),
        }
    }

    // Port of GetMetricsAndActivity: QueueLength compares messages, all other
    // modes activate on publishRate OR messages above activationValue.
    fn is_active(&self) -> bool {
        let messages = self.message_count() as f64;
        if self.mode == RabbitMqMode::QueueLength {
            messages > self.activation_value
        } else {
            self.publish_rate() > self.activation_value || messages > self.activation_value
        }
    }

    fn activation_threshold(&self) -> f64 {
        self.activation_value
    }

    fn polling_interval(&self) -> Duration {
        Duration::from_secs(30)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_upstream() {
        let s = RabbitMqScaler::new("t");
        assert_eq!(s.queue_length, 20.0);
        assert_eq!(s.page_size, 100);
        assert_eq!(s.protocol, "auto");
        assert_eq!(s.operation, RabbitMqOperation::Sum);
        assert_eq!(s.mode, RabbitMqMode::Unknown);
    }

    #[test]
    fn single_queue_message_count_uses_total_messages() {
        let mut s = RabbitMqScaler::new("t");
        s.record_queue(QueueInfo {
            messages: 42,
            messages_ready: 30,
            messages_unacknowledged: 12,
            publish_rate: 0.0,
        });
        assert_eq!(s.message_count(), 42);
    }

    #[test]
    fn exclude_unacknowledged_uses_ready_only() {
        let mut s = RabbitMqScaler::new("t");
        s.exclude_unacknowledged = true;
        s.record_queue(QueueInfo {
            messages: 42,
            messages_ready: 30,
            messages_unacknowledged: 12,
            publish_rate: 0.0,
        });
        assert_eq!(s.message_count(), 30);
    }

    #[test]
    fn queue_length_mode_active_above_activation_value() {
        let mut s = RabbitMqScaler::new("t");
        s.mode = RabbitMqMode::QueueLength;
        s.activation_value = 10.0;
        s.record_queue(QueueInfo { messages: 5, ..Default::default() });
        assert!(!s.is_active());
        s.record_queue(QueueInfo { messages: 50, ..Default::default() });
        assert!(s.is_active());
    }

    #[test]
    fn message_rate_mode_active_on_rate_or_messages() {
        let mut s = RabbitMqScaler::new("t");
        s.mode = RabbitMqMode::MessageRate;
        s.activation_value = 10.0;
        // rate above threshold, zero messages → active
        s.record_queue(QueueInfo { messages: 0, publish_rate: 25.0, ..Default::default() });
        assert!(s.is_active());
        assert_eq!(s.metric_value(), Some(25.0));
    }

    #[test]
    fn compose_sum_adds_messages_and_rate() {
        let mut s = RabbitMqScaler::new("t");
        s.use_regex = true;
        s.record_queue(QueueInfo { messages: 100, messages_ready: 80, publish_rate: 1.5, ..Default::default() });
        s.record_queue(QueueInfo { messages: 200, messages_ready: 150, publish_rate: 2.5, ..Default::default() });
        let c = s.composed();
        assert_eq!(c.messages, 300);
        assert_eq!(c.messages_ready, 230);
        assert!((c.publish_rate - 4.0).abs() < 1e-9);
    }

    #[test]
    fn compose_avg_integer_division_like_upstream() {
        let mut s = RabbitMqScaler::new("t");
        s.operation = RabbitMqOperation::Avg;
        s.record_queue(QueueInfo { messages: 10, ..Default::default() });
        s.record_queue(QueueInfo { messages: 21, ..Default::default() });
        // (10 + 21) / 2 = 15 with integer division (upstream getAverage)
        assert_eq!(s.composed().messages, 15);
    }

    #[test]
    fn compose_max_takes_largest_per_field() {
        let mut s = RabbitMqScaler::new("t");
        s.operation = RabbitMqOperation::Max;
        s.record_queue(QueueInfo { messages: 10, publish_rate: 9.0, ..Default::default() });
        s.record_queue(QueueInfo { messages: 7, publish_rate: 12.0, ..Default::default() });
        let c = s.composed();
        assert_eq!(c.messages, 10);
        assert!((c.publish_rate - 12.0).abs() < 1e-9);
    }

    #[test]
    fn recommended_replicas_ceils_against_queue_length() {
        let mut s = RabbitMqScaler::new("t");
        s.queue_length = 20.0;
        s.record_queue(QueueInfo { messages: 45, ..Default::default() });
        // ceil(45 / 20) = 3
        assert_eq!(s.recommended_replicas(), 3);
    }

    #[test]
    fn empty_queue_set_is_inactive() {
        let s = RabbitMqScaler::new("t");
        assert!(!s.is_active());
        assert_eq!(s.message_count(), 0);
    }
}
