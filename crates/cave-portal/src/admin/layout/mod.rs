// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! UX foundation — global chrome (nav, sidebar, breadcrumb, footer),
//! command palette, keyboard shortcuts, dark-mode toggle, toast +
//! skeleton components.
//!
//! Server-rendered. Every helper returns a `String` of HTML; inline
//! `<script>`/`<style>` blocks ship the small amount of JS that's
//! genuinely necessary (keyboard handling + cookie reads). No SPA
//! framework — the existing htmx is already loaded by the shell.
//!
//! ## Why this lives in `admin/`
//!
//! The legacy `cave-portal/src/admin/render.rs::page_shell` is what
//! every existing handler renders. Extending it here keeps the
//! adopter surface tiny: handlers continue calling `page_shell(title,
//! body)` and get the new chrome for free. The legacy markup
//! (`<h1>{title}</h1>` + `<main>...</main>`) is preserved inside the
//! richer wrapper so the 1000+ existing tests don't churn.

pub mod a11y;
pub mod breadcrumb;
pub mod command_palette;
pub mod footer;
pub mod help;
pub mod nav;
pub mod shell;
pub mod shortcuts;
pub mod skeleton;
pub mod theme;
pub mod toast;

pub use a11y::{A11yCode, A11yIssue, audit as a11y_audit};
pub use breadcrumb::{Crumb, breadcrumb_for_path};
pub use command_palette::{CommandItem, command_palette_modal};
pub use footer::footer;
pub use help::{empty_state, tooltip};
pub use nav::{NavItem, nav_items_for_persona, sidebar};
pub use shell::{ShellOptions, shell_v2};
pub use shortcuts::{DEFAULT_BINDINGS, ShortcutBinding, shortcuts_help_modal};
pub use skeleton::{error_panel, skeleton_table};
pub use theme::{ThemePreference, theme_class_for_cookie};
pub use toast::toast_container;
