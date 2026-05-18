// SPDX-License-Identifier: AGPL-3.0-or-later
//! Events tab — chronological per-node feed derived from pod state.
//!
//! Mirrors the upstream Kubernetes Dashboard's Events view (timestamp,
//! source, type, reason, message). The events here are derived from
//! the kubelet pod set (a Running pod emits a "Started" event, a
//! Failed pod emits a "BackOff" event) so the surface is meaningful
//! without a separate Event resource type in state.

use super::KubeletViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub when_unix: i64,
    pub source: String,
    pub event_type: &'static str, // "Normal" | "Warning"
    pub reason: &'static str,
    pub message: String,
}

pub fn list_events(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<EventRow>, KubeletViewError> {
    let pods = super::pods::list_pods(state, ctx)?;
    let mut out: Vec<EventRow> = pods
        .iter()
        .enumerate()
        .map(|(idx, p)| {
            let base_when = 1_000_500 + (idx as i64) * 30;
            match p.status {
                "Running" => EventRow {
                    when_unix: base_when,
                    source: format!("kubelet:{}", p.node),
                    event_type: "Normal",
                    reason: "Started",
                    message: format!("Started container in pod {}", p.pod_name),
                },
                "Pending" => EventRow {
                    when_unix: base_when,
                    source: format!("kubelet:{}", p.node),
                    event_type: "Warning",
                    reason: "FailedScheduling",
                    message: format!("Pending pod {} awaiting resources", p.pod_name),
                },
                "Failed" => EventRow {
                    when_unix: base_when,
                    source: format!("kubelet:{}", p.node),
                    event_type: "Warning",
                    reason: "BackOff",
                    message: format!("Pod {} in CrashLoopBackOff", p.pod_name),
                },
                _ => EventRow {
                    when_unix: base_when,
                    source: format!("kubelet:{}", p.node),
                    event_type: "Normal",
                    reason: "Updated",
                    message: format!("Pod {} status: {}", p.pod_name, p.status),
                },
            }
        })
        .collect();
    out.sort_by(|a, b| b.when_unix.cmp(&a.when_unix));
    Ok(out)
}

pub fn warning_count(events: &[EventRow]) -> usize {
    events.iter().filter(|e| e.event_type == "Warning").count()
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, KubeletViewError> {
    let events = list_events(state, ctx)?;
    let rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            vec![
                e.when_unix.to_string(),
                e.source.clone(),
                e.event_type.into(),
                e.reason.into(),
                e.message.clone(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="kubelet-events" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Events ({n}, {w} Warning)</h2>
  {tbl}
</section>"#,
        n = events.len(),
        w = warning_count(&events),
        tbl = table(
            &["time", "source", "type", "reason", "message"],
            &rows
        ),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin::permission::Permission;
    use crate::portal_test_ctx;

    fn ctx(perms: &[Permission]) -> RequestCtx {
        RequestCtx::developer("acme", perms)
    }

    #[test]
    fn list_events_sorts_newest_first() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Events/EventList.tsx",
            "EventList",
            "acme"
        );
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        for w in events.windows(2) {
            assert!(w[0].when_unix >= w[1].when_unix);
        }
    }

    #[test]
    fn list_events_emits_one_row_per_pod() {
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        let pods = super::super::pods::list_pods(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        assert_eq!(events.len(), pods.len());
    }

    #[test]
    fn list_events_requires_kubelet_read() {
        let s = AdminState::seeded();
        assert!(list_events(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn warning_count_matches_warning_rows() {
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        let manual: usize = events.iter().filter(|e| e.event_type == "Warning").count();
        assert_eq!(warning_count(&events), manual);
    }

    #[test]
    fn render_section_includes_reason_column() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::KubeletRead])).unwrap();
        for col in ["time", "source", "type", "reason", "message"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
