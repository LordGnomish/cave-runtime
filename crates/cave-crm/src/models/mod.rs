// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Twenty CRM data model — mirrors `packages/twenty-server/src/modules/`
//! workspace-entity definitions at upstream `twentyhq/twenty` v2.6.0.
//!
//! Per ADR-145 (CRM Upstream Selection — Twenty), cave-crm is a function-
//! based reimplementation of Twenty's data model and HTTP/GraphQL surface.
//! No upstream source is vendored; the entity shapes here are independent
//! Rust definitions whose semantics match Twenty's TypeORM @WorkspaceEntity
//! decorators line-by-line per parity manifest mappings.

pub mod activity;
pub mod api_key;
pub mod calendar_event;
pub mod company;
pub mod custom_field;
pub mod custom_object;
pub mod lead;
pub mod opportunity;
pub mod person;
pub mod pipeline_step;
pub mod task;
pub mod user;
pub mod view;
pub mod workspace;

pub use activity::{Activity, ActivityKind, ActivityTarget, ActivityTargetKind, Note};
pub use api_key::ApiKey;
pub use calendar_event::{CalendarEvent, CalendarEventAttendee, CalendarEventVisibility};
pub use company::Company;
pub use custom_field::{FieldKind, FieldMetadata};
pub use custom_object::ObjectMetadata;
pub use lead::{Lead, LeadStatus};
pub use opportunity::{Opportunity, OpportunityStatus};
pub use person::Person;
pub use pipeline_step::PipelineStep;
pub use task::{Task, TaskStatus};
pub use user::User;
pub use view::{View, ViewKind};
pub use workspace::{Workspace, WorkspaceMember, WorkspaceMemberRole};
