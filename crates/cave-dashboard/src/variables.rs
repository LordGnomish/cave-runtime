// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Template variable interpolation engine.
//!
//! Supports Grafana's three syntaxes:
//!   `$var_name`, `${var_name}`, `[[var_name]]`
//! plus special built-ins: `$__interval`, `$__from`, `$__to`, `$__range`,
//! `$__user`, `$__org`, `$__dashboard`, `$__name`, `$__timeFilter`.

use crate::models::{Variable, VariableOption};
use chrono::Utc;
use regex::Regex;
use std::collections::HashMap;

/// Context passed to the interpolation engine.
pub struct InterpolationContext<'a> {
    pub variables: &'a [Variable],
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
    pub dashboard_title: Option<&'a str>,
    pub org_name: Option<&'a str>,
    pub user_login: Option<&'a str>,
}

impl<'a> InterpolationContext<'a> {
    pub fn new(variables: &'a [Variable]) -> Self {
        Self {
            variables,
            from: None,
            to: None,
            dashboard_title: None,
            org_name: None,
            user_login: None,
        }
    }
}

/// Build a map of variable name → current value string from a variable list.
fn build_var_map(variables: &[Variable]) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for var in variables {
        let val = option_value(&var.current);
        map.insert(var.name.clone(), val);
    }
    map
}

fn option_value(opt: &VariableOption) -> String {
    match &opt.value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .collect::<Vec<_>>()
            .join(","),
        other => other.to_string(),
    }
}

/// Compute a sensible `$__interval` from a time range string like "now-6h" / "now".
pub fn compute_interval(from: &str, to: &str) -> String {
    // Rough heuristic from the from→to labels
    let secs = parse_relative_secs(from, to);
    match secs {
        s if s <= 3600 => "10s".into(),            // ≤ 1h
        s if s <= 6 * 3600 => "30s".into(),        // ≤ 6h
        s if s < 24 * 3600 => "1m".into(),         // < 24h
        s if s < 7 * 86400 => "5m".into(),         // < 7d
        s if s < 30 * 86400 => "30m".into(),       // < 30d
        s if s <= 90 * 86400 => "1h".into(),       // ≤ 90d
        _ => "1d".into(),
    }
}

fn parse_relative_secs(from: &str, _to: &str) -> i64 {
    if from.starts_with("now-") {
        let rest = &from[4..];
        let (n, unit) = rest.split_at(rest.len().saturating_sub(1));
        let n: i64 = n.parse().unwrap_or(1);
        match unit {
            "s" => n,
            "m" => n * 60,
            "h" => n * 3600,
            "d" => n * 86400,
            "w" => n * 7 * 86400,
            "M" => n * 30 * 86400,
            "y" => n * 365 * 86400,
            _ => 3600,
        }
    } else {
        3600
    }
}

/// Interpolate all variable references in `input`.
pub fn interpolate(input: &str, ctx: &InterpolationContext<'_>) -> String {
    let mut var_map = build_var_map(ctx.variables);

    // Special built-ins
    let now_ms = Utc::now().timestamp_millis();
    let from = ctx.from.unwrap_or("now-6h");
    let to = ctx.to.unwrap_or("now");
    var_map.insert("__interval".into(), compute_interval(from, to));
    var_map.insert("__interval_ms".into(), "30000".into());
    var_map.insert("__from".into(), from.to_string());
    var_map.insert("__to".into(), to.to_string());
    var_map.insert("__range".into(), format!("{from}/{to}"));
    var_map.insert("__range_s".into(), parse_relative_secs(from, to).to_string());
    var_map.insert("__range_ms".into(), (parse_relative_secs(from, to) * 1000).to_string());
    var_map.insert("__dashboard".into(), ctx.dashboard_title.unwrap_or("").to_string());
    var_map.insert("__name".into(), ctx.dashboard_title.unwrap_or("").to_string());
    var_map.insert("__org".into(), ctx.org_name.unwrap_or("").to_string());
    var_map.insert("__user".into(), ctx.user_login.unwrap_or("").to_string());
    var_map.insert("__timeFilter".into(), format!("time >= {now_ms}ms - {from} AND time <= {now_ms}ms"));

    replace_all(input, &var_map)
}

/// Core substitution — handles all three syntaxes in a single pass.
fn replace_all(input: &str, vars: &HashMap<String, String>) -> String {
    // Order matters: longest match first.
    // We replace `${var}`, `[[var]]`, `$var` in that priority.
    let re_brace = Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}").unwrap();
    let re_legacy = Regex::new(r"\[\[([A-Za-z_][A-Za-z0-9_]*)\]\]").unwrap();
    let re_dollar = Regex::new(r"\$([A-Za-z_][A-Za-z0-9_]*)").unwrap();

    let s = re_brace.replace_all(input, |caps: &regex::Captures<'_>| {
        let name = &caps[1];
        vars.get(name).cloned().unwrap_or_else(|| caps[0].to_string())
    });
    let s = re_legacy.replace_all(&s, |caps: &regex::Captures<'_>| {
        let name = &caps[1];
        vars.get(name).cloned().unwrap_or_else(|| caps[0].to_string())
    });
    let s = re_dollar.replace_all(&s, |caps: &regex::Captures<'_>| {
        let name = &caps[1];
        vars.get(name).cloned().unwrap_or_else(|| caps[0].to_string())
    });
    s.into_owned()
}

/// Resolve ad-hoc filter variables into a label matcher string.
pub fn adhoc_filters_to_label_matchers(filters: &[AdhocFilter]) -> String {
    filters.iter()
        .map(|f| {
            let op = match f.operator.as_str() {
                "=" => "=",
                "!=" => "!=",
                "=~" => "=~",
                "!~" => "!~",
                other => other,
            };
            format!("{}{}\"{}\"", f.key, op, f.value)
        })
        .collect::<Vec<_>>()
        .join(",")
}

/// Represents a single ad-hoc filter value.
#[derive(Debug, Clone)]
pub struct AdhocFilter {
    pub key: String,
    pub operator: String,
    pub value: String,
}

/// Parse options string for custom variables ("a,b,c" or "value : label, ...").
pub fn parse_custom_options(options_str: &str) -> Vec<VariableOption> {
    options_str
        .split(',')
        .map(|part| {
            let part = part.trim();
            if let Some((value, label)) = part.split_once(':') {
                VariableOption {
                    value: serde_json::Value::String(value.trim().to_string()),
                    text: serde_json::Value::String(label.trim().to_string()),
                    selected: false,
                }
            } else {
                VariableOption {
                    value: serde_json::Value::String(part.to_string()),
                    text: serde_json::Value::String(part.to_string()),
                    selected: false,
                }
            }
        })
        .collect()
}

/// Parse interval options string (e.g. "1m,5m,10m,30m,1h,6h,12h,1d").
pub fn parse_interval_options(options_str: &str) -> Vec<VariableOption> {
    parse_custom_options(options_str)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Variable, VariableHide, VariableOption, VariableRefresh, VariableSort, VariableType};

    fn make_var(name: &str, value: &str) -> Variable {
        Variable {
            name: name.to_string(),
            label: name.to_string(),
            var_type: VariableType::Custom,
            description: String::new(),
            hide: VariableHide::DontHide,
            refresh: VariableRefresh::Never,
            sort: VariableSort::Disabled,
            query: serde_json::Value::String(String::new()),
            datasource: None,
            options: vec![],
            current: VariableOption {
                value: serde_json::Value::String(value.to_string()),
                text: serde_json::Value::String(value.to_string()),
                selected: true,
            },
            multi: false,
            include_all: false,
            all_value: None,
            regex: String::new(),
            values_text: String::new(),
            skip_url_sync: false,
        }
    }

    #[test]
    fn test_simple_dollar_var() {
        let vars = vec![make_var("env", "production")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("namespace=$env", &ctx), "namespace=production");
    }

    #[test]
    fn test_brace_syntax() {
        let vars = vec![make_var("env", "staging")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("${env}-cluster", &ctx), "staging-cluster");
    }

    #[test]
    fn test_legacy_bracket_syntax() {
        let vars = vec![make_var("region", "eu-west-1")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("region=[[region]]", &ctx), "region=eu-west-1");
    }

    #[test]
    fn test_multiple_vars() {
        let vars = vec![make_var("ns", "default"), make_var("pod", "web-1")];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("namespace=$ns,pod=$pod", &ctx), "namespace=default,pod=web-1");
    }

    #[test]
    fn test_unknown_var_left_unchanged() {
        let vars = vec![];
        let ctx = InterpolationContext::new(&vars);
        assert_eq!(interpolate("$unknown", &ctx), "$unknown");
    }

    #[test]
    fn test_builtin_interval() {
        let vars = vec![];
        let mut ctx = InterpolationContext::new(&vars);
        ctx.from = Some("now-1h");
        ctx.to = Some("now");
        let result = interpolate("step=$__interval", &ctx);
        assert!(result.starts_with("step="), "got: {result}");
    }

    #[test]
    fn test_compute_interval() {
        assert_eq!(compute_interval("now-30m", "now"), "10s");
        assert_eq!(compute_interval("now-3h", "now"), "30s");
        assert_eq!(compute_interval("now-12h", "now"), "1m");
        assert_eq!(compute_interval("now-7d", "now"), "30m");
    }

    #[test]
    fn test_custom_options_parsing() {
        let opts = parse_custom_options("prod,staging,dev");
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0].value, serde_json::Value::String("prod".into()));
    }

    #[test]
    fn test_custom_options_with_labels() {
        let opts = parse_custom_options("1 : One, 2 : Two");
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].value, serde_json::Value::String("1".into()));
        assert_eq!(opts[0].text, serde_json::Value::String("One".into()));
    }
}
