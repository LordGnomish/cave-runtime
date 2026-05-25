// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
// Source: SARIF v2.1.0 OASIS standard

//! SARIF v2.1.0 serializer.

use super::{Finding, Report, Severity};
use serde_json::{Value, json};

pub fn to_sarif(report: &Report) -> Value {
    let rules: Vec<Value> = report
        .findings
        .iter()
        .map(|f| {
            json!({
                "id": f.id,
                "name": f.title,
                "shortDescription": { "text": f.title },
                "fullDescription": { "text": f.message },
                "defaultConfiguration": { "level": sarif_level(f.severity) },
            })
        })
        .collect();

    let results: Vec<Value> = report
        .findings
        .iter()
        .map(|f| {
            json!({
                "ruleId": f.id,
                "level": sarif_level(f.severity),
                "message": { "text": f.message },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": f.location }
                    }
                }],
            })
        })
        .collect();

    json!({
        "$schema": "https://raw.githubusercontent.com/oasis-tcs/sarif-spec/master/Schemata/sarif-schema-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "cave-scan",
                    "informationUri": "https://github.com/cave-runtime/cave-runtime",
                    "rules": rules
                }
            },
            "results": results
        }]
    })
}

fn sarif_level(s: Severity) -> &'static str {
    match s {
        Severity::Critical | Severity::High => "error",
        Severity::Medium => "warning",
        Severity::Low => "note",
        Severity::Info => "none",
    }
}

pub fn to_string_pretty(report: &Report) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(&to_sarif(report))
}
