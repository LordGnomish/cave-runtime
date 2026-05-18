// SPDX-License-Identifier: AGPL-3.0-or-later
//! Pure state reducer for the TUI.
//!
//! `AppState` is the entire UI state. `Action` is the union of every
//! input or async event. `reduce(state, action)` is the only state
//! transition — and it's pure, which makes the whole TUI testable
//! without a terminal.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppState {
    pub view: View,
    pub items: Vec<String>,
    pub selected: usize,
    pub filter: String,
    pub search_mode: bool,
    pub command_mode: bool,
    pub command_buffer: String,
    pub status_message: Option<String>,
    pub follow: bool,
    pub quit: bool,
    pub help_visible: bool,
    pub tenant: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            view: View::Pods,
            items: Vec::new(),
            selected: 0,
            filter: String::new(),
            search_mode: false,
            command_mode: false,
            command_buffer: String::new(),
            status_message: None,
            follow: false,
            quit: false,
            help_visible: false,
            tenant: None,
        }
    }
}

/// Top-level resource view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum View {
    Pods,
    Deployments,
    Services,
    Events,
    Logs,
    Topology,
    Tenants,
    Modules,
    Secrets,
    Flags,
}

impl View {
    pub fn name(&self) -> &'static str {
        match self {
            View::Pods => "pods",
            View::Deployments => "deployments",
            View::Services => "services",
            View::Events => "events",
            View::Logs => "logs",
            View::Topology => "topology",
            View::Tenants => "tenants",
            View::Modules => "modules",
            View::Secrets => "secrets",
            View::Flags => "flags",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Move selection by `delta` (negative = up, positive = down).
    Move(i32),
    /// Jump selection to a position (0..items.len()).
    JumpTo(usize),
    /// Switch the active view.
    SwitchView(View),
    /// Replace the items list (after a refresh from the API).
    LoadItems(Vec<String>),
    /// Append items (paginated load).
    AppendItems(Vec<String>),
    /// Append a single item — used by streaming subscribers.
    AppendItem(String),
    /// Enter search mode (`/`).
    EnterSearch,
    /// Append a char to the filter while in search mode.
    SearchChar(char),
    /// Backspace one char from the filter.
    SearchBackspace,
    /// Commit search and exit search mode (`Enter`).
    CommitSearch,
    /// Cancel search, restore the prior filter (`Esc`).
    CancelSearch,
    /// Enter command mode (`:`).
    EnterCommand,
    /// Append a char to the command buffer.
    CommandChar(char),
    /// Backspace one char from the command buffer.
    CommandBackspace,
    /// Run the command and exit command mode.
    CommitCommand,
    /// Cancel command mode.
    CancelCommand,
    /// Toggle log follow.
    ToggleFollow,
    /// Toggle help overlay.
    ToggleHelp,
    /// Switch tenant.
    SwitchTenant(Option<String>),
    /// Set a transient status line.
    SetStatus(Option<String>),
    /// Quit the app.
    Quit,
}

pub fn reduce(state: AppState, action: Action) -> AppState {
    let mut s = state;
    match action {
        Action::Move(delta) => {
            if s.items.is_empty() {
                s.selected = 0;
                return s;
            }
            let len = s.items.len() as i32;
            let new = (s.selected as i32) + delta;
            // Clamp without wrapping; wrap-around behaviour belongs in a
            // higher-level keybinding if we ever want it.
            s.selected = new.clamp(0, len - 1) as usize;
        }
        Action::JumpTo(i) => {
            if !s.items.is_empty() && i < s.items.len() {
                s.selected = i;
            }
        }
        Action::SwitchView(v) => {
            s.view = v;
            s.selected = 0;
            s.items.clear();
            s.filter.clear();
            s.follow = false;
        }
        Action::LoadItems(items) => {
            s.items = items;
            // Keep selection in range.
            if s.selected >= s.items.len() {
                s.selected = s.items.len().saturating_sub(1);
            }
        }
        Action::AppendItems(mut more) => {
            s.items.append(&mut more);
        }
        Action::AppendItem(item) => {
            s.items.push(item);
        }
        Action::EnterSearch => {
            s.search_mode = true;
            // Editing the live filter; preserve current value as starting point.
        }
        Action::SearchChar(c) => {
            if s.search_mode {
                s.filter.push(c);
            }
        }
        Action::SearchBackspace => {
            if s.search_mode {
                s.filter.pop();
            }
        }
        Action::CommitSearch => {
            s.search_mode = false;
        }
        Action::CancelSearch => {
            s.search_mode = false;
            s.filter.clear();
        }
        Action::EnterCommand => {
            s.command_mode = true;
            s.command_buffer.clear();
        }
        Action::CommandChar(c) => {
            if s.command_mode {
                s.command_buffer.push(c);
            }
        }
        Action::CommandBackspace => {
            if s.command_mode {
                s.command_buffer.pop();
            }
        }
        Action::CommitCommand => {
            s.command_mode = false;
            // Side effects (running the command) belong outside the reducer.
        }
        Action::CancelCommand => {
            s.command_mode = false;
            s.command_buffer.clear();
        }
        Action::ToggleFollow => {
            s.follow = !s.follow;
        }
        Action::ToggleHelp => {
            s.help_visible = !s.help_visible;
        }
        Action::SwitchTenant(t) => {
            s.tenant = t;
            s.items.clear();
            s.selected = 0;
        }
        Action::SetStatus(m) => {
            s.status_message = m;
        }
        Action::Quit => {
            s.quit = true;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st() -> AppState {
        AppState::default()
    }

    fn with_items(n: usize) -> AppState {
        let mut s = st();
        s.items = (0..n).map(|i| format!("item-{}", i)).collect();
        s
    }

    #[test]
    fn default_starts_on_pods() {
        assert_eq!(st().view, View::Pods);
    }

    #[test]
    fn default_no_quit() {
        assert!(!st().quit);
    }

    #[test]
    fn quit_sets_flag() {
        let s = reduce(st(), Action::Quit);
        assert!(s.quit);
    }

    #[test]
    fn move_down_increments() {
        let s = reduce(with_items(5), Action::Move(1));
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn move_up_decrements() {
        let mut s = with_items(5);
        s.selected = 3;
        let s = reduce(s, Action::Move(-1));
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn move_clamps_at_top() {
        let s = reduce(with_items(5), Action::Move(-1));
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn move_clamps_at_bottom() {
        let mut s = with_items(3);
        s.selected = 2;
        let s = reduce(s, Action::Move(10));
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn move_on_empty_keeps_zero() {
        let s = reduce(st(), Action::Move(5));
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn jump_to_in_range() {
        let s = reduce(with_items(5), Action::JumpTo(3));
        assert_eq!(s.selected, 3);
    }

    #[test]
    fn jump_to_out_of_range_ignored() {
        let s = with_items(3);
        let s = reduce(s, Action::JumpTo(100));
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn jump_to_on_empty_ignored() {
        let s = reduce(st(), Action::JumpTo(2));
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn switch_view_resets_state() {
        let mut s = with_items(5);
        s.selected = 3;
        s.filter = "abc".into();
        s.follow = true;
        let s = reduce(s, Action::SwitchView(View::Events));
        assert_eq!(s.view, View::Events);
        assert_eq!(s.selected, 0);
        assert!(s.items.is_empty());
        assert_eq!(s.filter, "");
        assert!(!s.follow);
    }

    #[test]
    fn load_items_replaces_list() {
        let mut s = with_items(3);
        s.selected = 2;
        let s = reduce(s, Action::LoadItems(vec!["x".into()]));
        assert_eq!(s.items, vec!["x".to_string()]);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn load_items_keeps_in_range_selection() {
        let mut s = with_items(5);
        s.selected = 2;
        let s = reduce(s, Action::LoadItems(vec!["a".into(), "b".into(), "c".into()]));
        assert_eq!(s.selected, 2);
    }

    #[test]
    fn append_items_extends() {
        let s = with_items(2);
        let s = reduce(s, Action::AppendItems(vec!["x".into(), "y".into()]));
        assert_eq!(s.items.len(), 4);
    }

    #[test]
    fn append_single_item() {
        let s = reduce(with_items(1), Action::AppendItem("new".into()));
        assert_eq!(s.items.len(), 2);
        assert_eq!(s.items.last().unwrap(), "new");
    }

    #[test]
    fn search_mode_lifecycle() {
        let s = reduce(st(), Action::EnterSearch);
        assert!(s.search_mode);
        let s = reduce(s, Action::SearchChar('a'));
        assert_eq!(s.filter, "a");
        let s = reduce(s, Action::SearchChar('b'));
        assert_eq!(s.filter, "ab");
        let s = reduce(s, Action::SearchBackspace);
        assert_eq!(s.filter, "a");
        let s = reduce(s, Action::CommitSearch);
        assert!(!s.search_mode);
        assert_eq!(s.filter, "a");
    }

    #[test]
    fn search_cancel_clears_filter() {
        let mut s = st();
        s.filter = "old".into();
        let s = reduce(s, Action::EnterSearch);
        let s = reduce(s, Action::SearchChar('x'));
        let s = reduce(s, Action::CancelSearch);
        assert!(!s.search_mode);
        assert_eq!(s.filter, "");
    }

    #[test]
    fn search_char_outside_mode_ignored() {
        let s = reduce(st(), Action::SearchChar('x'));
        assert_eq!(s.filter, "");
    }

    #[test]
    fn command_mode_lifecycle() {
        let s = reduce(st(), Action::EnterCommand);
        assert!(s.command_mode);
        let s = reduce(s, Action::CommandChar('q'));
        let s = reduce(s, Action::CommandChar('u'));
        assert_eq!(s.command_buffer, "qu");
        let s = reduce(s, Action::CommandBackspace);
        assert_eq!(s.command_buffer, "q");
        let s = reduce(s, Action::CommitCommand);
        assert!(!s.command_mode);
        // CommitCommand keeps the buffer; the runner reads it.
        assert_eq!(s.command_buffer, "q");
    }

    #[test]
    fn command_cancel_clears_buffer() {
        let s = reduce(st(), Action::EnterCommand);
        let s = reduce(s, Action::CommandChar('x'));
        let s = reduce(s, Action::CancelCommand);
        assert!(!s.command_mode);
        assert_eq!(s.command_buffer, "");
    }

    #[test]
    fn toggle_follow() {
        let s = reduce(st(), Action::ToggleFollow);
        assert!(s.follow);
        let s = reduce(s, Action::ToggleFollow);
        assert!(!s.follow);
    }

    #[test]
    fn toggle_help() {
        let s = reduce(st(), Action::ToggleHelp);
        assert!(s.help_visible);
        let s = reduce(s, Action::ToggleHelp);
        assert!(!s.help_visible);
    }

    #[test]
    fn switch_tenant_clears_items() {
        let s = with_items(5);
        let s = reduce(s, Action::SwitchTenant(Some("acme".into())));
        assert_eq!(s.tenant.as_deref(), Some("acme"));
        assert!(s.items.is_empty());
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn switch_tenant_to_none() {
        let mut s = st();
        s.tenant = Some("acme".into());
        let s = reduce(s, Action::SwitchTenant(None));
        assert!(s.tenant.is_none());
    }

    #[test]
    fn set_status_msg() {
        let s = reduce(st(), Action::SetStatus(Some("hi".into())));
        assert_eq!(s.status_message.as_deref(), Some("hi"));
        let s = reduce(s, Action::SetStatus(None));
        assert!(s.status_message.is_none());
    }

    #[test]
    fn view_name_round_trip() {
        for v in [
            View::Pods,
            View::Deployments,
            View::Services,
            View::Events,
            View::Logs,
            View::Topology,
            View::Tenants,
            View::Modules,
            View::Secrets,
            View::Flags,
        ] {
            assert!(!v.name().is_empty());
        }
    }

    #[test]
    fn move_selection_with_load_in_range() {
        let mut s = with_items(5);
        s.selected = 4;
        // Load fewer items; selection should clamp.
        let s = reduce(s, Action::LoadItems(vec!["a".into(), "b".into()]));
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn empty_load_resets_selection() {
        let mut s = with_items(5);
        s.selected = 3;
        let s = reduce(s, Action::LoadItems(vec![]));
        assert_eq!(s.selected, 0);
        assert!(s.items.is_empty());
    }

    #[test]
    fn deep_view_change_resets_filter() {
        let mut s = st();
        s.filter = "abc".into();
        let s = reduce(s, Action::SwitchView(View::Logs));
        assert_eq!(s.filter, "");
    }

    #[test]
    fn move_zero_is_noop() {
        let mut s = with_items(5);
        s.selected = 2;
        let s = reduce(s, Action::Move(0));
        assert_eq!(s.selected, 2);
    }
}
