# Prometheus alerting rules

YAML files in this directory describe Prometheus-compatible alerting
rules per CAVE crate. Each file contains exactly eight alerts covering
the standard SLO surface:

1. SLO burn rate — fast (1h)
2. SLO burn rate — slow (6h)
3. Error budget low
4. Saturation high
5. Cardinality explosion
6. Crate-specific saturation #1
7. Crate-specific saturation #2
8. Health probe failing

## File format

```yaml
crate: cave-<name>
group: cave-<name>-slo

alerts:
  - alert: <PascalCaseName>
    expr:  '<PromQL expression>'
    for:   5m
    severity: critical | warning | info
    labels:
      tenant_scoped: "true"
    annotations:
      summary:     "Short headline"
      description: "Multi-line context with {{ $value }} interpolation"
      runbook_url: "https://docs.cave.dev/runbooks/<crate>/<alert-id>.md"
```

## Loading

Rules are loaded via `cave-alerts::store::AlertStore` (extended with a YAML
loader in a follow-up). Each rule's tenant scope is derived from the
`tenant_id` label in the matched series; alerts produced by these rules
flow through the same routing → inhibit → silence → grouping pipeline as
externally-pushed alerts.
