// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// A generic row-oriented table primitive.
///
/// Cluster Overview uses this for the pod/node lists; later screens (logs,
/// events, audit) will reuse it. Sorting and filtering are caller-driven —
/// the primitive only renders what it's given.
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

impl Table {
    pub fn new(columns: Vec<String>) -> Self {
        Self { columns, rows: Vec::new() }
    }

    pub fn push_row(&mut self, row: Vec<String>) {
        self.rows.push(row);
    }

    pub fn row_count(&self) -> usize {
        self.rows.len()
    }

    // TODO(adr-portal-desktop-001): GPUI render impl behind `gpui-runtime`.
    //   - Virtualized scrolling once row count is meaningful
    //   - Column-width auto-fit
    //   - Sticky header
}
