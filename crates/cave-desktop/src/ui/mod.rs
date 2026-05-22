// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// UI primitives for the cave-desktop native shell.
//
// These are data-shape skeletons. Real rendering lands when the
// `gpui-runtime` feature is wired up — see ADR-PORTAL-DESKTOP-001.

pub mod metric_card;
pub mod panel;
pub mod table;

pub use metric_card::MetricCard;
pub use panel::Panel;
pub use table::Table;
