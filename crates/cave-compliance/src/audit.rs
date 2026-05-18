// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::models::AuditEvent;
use uuid::Uuid;

pub fn record_event(actor: &str, action: &str, resource_type: &str, resource_id: &str, details: serde_json::Value) -> AuditEvent {
    AuditEvent {
        id: Uuid::new_v4(),
        actor: actor.to_string(),
        action: action.to_string(),
        resource_type: resource_type.to_string(),
        resource_id: resource_id.to_string(),
        details,
        ip_address: None,
        user_agent: None,
        occurred_at: chrono::Utc::now(),
    }
}

pub fn filter_events<'a>(events: &'a [AuditEvent], resource_type: Option<&str>, actor: Option<&str>) -> Vec<&'a AuditEvent> {
    events.iter().filter(|e| {
        resource_type.map_or(true, |rt| e.resource_type == rt) &&
        actor.map_or(true, |a| e.actor == a)
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_record_event() {
        let ev = record_event("admin", "create", "finding", "abc", serde_json::json!({}));
        assert_eq!(ev.actor, "admin");
        assert_eq!(ev.action, "create");
    }
    #[test]
    fn test_filter_events() {
        let events = vec![
            record_event("alice", "read", "control", "1", serde_json::json!({})),
            record_event("bob", "update", "finding", "2", serde_json::json!({})),
        ];
        let alice_events = filter_events(&events, None, Some("alice"));
        assert_eq!(alice_events.len(), 1);
    }
}
