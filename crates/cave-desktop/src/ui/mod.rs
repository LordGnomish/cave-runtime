// SPDX-License-Identifier: AGPL-3.0-or-later
// UI primitives for the cave-desktop native shell.
//
// These are data-shape skeletons. Real rendering lands when the
// `gpui-runtime` feature is wired up — see ADR-PORTAL-DESKTOP-001.

pub mod panel;
pub mod metric_card;
pub mod table;

pub use panel::Panel;
pub use metric_card::MetricCard;
pub use table::Table;
