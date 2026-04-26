use crate::models::{Alert, AlertGroup, AlertStats, AlertStatus, Receiver, Silence, SilenceMatcher};
use chrono::Utc;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

#[derive(Default)]
pub struct AlertStore {
    pub alerts: RwLock<HashMap<Uuid, Alert>>,
    pub groups: RwLock<HashMap<Uuid, AlertGroup>>,
    pub silences: RwLock<HashMap<Uuid, Silence>>,
    pub receivers: RwLock<HashMap<Uuid, Receiver>>,
}

impl AlertStore {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    // -----------------------------------------------------------------------
    // Alerts
    // -----------------------------------------------------------------------

    pub fn insert_alert(&self, alert: Alert) {
        self.alerts.write().unwrap().insert(alert.id, alert);
    }

    pub fn get_alert(&self, id: Uuid) -> Option<Alert> {
        self.alerts.read().unwrap().get(&id).cloned()
    }

    pub fn list_alerts(&self) -> Vec<Alert> {
        self.alerts.read().unwrap().values().cloned().collect()
    }

    pub fn update_alert(&self, alert: Alert) -> bool {
        let mut map = self.alerts.write().unwrap();
        if map.contains_key(&alert.id) {
            map.insert(alert.id, alert);
            true
        } else {
            false
        }
    }

    pub fn remove_alert(&self, id: Uuid) -> Option<Alert> {
        self.alerts.write().unwrap().remove(&id)
    }

    /// Return alerts optionally filtered by status/severity/source.
    pub fn filter_alerts(
        &self,
        status: Option<&str>,
        severity: Option<&str>,
        source: Option<&str>,
    ) -> Vec<Alert> {
        self.alerts
            .read()
            .unwrap()
            .values()
            .filter(|a| {
                if let Some(s) = status {
                    let s_str = serde_json::to_string(&a.status)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    if s_str != s {
                        return false;
                    }
                }
                if let Some(sev) = severity {
                    let sev_str = serde_json::to_string(&a.severity)
                        .unwrap_or_default()
                        .trim_matches('"')
                        .to_string();
                    if sev_str != sev {
                        return false;
                    }
                }
                if let Some(src) = source {
                    if a.source != src {
                        return false;
                    }
                }
                true
            })
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // Silences
    // -----------------------------------------------------------------------

    pub fn insert_silence(&self, silence: Silence) {
        self.silences.write().unwrap().insert(silence.id, silence);
    }

    pub fn get_silence(&self, id: Uuid) -> Option<Silence> {
        self.silences.read().unwrap().get(&id).cloned()
    }

    pub fn list_silences(&self) -> Vec<Silence> {
        self.silences.read().unwrap().values().cloned().collect()
    }

    /// Expire a silence (set active=false and ends_at=now).
    pub fn expire_silence(&self, id: Uuid) -> bool {
        let mut map = self.silences.write().unwrap();
        if let Some(s) = map.get_mut(&id) {
            s.active = false;
            s.ends_at = Utc::now();
            true
        } else {
            false
        }
    }

    /// Find all currently active silences whose matchers match the given labels.
    pub fn find_active_silences(&self, labels: &HashMap<String, String>) -> Vec<Silence> {
        let now = Utc::now();
        self.silences
            .read()
            .unwrap()
            .values()
            .filter(|s| {
                s.active
                    && s.starts_at <= now
                    && s.ends_at > now
                    && silence_matches_labels(&s.matchers, labels)
            })
            .cloned()
            .collect()
    }

    // -----------------------------------------------------------------------
    // Receivers
    // -----------------------------------------------------------------------

    pub fn insert_receiver(&self, receiver: Receiver) {
        self.receivers.write().unwrap().insert(receiver.id, receiver);
    }

    pub fn list_receivers(&self) -> Vec<Receiver> {
        self.receivers.read().unwrap().values().cloned().collect()
    }

    // -----------------------------------------------------------------------
    // Groups
    // -----------------------------------------------------------------------

    /// Group firing alerts by the union of their label keys (common grouping).
    pub fn group_alerts(&self) -> Vec<AlertGroup> {
        let alerts = self.alerts.read().unwrap();
        let mut buckets: HashMap<String, Vec<Alert>> = HashMap::new();

        for alert in alerts.values() {
            if alert.status != AlertStatus::Firing {
                continue;
            }
            // Group key: sorted label key=value pairs
            let mut parts: Vec<String> =
                alert.labels.iter().map(|(k, v)| format!("{k}={v}")).collect();
            parts.sort();
            let key = parts.join(",");
            buckets.entry(key).or_default().push(alert.clone());
        }

        let mut groups_lock = self.groups.write().unwrap();
        groups_lock.clear();

        buckets
            .into_iter()
            .map(|(key, group_alerts)| {
                // Build common labels from the group key
                let labels: HashMap<String, String> = key
                    .split(',')
                    .filter_map(|kv| {
                        let mut it = kv.splitn(2, '=');
                        let k = it.next()?;
                        let v = it.next()?;
                        Some((k.to_string(), v.to_string()))
                    })
                    .collect();

                let group = AlertGroup {
                    id: Uuid::new_v4(),
                    name: key.clone(),
                    labels,
                    alerts: group_alerts,
                    created_at: Utc::now(),
                };
                groups_lock.insert(group.id, group.clone());
                group
            })
            .collect()
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    pub fn compute_stats(&self) -> AlertStats {
        let alerts = self.alerts.read().unwrap();
        let mut stats = AlertStats::default();
        stats.total = alerts.len() as u64;

        for a in alerts.values() {
            match a.status {
                AlertStatus::Firing => stats.firing += 1,
                AlertStatus::Resolved => stats.resolved += 1,
                AlertStatus::Silenced => stats.silenced += 1,
                AlertStatus::Acknowledged => stats.acknowledged += 1,
            }
            let sev_key = serde_json::to_string(&a.severity)
                .unwrap_or_default()
                .trim_matches('"')
                .to_string();
            *stats.by_severity.entry(sev_key).or_insert(0) += 1;
        }

        stats
    }
}

fn silence_matches_labels(matchers: &[SilenceMatcher], labels: &HashMap<String, String>) -> bool {
    matchers.iter().all(|m| {
        let label_value = labels.get(&m.name);
        match label_value {
            None => false,
            Some(v) => {
                if m.is_regex {
                    v.contains(m.value.as_str())
                } else if m.is_equal {
                    v == &m.value
                } else {
                    v != &m.value
                }
            }
        }
    })
}
