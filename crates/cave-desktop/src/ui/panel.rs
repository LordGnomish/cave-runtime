// SPDX-License-Identifier: AGPL-3.0-or-later
/// A titled container with optional sidebar/footer slots.
///
/// Mirrors the `<div class="panel">` shape used in `portal_index.html` so
/// desktop and web stay visually coherent.
pub struct Panel {
    pub title: String,
    pub body: Vec<String>,
}

impl Panel {
    pub fn new(title: impl Into<String>) -> Self {
        Self { title: title.into(), body: Vec::new() }
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        self.body.push(line.into());
    }

    // TODO(adr-portal-desktop-001): GPUI render impl behind `gpui-runtime`.
    // Until the feature is wired, callers can read `title`/`body` directly.
}
