// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! k9s-style keybindings for the TUI.
//!
//! Maps a `Key` (parsed from `crossterm::event::KeyEvent` at the I/O
//! boundary) to a `KeyAction` that the reducer understands. Keeping
//! this layer pure means we can test "what does `j` do in normal
//! mode?" without touching the terminal.

use crate::tui::app::View;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Up,
    Down,
    PageUp,
    PageDown,
    Home,
    End,
    /// Any function key.
    F(u8),
    Tab,
    ShiftTab,
    Ctrl(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeyAction {
    /// No-op (key was ignored at this layer).
    Noop,
    Move(i32),
    JumpTop,
    JumpBottom,
    Switch(View),
    EnterSearch,
    Search(SearchAction),
    EnterCommand,
    Command(CommandAction),
    ToggleFollow,
    ToggleHelp,
    Quit,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchAction {
    Char(char),
    Backspace,
    Commit,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandAction {
    Char(char),
    Backspace,
    Commit,
    Cancel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Search,
    Command,
}

/// Resolve a key in the given mode.
pub fn default_keymap(mode: Mode, key: Key) -> KeyAction {
    match mode {
        Mode::Normal => normal(key),
        Mode::Search => search(key),
        Mode::Command => command(key),
    }
}

fn normal(key: Key) -> KeyAction {
    match key {
        Key::Char('q') | Key::Ctrl('c') => KeyAction::Quit,
        Key::Char('j') | Key::Down => KeyAction::Move(1),
        Key::Char('k') | Key::Up => KeyAction::Move(-1),
        Key::PageDown | Key::Ctrl('d') => KeyAction::Move(10),
        Key::PageUp | Key::Ctrl('u') => KeyAction::Move(-10),
        Key::Char('g') | Key::Home => KeyAction::JumpTop,
        Key::Char('G') | Key::End => KeyAction::JumpBottom,
        Key::Char('/') => KeyAction::EnterSearch,
        Key::Char(':') => KeyAction::EnterCommand,
        Key::Char('?') | Key::F(1) => KeyAction::ToggleHelp,
        Key::Char('f') => KeyAction::ToggleFollow,
        // Resource shortcuts
        Key::Char('1') => KeyAction::Switch(View::Pods),
        Key::Char('2') => KeyAction::Switch(View::Deployments),
        Key::Char('3') => KeyAction::Switch(View::Services),
        Key::Char('4') => KeyAction::Switch(View::Events),
        Key::Char('5') => KeyAction::Switch(View::Logs),
        Key::Char('6') => KeyAction::Switch(View::Topology),
        Key::Char('7') => KeyAction::Switch(View::Tenants),
        Key::Char('8') => KeyAction::Switch(View::Modules),
        Key::Char('9') => KeyAction::Switch(View::Secrets),
        Key::Char('0') => KeyAction::Switch(View::Flags),
        _ => KeyAction::Noop,
    }
}

fn search(key: Key) -> KeyAction {
    match key {
        Key::Esc => KeyAction::Search(SearchAction::Cancel),
        Key::Enter => KeyAction::Search(SearchAction::Commit),
        Key::Backspace => KeyAction::Search(SearchAction::Backspace),
        Key::Char(c) => KeyAction::Search(SearchAction::Char(c)),
        _ => KeyAction::Noop,
    }
}

fn command(key: Key) -> KeyAction {
    match key {
        Key::Esc => KeyAction::Command(CommandAction::Cancel),
        Key::Enter => KeyAction::Command(CommandAction::Commit),
        Key::Backspace => KeyAction::Command(CommandAction::Backspace),
        Key::Char(c) => KeyAction::Command(CommandAction::Char(c)),
        _ => KeyAction::Noop,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn j_moves_down() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('j')),
            KeyAction::Move(1)
        );
    }

    #[test]
    fn k_moves_up() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('k')),
            KeyAction::Move(-1)
        );
    }

    #[test]
    fn arrow_down_moves_down() {
        assert_eq!(default_keymap(Mode::Normal, Key::Down), KeyAction::Move(1));
    }

    #[test]
    fn arrow_up_moves_up() {
        assert_eq!(default_keymap(Mode::Normal, Key::Up), KeyAction::Move(-1));
    }

    #[test]
    fn pgdn_moves_ten() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::PageDown),
            KeyAction::Move(10)
        );
        assert_eq!(
            default_keymap(Mode::Normal, Key::Ctrl('d')),
            KeyAction::Move(10)
        );
    }

    #[test]
    fn pgup_moves_ten_up() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::PageUp),
            KeyAction::Move(-10)
        );
    }

    #[test]
    fn g_jumps_top() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('g')),
            KeyAction::JumpTop
        );
    }

    #[test]
    fn capital_g_jumps_bottom() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('G')),
            KeyAction::JumpBottom
        );
    }

    #[test]
    fn q_quits() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('q')),
            KeyAction::Quit
        );
    }

    #[test]
    fn ctrl_c_quits() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Ctrl('c')),
            KeyAction::Quit
        );
    }

    #[test]
    fn slash_enters_search() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('/')),
            KeyAction::EnterSearch
        );
    }

    #[test]
    fn colon_enters_command() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char(':')),
            KeyAction::EnterCommand
        );
    }

    #[test]
    fn question_toggles_help() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('?')),
            KeyAction::ToggleHelp
        );
    }

    #[test]
    fn f1_toggles_help() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::F(1)),
            KeyAction::ToggleHelp
        );
    }

    #[test]
    fn f_toggles_follow() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('f')),
            KeyAction::ToggleFollow
        );
    }

    #[test]
    fn digit_shortcuts_switch_view() {
        let pairs = [
            ('1', View::Pods),
            ('2', View::Deployments),
            ('3', View::Services),
            ('4', View::Events),
            ('5', View::Logs),
            ('6', View::Topology),
            ('7', View::Tenants),
            ('8', View::Modules),
            ('9', View::Secrets),
            ('0', View::Flags),
        ];
        for (k, v) in pairs {
            assert_eq!(
                default_keymap(Mode::Normal, Key::Char(k)),
                KeyAction::Switch(v),
                "digit {} should switch to {:?}",
                k,
                v
            );
        }
    }

    #[test]
    fn unknown_key_is_noop() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::Char('x')),
            KeyAction::Noop
        );
    }

    #[test]
    fn search_char_in_search_mode() {
        assert_eq!(
            default_keymap(Mode::Search, Key::Char('a')),
            KeyAction::Search(SearchAction::Char('a'))
        );
    }

    #[test]
    fn search_backspace() {
        assert_eq!(
            default_keymap(Mode::Search, Key::Backspace),
            KeyAction::Search(SearchAction::Backspace)
        );
    }

    #[test]
    fn search_enter_commits() {
        assert_eq!(
            default_keymap(Mode::Search, Key::Enter),
            KeyAction::Search(SearchAction::Commit)
        );
    }

    #[test]
    fn search_esc_cancels() {
        assert_eq!(
            default_keymap(Mode::Search, Key::Esc),
            KeyAction::Search(SearchAction::Cancel)
        );
    }

    #[test]
    fn search_arrow_is_noop() {
        assert_eq!(default_keymap(Mode::Search, Key::Up), KeyAction::Noop);
    }

    #[test]
    fn command_char() {
        assert_eq!(
            default_keymap(Mode::Command, Key::Char('p')),
            KeyAction::Command(CommandAction::Char('p'))
        );
    }

    #[test]
    fn command_commit() {
        assert_eq!(
            default_keymap(Mode::Command, Key::Enter),
            KeyAction::Command(CommandAction::Commit)
        );
    }

    #[test]
    fn command_cancel() {
        assert_eq!(
            default_keymap(Mode::Command, Key::Esc),
            KeyAction::Command(CommandAction::Cancel)
        );
    }

    #[test]
    fn command_backspace() {
        assert_eq!(
            default_keymap(Mode::Command, Key::Backspace),
            KeyAction::Command(CommandAction::Backspace)
        );
    }

    #[test]
    fn ctrl_d_pages_down_only_in_normal() {
        // In Search mode, Ctrl(d) is not a search-relevant key, so noop.
        assert_eq!(
            default_keymap(Mode::Search, Key::Ctrl('d')),
            KeyAction::Noop
        );
    }

    #[test]
    fn home_jumps_top() {
        assert_eq!(default_keymap(Mode::Normal, Key::Home), KeyAction::JumpTop);
    }

    #[test]
    fn end_jumps_bottom() {
        assert_eq!(
            default_keymap(Mode::Normal, Key::End),
            KeyAction::JumpBottom
        );
    }
}
