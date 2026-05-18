// SPDX-License-Identifier: AGPL-3.0-or-later
//! TUI mode for `cavectl` — k9s-style terminal-first console.
//!
//! Per ADR-RUNTIME-CLI-CONSOLIDATION-001 M5: a pure-state TUI that
//! lets operators navigate Cave resources without leaving the
//! terminal. The reducer (`AppState` + `Action` + `reduce`) is
//! deliberately decoupled from ratatui rendering so it can be unit-
//! tested without a TTY.

pub mod app;
pub mod filter;
pub mod keymap;
pub mod render;

pub use app::{AppState, Action, View, reduce};
pub use filter::{fuzzy_match, fuzzy_score};
pub use keymap::{Key, KeyAction, default_keymap};
pub use render::{
    format_header, format_help, format_item_row, format_search_bar, format_status_line,
    format_tab_bar, paginate,
};
