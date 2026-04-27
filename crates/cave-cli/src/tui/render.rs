//! Pure formatters for the TUI.
//!
//! These functions take state and produce strings. Ratatui owns the
//! terminal escape codes; here we keep the layout testable as plain
//! strings — column widths, truncation, header content, status line,
//! help overlay, search bar, paginated viewport.

use crate::tui::app::{AppState, View};
use crate::tui::keymap::Mode;

/// Format the header bar.
///
/// `[VIEW] tenant=<t> | <n> items | follow:on/off`
pub fn format_header(state: &AppState, items_total: usize) -> String {
    let view = state.view.name().to_uppercase();
    let tenant = state
        .tenant
        .as_deref()
        .map(|t| format!("tenant={}", t))
        .unwrap_or_else(|| "tenant=-".to_string());
    let follow = if state.follow { "follow:on" } else { "follow:off" };
    format!(
        "[{}] {} | {} items | {}",
        view, tenant, items_total, follow
    )
}

/// Format the status line at the bottom.
pub fn format_status_line(state: &AppState, mode: Mode) -> String {
    if let Some(msg) = &state.status_message {
        return msg.clone();
    }
    match mode {
        Mode::Search => format!("/{}", state.filter),
        Mode::Command => format!(":{}", state.command_buffer),
        Mode::Normal => "press ? for help".to_string(),
    }
}

/// Format a single item row.
///
/// Selected rows get a `>` cursor; unselected rows get two spaces.
/// Truncates long names with `…`.
pub fn format_item_row(item: &str, selected: bool, width: usize) -> String {
    let cursor = if selected { "> " } else { "  " };
    let inner_width = width.saturating_sub(2);
    let body = trunc_chars(item, inner_width);
    format!("{}{}", cursor, body)
}

/// Format the search bar (only meaningful in Search mode).
pub fn format_search_bar(filter: &str) -> String {
    format!("/{}_", filter)
}

/// Compute the slice of items visible given a viewport height and the
/// current selection. The selected item is always within the viewport.
///
/// Returns `(start_index, slice)` so the caller can render absolute
/// row numbers if it wants to.
pub fn paginate<'a, T>(items: &'a [T], viewport_height: usize, selected: usize) -> (usize, &'a [T]) {
    if items.is_empty() || viewport_height == 0 {
        return (0, &[]);
    }
    let len = items.len();
    let h = viewport_height.min(len);
    if selected < h {
        return (0, &items[..h]);
    }
    if selected >= len {
        // Out of range: show the last page.
        let start = len.saturating_sub(h);
        return (start, &items[start..]);
    }
    // Keep selected at the bottom of the viewport.
    let start = selected + 1 - h;
    (start, &items[start..start + h])
}

/// Format the help overlay.
pub fn format_help() -> String {
    let lines = [
        "Cavectl TUI — keybindings",
        "",
        "Navigation",
        "  j / ↓     move down            k / ↑     move up",
        "  PgDn      jump 10 down         PgUp      jump 10 up",
        "  g / Home  jump to top          G / End   jump to bottom",
        "",
        "Views (number = view)",
        "  1 pods       2 deployments  3 services    4 events",
        "  5 logs       6 topology     7 tenants     8 modules",
        "  9 secrets    0 flags",
        "",
        "Modes",
        "  /          search-filter        :    command",
        "  Esc        cancel                Enter commit",
        "",
        "Other",
        "  f          toggle log follow    ?    toggle this help",
        "  q          quit                 Ctrl-C quit",
    ];
    lines.join("\n")
}

/// Tab bar listing every view, with the active view highlighted by `*`.
///
/// Compact and keyboard-shortcut-prefixed:
/// `[1*pods] [2 deployments] [3 services] ...`
pub fn format_tab_bar(active: View) -> String {
    let entries = [
        (1, View::Pods),
        (2, View::Deployments),
        (3, View::Services),
        (4, View::Events),
        (5, View::Logs),
        (6, View::Topology),
        (7, View::Tenants),
        (8, View::Modules),
        (9, View::Secrets),
        (0, View::Flags),
    ];
    entries
        .iter()
        .map(|(n, v)| {
            let mark = if *v == active { "*" } else { " " };
            format!("[{}{}{}]", n, mark, v.name())
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn trunc_chars(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let cut = max - 1;
    let mut out: String = chars[..cut].iter().collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st_with(view: View, tenant: Option<&str>) -> AppState {
        let mut s = AppState::default();
        s.view = view;
        s.tenant = tenant.map(String::from);
        s
    }

    // ── format_header ───────────────────────────────────────────────────────

    #[test]
    fn header_default() {
        let s = AppState::default();
        let h = format_header(&s, 0);
        assert!(h.contains("[PODS]"));
        assert!(h.contains("tenant=-"));
        assert!(h.contains("0 items"));
        assert!(h.contains("follow:off"));
    }

    #[test]
    fn header_with_tenant() {
        let s = st_with(View::Logs, Some("acme"));
        let h = format_header(&s, 5);
        assert!(h.contains("[LOGS]"));
        assert!(h.contains("tenant=acme"));
        assert!(h.contains("5 items"));
    }

    #[test]
    fn header_follow_on() {
        let mut s = AppState::default();
        s.follow = true;
        let h = format_header(&s, 0);
        assert!(h.contains("follow:on"));
    }

    #[test]
    fn header_view_uppercase() {
        let s = st_with(View::Topology, None);
        assert!(format_header(&s, 0).starts_with("[TOPOLOGY]"));
    }

    // ── format_status_line ──────────────────────────────────────────────────

    #[test]
    fn status_normal_mode_prompts_help() {
        let s = AppState::default();
        assert_eq!(format_status_line(&s, Mode::Normal), "press ? for help");
    }

    #[test]
    fn status_search_mode_shows_filter() {
        let mut s = AppState::default();
        s.filter = "ngx".into();
        assert_eq!(format_status_line(&s, Mode::Search), "/ngx");
    }

    #[test]
    fn status_command_mode_shows_buffer() {
        let mut s = AppState::default();
        s.command_buffer = "logs".into();
        assert_eq!(format_status_line(&s, Mode::Command), ":logs");
    }

    #[test]
    fn status_message_overrides_mode() {
        let mut s = AppState::default();
        s.status_message = Some("saved".into());
        assert_eq!(format_status_line(&s, Mode::Normal), "saved");
        assert_eq!(format_status_line(&s, Mode::Search), "saved");
    }

    // ── format_item_row ─────────────────────────────────────────────────────

    #[test]
    fn item_row_unselected_prefix() {
        let row = format_item_row("nginx", false, 20);
        assert!(row.starts_with("  nginx"));
    }

    #[test]
    fn item_row_selected_prefix() {
        let row = format_item_row("nginx", true, 20);
        assert!(row.starts_with("> nginx"));
    }

    #[test]
    fn item_row_truncates_long_name() {
        let row = format_item_row("supercalifragilistic", false, 12);
        assert!(row.ends_with('…'));
        assert!(row.chars().count() <= 12);
    }

    #[test]
    fn item_row_zero_inner_width() {
        let row = format_item_row("anything", false, 2);
        assert_eq!(row, "  ");
    }

    #[test]
    fn item_row_unicode_safe() {
        let row = format_item_row("café-pod", true, 20);
        assert!(row.contains("café"));
    }

    // ── format_search_bar ───────────────────────────────────────────────────

    #[test]
    fn search_bar_shows_filter_with_caret() {
        assert_eq!(format_search_bar("ngx"), "/ngx_");
    }

    #[test]
    fn search_bar_empty_filter() {
        assert_eq!(format_search_bar(""), "/_");
    }

    // ── paginate ────────────────────────────────────────────────────────────

    #[test]
    fn paginate_selection_in_first_page() {
        let items: Vec<i32> = (0..20).collect();
        let (start, slice) = paginate(&items, 10, 3);
        assert_eq!(start, 0);
        assert_eq!(slice.len(), 10);
        assert_eq!(slice[0], 0);
    }

    #[test]
    fn paginate_selection_at_bottom() {
        let items: Vec<i32> = (0..20).collect();
        let (start, slice) = paginate(&items, 10, 19);
        assert_eq!(start, 10);
        assert_eq!(slice[0], 10);
        assert_eq!(slice[9], 19);
    }

    #[test]
    fn paginate_selection_in_middle() {
        let items: Vec<i32> = (0..20).collect();
        let (start, slice) = paginate(&items, 5, 10);
        // Selected at the bottom of the viewport.
        assert_eq!(start, 6);
        assert_eq!(slice.len(), 5);
        assert_eq!(slice.last().unwrap(), &10);
    }

    #[test]
    fn paginate_empty() {
        let items: Vec<i32> = vec![];
        let (start, slice) = paginate(&items, 10, 0);
        assert_eq!(start, 0);
        assert!(slice.is_empty());
    }

    #[test]
    fn paginate_zero_height() {
        let items: Vec<i32> = (0..5).collect();
        let (_, slice) = paginate(&items, 0, 0);
        assert!(slice.is_empty());
    }

    #[test]
    fn paginate_viewport_larger_than_items() {
        let items: Vec<i32> = (0..3).collect();
        let (start, slice) = paginate(&items, 10, 0);
        assert_eq!(start, 0);
        assert_eq!(slice.len(), 3);
    }

    #[test]
    fn paginate_out_of_range_selection_clamps_to_last_page() {
        let items: Vec<i32> = (0..10).collect();
        let (start, slice) = paginate(&items, 5, 999);
        assert_eq!(start, 5);
        assert_eq!(slice.len(), 5);
    }

    #[test]
    fn paginate_keeps_selection_visible() {
        let items: Vec<i32> = (0..100).collect();
        for sel in [0, 5, 50, 99] {
            let (start, slice) = paginate(&items, 10, sel);
            let visible_end = start + slice.len();
            assert!(
                sel >= start && sel < visible_end,
                "selection {} not in [{}, {})",
                sel,
                start,
                visible_end
            );
        }
    }

    // ── format_help ─────────────────────────────────────────────────────────

    #[test]
    fn help_mentions_navigation_keys() {
        let h = format_help();
        assert!(h.contains("j"));
        assert!(h.contains("k"));
        assert!(h.contains("PgDn"));
        assert!(h.contains("g"));
        assert!(h.contains("G"));
    }

    #[test]
    fn help_mentions_modes() {
        let h = format_help();
        assert!(h.contains("/"));
        assert!(h.contains(":"));
        assert!(h.contains("Esc"));
    }

    #[test]
    fn help_mentions_view_shortcuts() {
        let h = format_help();
        for v in [
            "pods",
            "deployments",
            "services",
            "events",
            "logs",
            "topology",
            "tenants",
            "modules",
            "secrets",
            "flags",
        ] {
            assert!(h.contains(v), "help missing view name `{}`", v);
        }
    }

    #[test]
    fn help_mentions_quit() {
        let h = format_help();
        assert!(h.contains("q"));
        assert!(h.contains("Ctrl-C"));
    }

    // ── format_tab_bar ──────────────────────────────────────────────────────

    #[test]
    fn tab_bar_marks_active() {
        let bar = format_tab_bar(View::Pods);
        assert!(bar.contains("[1*pods]"));
        assert!(bar.contains("[2 deployments]"));
    }

    #[test]
    fn tab_bar_active_changes() {
        let bar = format_tab_bar(View::Logs);
        assert!(bar.contains("[5*logs]"));
        assert!(bar.contains("[1 pods]"));
    }

    #[test]
    fn tab_bar_includes_all_ten_views() {
        let bar = format_tab_bar(View::Pods);
        for v in [
            "pods",
            "deployments",
            "services",
            "events",
            "logs",
            "topology",
            "tenants",
            "modules",
            "secrets",
            "flags",
        ] {
            assert!(bar.contains(v));
        }
    }

    // ── trunc_chars edge cases ──────────────────────────────────────────────

    #[test]
    fn trunc_passes_short_through() {
        assert_eq!(trunc_chars("abc", 10), "abc");
    }

    #[test]
    fn trunc_at_exact_length() {
        assert_eq!(trunc_chars("abcde", 5), "abcde");
    }

    #[test]
    fn trunc_with_ellipsis() {
        let t = trunc_chars("abcdefghij", 5);
        assert_eq!(t.chars().count(), 5);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn trunc_zero_width_returns_empty() {
        assert_eq!(trunc_chars("anything", 0), "");
    }

    #[test]
    fn trunc_unicode_chars_not_bytes() {
        // 5 unicode chars; truncate to 4 → "abc…".
        let t = trunc_chars("abcdé", 4);
        assert_eq!(t.chars().count(), 4);
    }

    // ── integration shape ───────────────────────────────────────────────────

    #[test]
    fn full_screen_rough_layout() {
        let mut s = st_with(View::Pods, Some("acme"));
        s.items = vec!["a".into(), "b".into(), "c".into()];
        s.selected = 1;
        let header = format_header(&s, s.items.len());
        let tab = format_tab_bar(s.view);
        let row0 = format_item_row(&s.items[0], false, 20);
        let row1 = format_item_row(&s.items[1], true, 20);
        let row2 = format_item_row(&s.items[2], false, 20);
        let status = format_status_line(&s, Mode::Normal);
        // Concatenated layout still has all the bits we care about.
        let screen = format!("{}\n{}\n{}\n{}\n{}\n{}", header, tab, row0, row1, row2, status);
        assert!(screen.contains("acme"));
        assert!(screen.contains("> b"));
        assert!(screen.contains("press ? for help"));
    }
}
