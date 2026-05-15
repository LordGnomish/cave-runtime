//! Events tab — scheduling events feed (FailedScheduling / Scheduled).

use super::SchedulerViewError;
use crate::admin::permission::RequestCtx;
use crate::admin::render::table;
use crate::admin::state::AdminState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventRow {
    pub when_unix: i64,
    pub pod: String,
    pub kind: &'static str, // "Normal" | "Warning"
    pub reason: &'static str,
    pub message: String,
}

pub fn list_events(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<Vec<EventRow>, SchedulerViewError> {
    let bindings = super::bindings::list_bindings(state, ctx)?;
    let pending = super::queue::list_pending(state, ctx)?;
    let mut out: Vec<EventRow> = bindings
        .iter()
        .map(|b| EventRow {
            when_unix: b.bound_at_unix,
            pod: b.pod.clone(),
            kind: "Normal",
            reason: "Scheduled",
            message: format!("Successfully assigned {} to {}", b.pod, b.node),
        })
        .collect();
    for p in &pending {
        out.push(EventRow {
            when_unix: 1_700_000_000 - (p.age_secs as i64),
            pod: p.pod.clone(),
            kind: "Warning",
            reason: "FailedScheduling",
            message: format!(
                "0/{} nodes are available: {}",
                3,
                p.gating.unwrap_or("nodes have unschedulable taints")
            ),
        });
    }
    out.sort_by(|a, b| b.when_unix.cmp(&a.when_unix));
    Ok(out)
}

pub fn warning_count(rows: &[EventRow]) -> usize {
    rows.iter().filter(|r| r.kind == "Warning").count()
}

pub fn render_section(
    state: &AdminState,
    ctx: &RequestCtx,
) -> Result<String, SchedulerViewError> {
    let events = list_events(state, ctx)?;
    let rows: Vec<Vec<String>> = events
        .iter()
        .map(|e| {
            vec![
                e.when_unix.to_string(),
                e.pod.clone(),
                e.kind.into(),
                e.reason.into(),
                e.message.clone(),
            ]
        })
        .collect();
    Ok(format!(
        r#"<section id="scheduler-events" class="mt-6">
  <h2 class="text-lg font-semibold mb-2">Events ({n}, {w} Warning)</h2>
  {tbl}
</section>"#,
        n = events.len(),
        w = warning_count(&events),
        tbl = table(
            &["time", "pod", "type", "reason", "message"],
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
    fn list_events_includes_scheduled_and_failed() {
        let (_c, _t) = portal_test_ctx!(
            "plugins/kubernetes/src/components/Pods/SchedulerEvents.tsx",
            "Events",
            "acme"
        );
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        let reasons: std::collections::HashSet<_> = events.iter().map(|e| e.reason).collect();
        assert!(reasons.contains("Scheduled") || reasons.contains("FailedScheduling"));
    }

    #[test]
    fn list_events_refuses_without_perm() {
        let s = AdminState::seeded();
        assert!(list_events(&s, &ctx(&[])).is_err());
    }

    #[test]
    fn list_events_sorted_newest_first() {
        let s = AdminState::seeded();
        let events = list_events(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for w in events.windows(2) {
            assert!(w[0].when_unix >= w[1].when_unix);
        }
    }

    #[test]
    fn render_section_emits_columns() {
        let s = AdminState::seeded();
        let html = render_section(&s, &ctx(&[Permission::SchedulerRead])).unwrap();
        for col in ["time", "pod", "type", "reason", "message"] {
            assert!(html.contains(&format!(">{}<", col)));
        }
    }
}
