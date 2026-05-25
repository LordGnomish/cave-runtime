// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! SMS + voice gateway — `engine/apps/alerts/sms_gateway + voice`.
//!
//! Ports OnCall's notify-by-SMS and notify-by-voice surfaces. Upstream
//! uses Twilio's REST API exclusively; this port models the *provider
//! interface* and adds a Twilio adapter shape so cave-oncall can later
//! drive Twilio, AWS End-User-Messaging, MessageBird, or any compatible
//! SMS provider without rewiring callers.
//!
//! Mapped surfaces:
//! * `engine/apps/twilioapp/twilio_client.py`         — outbound SMS
//! * `engine/apps/twilioapp/voice_renderer.py`        — TwiML voice template
//! * `engine/apps/alerts/sms_gateway/dispatcher.py`   — per-user routing
//! * `engine/apps/alerts/voice_gateway/dispatcher.py` — voice call routing

use crate::models::{Alert, User};
use std::sync::Mutex;

/// One delivery attempt — outbound to either SMS or voice channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Channel {
    Sms,
    Voice,
}

#[derive(Debug, Clone)]
pub struct DeliveryRequest {
    pub channel: Channel,
    pub to_phone: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryStatus {
    Queued,
    Sent,
    Failed(String),
}

pub trait SmsVoiceProvider: Send + Sync {
    fn send(&self, req: &DeliveryRequest) -> DeliveryStatus;
    fn name(&self) -> &'static str;
}

/// Twilio-shaped provider — `From` is the Twilio number used as caller-id.
pub struct TwilioProvider {
    pub account_sid: String,
    pub auth_token: String,
    pub from_number: String,
    pub sent: Mutex<Vec<DeliveryRequest>>,
}

impl TwilioProvider {
    pub fn new(
        account_sid: impl Into<String>,
        auth_token: impl Into<String>,
        from_number: impl Into<String>,
    ) -> Self {
        Self {
            account_sid: account_sid.into(),
            auth_token: auth_token.into(),
            from_number: from_number.into(),
            sent: Mutex::new(Vec::new()),
        }
    }
}

impl SmsVoiceProvider for TwilioProvider {
    fn send(&self, req: &DeliveryRequest) -> DeliveryStatus {
        if !is_valid_e164(&req.to_phone) {
            return DeliveryStatus::Failed(format!("invalid E.164: {}", req.to_phone));
        }
        if req.body.is_empty() {
            return DeliveryStatus::Failed("empty body".into());
        }
        self.sent.lock().unwrap().push(req.clone());
        DeliveryStatus::Sent
    }
    fn name(&self) -> &'static str {
        "twilio"
    }
}

/// Validate E.164 phone shape (`+` followed by 8-15 digits).
pub fn is_valid_e164(p: &str) -> bool {
    if !p.starts_with('+') {
        return false;
    }
    let digits = &p[1..];
    digits.len() >= 8 && digits.len() <= 15 && digits.chars().all(|c| c.is_ascii_digit())
}

/// SMS body renderer — upstream's `format_sms_body` produces a fixed-shape
/// "[OnCall] <severity> <title>" line plus a deep link, capped at 160 chars.
pub fn render_sms_body(alert: &Alert) -> String {
    let sev = format!("{:?}", alert.severity);
    let base = format!("[OnCall] {} {}", sev, alert.title);
    if base.len() <= 160 {
        base
    } else {
        format!("{}…", &base[..159])
    }
}

/// Voice (TwiML) body renderer — upstream's `voice_renderer` builds a
/// `<Response><Say>…</Say></Response>` script.
pub fn render_voice_twiml(alert: &Alert) -> String {
    let sev = format!("{:?}", alert.severity);
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<Response><Say>Severity {sev}. Alert: {title}. Press 1 to acknowledge, 2 to escalate.</Say></Response>",
        sev = sev,
        title = xml_escape(&alert.title)
    )
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Dispatcher: route an alert to the correct phone+channel for the user.
pub struct Dispatcher<P: SmsVoiceProvider> {
    pub provider: P,
}

impl<P: SmsVoiceProvider> Dispatcher<P> {
    pub fn new(provider: P) -> Self {
        Self { provider }
    }

    pub fn dispatch_sms(&self, user: &User, alert: &Alert) -> DeliveryStatus {
        let Some(phone) = user.phone.as_deref() else {
            return DeliveryStatus::Failed("user has no phone".into());
        };
        let req = DeliveryRequest {
            channel: Channel::Sms,
            to_phone: phone.to_string(),
            body: render_sms_body(alert),
        };
        self.provider.send(&req)
    }

    pub fn dispatch_voice(&self, user: &User, alert: &Alert) -> DeliveryStatus {
        let Some(phone) = user.phone.as_deref() else {
            return DeliveryStatus::Failed("user has no phone".into());
        };
        let req = DeliveryRequest {
            channel: Channel::Voice,
            to_phone: phone.to_string(),
            body: render_voice_twiml(alert),
        };
        self.provider.send(&req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{AlertState, Severity};
    use chrono::Utc;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn mk_alert(title: &str) -> Alert {
        Alert {
            id: Uuid::new_v4(),
            team_id: Uuid::new_v4(),
            title: title.into(),
            severity: Severity::Critical,
            source: "prom".into(),
            fingerprint: "fp".into(),
            state: AlertState::Firing,
            labels: HashMap::new(),
            annotations: HashMap::new(),
            created_at: Utc::now(),
            ack_at: None,
            ack_by: None,
            resolved_at: None,
            escalation_policy_id: None,
            current_escalation_step: 0,
        }
    }

    fn mk_user(phone: Option<&str>) -> User {
        User {
            id: Uuid::new_v4(),
            username: "alice".into(),
            email: "a@x".into(),
            display_name: "alice".into(),
            timezone: "UTC".into(),
            phone: phone.map(|p| p.to_string()),
            slack_id: None,
            active: true,
        }
    }

    #[test]
    fn e164_valid_examples() {
        assert!(is_valid_e164("+14155551234"));
        assert!(is_valid_e164("+905551234567"));
        assert!(is_valid_e164("+12345678"));
    }

    #[test]
    fn e164_rejects_missing_plus_or_letters() {
        assert!(!is_valid_e164("14155551234"));
        assert!(!is_valid_e164("+abc12345678"));
        assert!(!is_valid_e164(""));
        assert!(!is_valid_e164("+12"));
    }

    #[test]
    fn render_sms_body_includes_severity_and_title() {
        let a = mk_alert("Disk full");
        let body = render_sms_body(&a);
        assert!(body.starts_with("[OnCall] Critical"));
        assert!(body.contains("Disk full"));
    }

    #[test]
    fn render_sms_body_truncates_to_160_chars() {
        let long = "X".repeat(500);
        let a = mk_alert(&long);
        let body = render_sms_body(&a);
        assert!(body.len() <= 160 + "…".len());
        assert!(body.ends_with("…"));
    }

    #[test]
    fn voice_twiml_is_valid_xml_shape() {
        let a = mk_alert("DB down");
        let xml = render_voice_twiml(&a);
        assert!(xml.starts_with("<?xml"));
        assert!(xml.contains("<Response>"));
        assert!(xml.contains("DB down"));
        assert!(xml.contains("Press 1"));
    }

    #[test]
    fn dispatch_sms_records_in_provider() {
        let prov = TwilioProvider::new("AC1", "tok", "+15550001234");
        let user = mk_user(Some("+14155551234"));
        let alert = mk_alert("Disk");
        let d = Dispatcher::new(prov);
        let st = d.dispatch_sms(&user, &alert);
        assert_eq!(st, DeliveryStatus::Sent);
        assert_eq!(d.provider.sent.lock().unwrap().len(), 1);
        assert_eq!(d.provider.sent.lock().unwrap()[0].channel, Channel::Sms);
    }

    #[test]
    fn dispatch_voice_records_in_provider() {
        let prov = TwilioProvider::new("AC1", "tok", "+15550001234");
        let user = mk_user(Some("+14155551234"));
        let alert = mk_alert("Pager");
        let d = Dispatcher::new(prov);
        let st = d.dispatch_voice(&user, &alert);
        assert_eq!(st, DeliveryStatus::Sent);
        assert_eq!(d.provider.sent.lock().unwrap()[0].channel, Channel::Voice);
    }

    #[test]
    fn dispatch_fails_when_user_has_no_phone() {
        let prov = TwilioProvider::new("AC1", "tok", "+15550001234");
        let user = mk_user(None);
        let alert = mk_alert("X");
        let d = Dispatcher::new(prov);
        assert!(matches!(d.dispatch_sms(&user, &alert), DeliveryStatus::Failed(_)));
    }

    #[test]
    fn twilio_provider_rejects_invalid_e164() {
        let prov = TwilioProvider::new("AC1", "tok", "+15550001234");
        let req = DeliveryRequest {
            channel: Channel::Sms,
            to_phone: "not-a-phone".into(),
            body: "hi".into(),
        };
        assert!(matches!(prov.send(&req), DeliveryStatus::Failed(_)));
    }

    #[test]
    fn xml_escape_handles_special_chars() {
        assert_eq!(xml_escape("a & b <c>"), "a &amp; b &lt;c&gt;");
    }
}
