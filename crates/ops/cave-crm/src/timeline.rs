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

use crate::models::{ActivityKind, ActivityTarget, ActivityTargetKind, Note, Task};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TimelineItemKind {
    Note,
    Task,
    CalendarEvent,
    Message,
    /// A `TimelineActivity` audit row (e.g. a field-change diff).
    FieldUpdate,
}

/// An audit row — Twenty's `TimelineActivity` workspace-entity. Captures a
/// named event (`opportunity.updated`, `note.created`, …) against a
/// polymorphic target, with an optional `properties.diff` payload.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineActivity {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub name: String,
    pub target_kind: ActivityTargetKind,
    pub target_id: Uuid,
    pub linked_record_id: Option<Uuid>,
    pub workspace_member_id: Option<Uuid>,
    pub properties: Value,
    pub happens_at: DateTime<Utc>,
}

impl TimelineActivity {
    pub fn new(
        workspace_id: Uuid,
        name: impl Into<String>,
        target_kind: ActivityTargetKind,
        target_id: Uuid,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            workspace_id,
            name: name.into(),
            target_kind,
            target_id,
            linked_record_id: None,
            workspace_member_id: None,
            properties: Value::Null,
            happens_at: Utc::now(),
        }
    }

    pub fn with_properties(mut self, properties: Value) -> Self {
        self.properties = properties;
        self
    }

    /// Two audit rows merge when they describe the same event against the
    /// same target by the same actor (and the same linked record, if any) —
    /// the recency window is applied separately by [`merge_recent`].
    fn mergeable_with(&self, other: &Self) -> bool {
        self.name == other.name
            && self.target_kind == other.target_kind
            && self.target_id == other.target_id
            && self.workspace_member_id == other.workspace_member_id
            && self.linked_record_id == other.linked_record_id
    }
}

/// One entry in a record's timeline feed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TimelineEntry {
    pub kind: TimelineItemKind,
    /// The note/task/audit record id.
    pub record_id: Uuid,
    pub title: String,
    pub happens_at: DateTime<Utc>,
}

/// Build a record's timeline feed: every Note and Task linked to the target
/// via `ActivityTarget`, plus every `TimelineActivity` audit row pointed at
/// the target — ordered newest-first (Twenty orders `createdAt`/`happensAt`
/// DESC).
pub fn timeline_for_target(
    target_kind: ActivityTargetKind,
    target_id: Uuid,
    targets: &[ActivityTarget],
    notes: &[Note],
    tasks: &[Task],
    audits: &[TimelineActivity],
) -> Vec<TimelineEntry> {
    let mut feed: Vec<TimelineEntry> = Vec::new();

    // Polymorphic links pointing at this record.
    let linked: Vec<&ActivityTarget> = targets
        .iter()
        .filter(|t| t.target_kind == target_kind && t.target_id == target_id)
        .collect();

    for note in notes {
        let hit = linked
            .iter()
            .any(|t| t.activity_kind == ActivityKind::Note && t.activity_id == note.id);
        if hit {
            feed.push(TimelineEntry {
                kind: TimelineItemKind::Note,
                record_id: note.id,
                title: note.title.clone(),
                happens_at: note.created_at,
            });
        }
    }

    for task in tasks {
        let hit = linked
            .iter()
            .any(|t| t.activity_kind == ActivityKind::Task && t.activity_id == task.id);
        if hit {
            feed.push(TimelineEntry {
                kind: TimelineItemKind::Task,
                record_id: task.id,
                title: task.title.clone(),
                happens_at: task.created_at,
            });
        }
    }

    for a in audits {
        if a.target_kind == target_kind && a.target_id == target_id {
            feed.push(TimelineEntry {
                kind: TimelineItemKind::FieldUpdate,
                record_id: a.id,
                title: a.name.clone(),
                happens_at: a.happens_at,
            });
        }
    }

    feed.sort_by(|a, b| b.happens_at.cmp(&a.happens_at));
    feed
}

/// Collapse audit rows that are mergeable and fall within `window` of one
/// another (Twenty's 10-minute timeline dedup). Keeps the newest of each
/// merged cluster. Input order is irrelevant; output is newest-first.
pub fn merge_recent(activities: &[TimelineActivity], window: Duration) -> Vec<TimelineActivity> {
    let mut sorted: Vec<TimelineActivity> = activities.to_vec();
    sorted.sort_by(|a, b| b.happens_at.cmp(&a.happens_at));

    let mut kept: Vec<TimelineActivity> = Vec::new();
    for act in sorted {
        let dup = kept.iter().any(|k| {
            k.mergeable_with(&act) && (k.happens_at - act.happens_at).abs() <= window
        });
        if !dup {
            kept.push(act);
        }
    }
    kept
}

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
