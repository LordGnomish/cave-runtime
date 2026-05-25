// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Observability — 8 Prometheus panels + 5 alert rule definitions for the
//! cave-portal-api dashboard renderer.

use prometheus_client::metrics::counter::Counter;
use prometheus_client::metrics::gauge::Gauge;
use prometheus_client::registry::Registry;
use std::sync::atomic::AtomicI64;
use std::sync::Arc;

#[derive(Debug, Clone, Copy)]
pub enum PanelKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Debug, Clone)]
pub struct Panel {
    pub name: &'static str,
    pub kind: PanelKind,
    pub help: &'static str,
}

#[derive(Debug, Clone)]
pub struct AlertRule {
    pub name: &'static str,
    pub expr: &'static str,
    pub severity: &'static str,
}

/// The eight Prometheus panels exposed by cave-crossplane.
pub const PANELS: [Panel; 8] = [
    Panel {
        name: "cave_crossplane_xrds_total",
        kind: PanelKind::Gauge,
        help: "Number of XRDs registered",
    },
    Panel {
        name: "cave_crossplane_compositions_total",
        kind: PanelKind::Gauge,
        help: "Number of compositions registered",
    },
    Panel {
        name: "cave_crossplane_providers_total",
        kind: PanelKind::Gauge,
        help: "Number of providers installed",
    },
    Panel {
        name: "cave_crossplane_functions_total",
        kind: PanelKind::Gauge,
        help: "Number of functions installed",
    },
    Panel {
        name: "cave_crossplane_claims_total",
        kind: PanelKind::Gauge,
        help: "Total claims tracked",
    },
    Panel {
        name: "cave_crossplane_composite_resources_total",
        kind: PanelKind::Gauge,
        help: "Total composite resources tracked",
    },
    Panel {
        name: "cave_crossplane_reconcile_queue_length",
        kind: PanelKind::Gauge,
        help: "Pending reconcile-queue length",
    },
    Panel {
        name: "cave_crossplane_reconcile_failures_total",
        kind: PanelKind::Counter,
        help: "Cumulative reconcile failure count",
    },
];

/// The five alert rules.
pub const ALERTS: [AlertRule; 5] = [
    AlertRule {
        name: "CrossplaneReconcileBacklogHigh",
        expr: "cave_crossplane_reconcile_queue_length > 100",
        severity: "warning",
    },
    AlertRule {
        name: "CrossplaneReconcileFailuresSpiking",
        expr: "increase(cave_crossplane_reconcile_failures_total[5m]) > 10",
        severity: "critical",
    },
    AlertRule {
        name: "CrossplaneNoXRDs",
        expr: "cave_crossplane_xrds_total == 0",
        severity: "info",
    },
    AlertRule {
        name: "CrossplaneNoProviders",
        expr: "cave_crossplane_providers_total == 0",
        severity: "warning",
    },
    AlertRule {
        name: "CrossplaneFunctionUnhealthy",
        expr: "cave_crossplane_functions_total - cave_crossplane_functions_healthy > 0",
        severity: "warning",
    },
];

pub struct CrossplaneMetrics {
    pub registry: Registry,
    pub xrds: Gauge<i64, AtomicI64>,
    pub compositions: Gauge<i64, AtomicI64>,
    pub providers: Gauge<i64, AtomicI64>,
    pub functions: Gauge<i64, AtomicI64>,
    pub reconcile_failures: Counter,
}

impl CrossplaneMetrics {
    pub fn new() -> Self {
        let mut registry = Registry::default();
        let xrds = Gauge::<i64, AtomicI64>::default();
        let compositions = Gauge::<i64, AtomicI64>::default();
        let providers = Gauge::<i64, AtomicI64>::default();
        let functions = Gauge::<i64, AtomicI64>::default();
        let reconcile_failures = Counter::default();
        registry.register("cave_crossplane_xrds_total", "XRDs", xrds.clone());
        registry.register(
            "cave_crossplane_compositions_total",
            "Compositions",
            compositions.clone(),
        );
        registry.register(
            "cave_crossplane_providers_total",
            "Providers installed",
            providers.clone(),
        );
        registry.register(
            "cave_crossplane_functions_total",
            "Functions installed",
            functions.clone(),
        );
        registry.register(
            "cave_crossplane_reconcile_failures_total",
            "Reconcile failures",
            reconcile_failures.clone(),
        );
        Self {
            registry,
            xrds,
            compositions,
            providers,
            functions,
            reconcile_failures,
        }
    }

    pub fn snapshot_from(&self, state: &Arc<crate::CrossplaneState>) {
        self.xrds.set(state.xrd_store.len() as i64);
        self.compositions.set(state.composition_store.len() as i64);
        self.providers.set(state.provider_store.list().len() as i64);
        self.functions.set(state.function_store.len() as i64);
    }
}

impl Default for CrossplaneMetrics {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_have_eight() {
        assert_eq!(PANELS.len(), 8);
    }

    #[test]
    fn alerts_have_five() {
        assert_eq!(ALERTS.len(), 5);
    }

    #[test]
    fn panels_names_unique() {
        let names: std::collections::BTreeSet<&str> = PANELS.iter().map(|p| p.name).collect();
        assert_eq!(names.len(), PANELS.len());
    }

    #[test]
    fn alert_names_unique() {
        let names: std::collections::BTreeSet<&str> = ALERTS.iter().map(|a| a.name).collect();
        assert_eq!(names.len(), ALERTS.len());
    }

    #[test]
    fn metrics_register_no_panic() {
        let _m = CrossplaneMetrics::new();
    }

    #[test]
    fn snapshot_updates_gauges() {
        let m = CrossplaneMetrics::new();
        let s = Arc::new(crate::CrossplaneState::default());
        m.snapshot_from(&s);
        assert_eq!(m.xrds.get(), 0);
    }

    #[test]
    fn reconcile_failures_counter_increments() {
        let m = CrossplaneMetrics::new();
        m.reconcile_failures.inc();
        m.reconcile_failures.inc();
        assert_eq!(m.reconcile_failures.get(), 2);
    }
}
