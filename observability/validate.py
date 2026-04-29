#!/usr/bin/env python3
"""
Validator for the observability catalog.

Run after generate.py. Asserts:
  - every JSON dashboard is valid JSON, has 10 panels, unique panel IDs,
    a tenant-cardinality panel and a health-probe panel.
  - every YAML alert file parses, has exactly one group with 8 rules,
    every rule has a runbook_url under https://docs.cave.dev/runbooks/,
    every rule has a severity label, and the standard alert names are
    present.

Exits non-zero on any failure with a per-file diagnostic.
"""

import json
import sys
import yaml
from pathlib import Path

ROOT = Path(__file__).resolve().parent
DASH_DIR = ROOT / "dashboards"
ALERT_DIR = ROOT / "alerts"

REQUIRED_ALERTS_SUFFIXES = (
    "SloBurnRateFast",
    "SloBurnRateSlow",
    "ErrorBudgetLow",
    "LatencyP99High",
    "SaturationHigh",
    "MemoryPressure",
    "CardinalityExplosion",
    "HealthProbeFailing",
)


def validate_dashboards():
    failures = []
    for path in sorted(DASH_DIR.glob("*.json")):
        try:
            with open(path) as f:
                d = json.load(f)
        except json.JSONDecodeError as e:
            failures.append(f"{path.name}: bad JSON: {e}")
            continue

        panels = d.get("panels", [])
        if len(panels) != 10:
            failures.append(f"{path.name}: expected 10 panels, got {len(panels)}")

        ids = [p.get("id") for p in panels]
        if len(set(ids)) != len(ids):
            failures.append(f"{path.name}: duplicate panel ids")

        titles = " ".join(p.get("title", "").lower() for p in panels)
        if "tenant" not in titles:
            failures.append(f"{path.name}: no tenant-cardinality panel")
        if "health" not in titles and "up" not in " ".join(
            (p.get("targets", [{}])[0].get("expr", "")) for p in panels
        ):
            failures.append(f"{path.name}: no health-probe panel")

        if d.get("uid") != path.stem:
            failures.append(f"{path.name}: uid mismatch ({d.get('uid')} vs {path.stem})")

    return failures


def validate_alerts():
    failures = []
    for path in sorted(ALERT_DIR.glob("*.yml")):
        try:
            with open(path) as f:
                r = yaml.safe_load(f)
        except yaml.YAMLError as e:
            failures.append(f"{path.name}: bad YAML: {e}")
            continue

        groups = r.get("groups", [])
        if len(groups) != 1:
            failures.append(f"{path.name}: expected 1 group, got {len(groups)}")
            continue
        rules = groups[0].get("rules", [])
        if len(rules) != 8:
            failures.append(f"{path.name}: expected 8 rules, got {len(rules)}")
            continue

        for rule in rules:
            name = rule.get("alert", "")
            if not name:
                failures.append(f"{path.name}: rule missing 'alert' field")
                continue
            if not any(name.endswith(s) for s in REQUIRED_ALERTS_SUFFIXES):
                failures.append(f"{path.name}: unknown alert '{name}'")
            ann = rule.get("annotations", {})
            url = ann.get("runbook_url", "")
            if not url.startswith("https://docs.cave.dev/runbooks/"):
                failures.append(f"{path.name}: rule '{name}' bad runbook_url '{url}'")
            sev = (rule.get("labels") or {}).get("severity", "")
            if sev not in ("critical", "warning", "info"):
                failures.append(f"{path.name}: rule '{name}' bad severity '{sev}'")

        # Module covers all 8 standard alert types
        names = " ".join(rule.get("alert", "") for rule in rules)
        for s in REQUIRED_ALERTS_SUFFIXES:
            if s not in names:
                failures.append(f"{path.name}: missing required alert *{s}")

    return failures


def main():
    failures = validate_dashboards() + validate_alerts()
    n_dash = len(list(DASH_DIR.glob("*.json")))
    n_alert = len(list(ALERT_DIR.glob("*.yml")))
    print(f"dashboards: {n_dash} files, panels: {n_dash * 10}")
    print(f"alerts:     {n_alert} files, rules:  {n_alert * 8}")
    if failures:
        print("\nFAILURES:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    print("\nAll observability catalog files valid.")


if __name__ == "__main__":
    main()
