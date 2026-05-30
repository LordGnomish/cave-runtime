// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Timeline activity aggregation —
//! `packages/twenty-server/src/modules/timeline/services/timeline-activity.service.ts`.
//!
//! Twenty surfaces a per-record activity feed by walking the polymorphic
//! `noteTarget`/`taskTarget` links for a given Person/Company/Opportunity and
//! merging in `TimelineActivity` audit rows (field-change diffs), ordered
//! newest-first. Recent audit rows (< 10 min) for the same target + actor +
//! linked record + event name are merged. This module ports that read-side
//! aggregation contract — closing the `[[partial]]` timeline gap that the
//! bare `ActivityTarget` link only enabled.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ActivityKind, ActivityTarget, ActivityTargetKind, Note, Task};
    use chrono::{Duration, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn target(
        ws: Uuid,
        activity_id: Uuid,
        kind: ActivityKind,
        tk: ActivityTargetKind,
        tid: Uuid,
    ) -> ActivityTarget {
        ActivityTarget::new(ws, activity_id, kind, tk, tid)
    }

    #[test]
    fn aggregates_notes_and_tasks_for_a_target_newest_first() {
        let ws = Uuid::new_v4();
        let person = Uuid::new_v4();

        let mut note = Note::new(ws, "Kickoff call");
        note.created_at = Utc::now() - Duration::hours(2);
        let mut task = Task::new(ws, "Send proposal");
        task.created_at = Utc::now() - Duration::minutes(5);

        let targets = vec![
            target(ws, note.id, ActivityKind::Note, ActivityTargetKind::Person, person),
            target(ws, task.id, ActivityKind::Task, ActivityTargetKind::Person, person),
        ];

        let feed = timeline_for_target(
            ActivityTargetKind::Person,
            person,
            &targets,
            &[note.clone()],
            &[task.clone()],
            &[],
        );
        assert_eq!(feed.len(), 2);
        // Newest first → the task (5 min ago) precedes the note (2 h ago).
        assert_eq!(feed[0].kind, TimelineItemKind::Task);
        assert_eq!(feed[0].title, "Send proposal");
        assert_eq!(feed[1].kind, TimelineItemKind::Note);
    }

    #[test]
    fn ignores_activities_linked_to_other_records() {
        let ws = Uuid::new_v4();
        let alice = Uuid::new_v4();
        let bob = Uuid::new_v4();
        let note = Note::new(ws, "About Bob");
        let targets = vec![target(
            ws,
            note.id,
            ActivityKind::Note,
            ActivityTargetKind::Person,
            bob,
        )];
        let feed = timeline_for_target(
            ActivityTargetKind::Person,
            alice,
            &targets,
            &[note],
            &[],
            &[],
        );
        assert!(feed.is_empty());
    }

    #[test]
    fn includes_field_update_audit_rows() {
        let ws = Uuid::new_v4();
        let opp = Uuid::new_v4();
        let audit = TimelineActivity::new(
            ws,
            "opportunity.updated",
            ActivityTargetKind::Opportunity,
            opp,
        )
        .with_properties(json!({ "diff": { "stage": ["Discovery", "Proposal"] } }));

        let feed = timeline_for_target(
            ActivityTargetKind::Opportunity,
            opp,
            &[],
            &[],
            &[],
            &[audit],
        );
        assert_eq!(feed.len(), 1);
        assert_eq!(feed[0].kind, TimelineItemKind::FieldUpdate);
        assert_eq!(feed[0].title, "opportunity.updated");
    }

    #[test]
    fn merges_recent_duplicate_audit_rows_within_window() {
        let ws = Uuid::new_v4();
        let opp = Uuid::new_v4();
        let member = Uuid::new_v4();
        let now = Utc::now();

        let mut a = TimelineActivity::new(ws, "opportunity.updated", ActivityTargetKind::Opportunity, opp);
        a.workspace_member_id = Some(member);
        a.happens_at = now - Duration::minutes(2);
        let mut b = TimelineActivity::new(ws, "opportunity.updated", ActivityTargetKind::Opportunity, opp);
        b.workspace_member_id = Some(member);
        b.happens_at = now;
        // Different event, same record → not merged.
        let mut c = TimelineActivity::new(ws, "note.created", ActivityTargetKind::Opportunity, opp);
        c.workspace_member_id = Some(member);
        c.happens_at = now;

        let merged = merge_recent(&[a, b, c], Duration::minutes(10));
        // a+b collapse into one; c stays separate → 2 rows.
        assert_eq!(merged.len(), 2);
    }
}
