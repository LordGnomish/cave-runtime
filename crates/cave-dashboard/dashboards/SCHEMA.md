# Dashboard panel schema

YAML files in this directory describe Grafana-compatible dashboards for the
CAVE runtime. Each file targets one crate and exposes ten panels covering
the standard observability triad — RED (rate/error/duration), USE
(utilization/saturation/errors) and per-resource breakdowns.

## File structure

```yaml
crate: cave-<name>
title:  "Human-readable title"
uid:    "cave-<name>"             # Grafana dashboard UID
tags:   [observability, cave, ...]

variables:
  - name:   tenant
    type:   query
    query:  'label_values(...)'
    default: anonymous
  ... etc

panels:
  - id:    1
    title: "Request rate"
    type:  timeseries           # timeseries | stat | gauge | heatmap | table
    unit:  reqps                # Grafana unit string
    query: 'sum(rate(http_requests_total{job="cave-<name>"}[5m])) by (route)'
    thresholds:
      - value: 0
        color: green
      - value: 100
        color: yellow
      - value: 500
        color: red
    legend: '{{route}}'
    description: "..."
  ...
```

The `panels` list is required to contain ten entries:
1. Request rate (RED)
2. Error rate (RED)
3. Latency p50 / p95 / p99 (RED)
4. Saturation (USE)
5. Utilization (USE)
6. Errors / 5xx (USE)
7. Per-resource hot list (top-N table)
8. Inflight / queue depth
9. Tenant breakdown (multi-tenant cardinality)
10. Resource saturation map (heatmap or stat panel)

## Loading

Dashboards are loaded by the runtime through `cave-dashboard::provisioning`,
which converts each YAML into Grafana JSON via the schema in `models.rs`.
Tests live in `tests/dashboard_load.rs` and assert that every YAML in this
directory parses and contains the required ten panels.
