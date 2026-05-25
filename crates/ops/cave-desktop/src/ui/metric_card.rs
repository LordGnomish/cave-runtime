// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
/// A single big-number metric tile with optional delta and unit.
///
/// Same data contract as `.stat-card` in the web portal — value rendered
/// large, unit small, delta in green/red. Backend feeds JSON.
pub struct MetricCard {
    pub label: String,
    pub value: String,
    pub unit: Option<String>,
    pub delta_pct: Option<f64>,
}

impl MetricCard {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
            unit: None,
            delta_pct: None,
        }
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    pub fn with_delta(mut self, pct: f64) -> Self {
        self.delta_pct = Some(pct);
        self
    }

    // TODO(adr-portal-desktop-001): GPUI render impl behind `gpui-runtime`.
}
