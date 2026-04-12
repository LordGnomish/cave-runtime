//! Variable interpolation for CAVE Dashboard.
//!
//! Supports:
//! - `$var_name` — simple variable substitution
//! - `${var_name}` — brace-delimited substitution
//! - `[[var_name]]` — legacy Grafana syntax
//! - Special variables: `$__interval`, `$__from`, `$__to`, `$__range`, `$__user`

use regex::Regex;

use crate::models::{TimeRange, Variable};

/// Interpolate all variables in `query`.
///
/// Variables are looked up by name; unresolved tokens are left as-is.
pub fn interpolate(query: &str, vars: &[Variable], time: Option<&TimeRange>) -> String {
    let mut result = query.to_string();

    // Built-in / special variables.
    let interval = compute_interval(time);
    let (from, to) = time
        .map(|t| (t.from.as_str(), t.to.as_str()))
        .unwrap_or(("now-6h", "now"));

    let specials: &[(&str, &str)] = &[
        ("__interval", &interval),
        ("__rate_interval", &interval),
        ("__from", from),
        ("__to", to),
        ("__range", "6h"),
        ("__user", "anonymous"),
        ("__org", "1"),
        ("__dashboard", ""),
    ];

    for (name, value) in specials {
        result = replace_var(&result, name, value);
    }

    // User-defined variables.
    for var in vars {
        if let Some(current) = var.current_value() {
            result = replace_var(&result, &var.name, current);
        }
    }

    result
}

/// Replace `$name`, `${name}`, and `[[name]]` occurrences.
fn replace_var(input: &str, name: &str, value: &str) -> String {
    // ${name}
    let brace = format!("${{{name}}}");
    let mut out = input.replace(&brace, value);

    // [[name]] (legacy)
    let legacy = format!("[[{name}]]");
    out = out.replace(&legacy, value);

    // $name — only when followed by a non-alphanumeric / non-underscore char
    // Use regex so we don't match $name_longer.
    let pattern = format!(r"\$(?P<n>{name})\b");
    if let Ok(re) = Regex::new(&pattern) {
        out = re.replace_all(&out, value).into_owned();
    }

    out
}

/// Derive a sensible `$__interval` from the time range string.
fn compute_interval(time: Option<&TimeRange>) -> String {
    let from = time.map(|t| t.from.as_str()).unwrap_or("now-6h");
    // Very simplified mapping — a real implementation would do duration math.
    if from.contains("30d") || from.contains("90d") {
        "1d".to_string()
    } else if from.contains("7d") {
        "1h".to_string()
    } else if from.contains("24h") || from.contains("1d") {
        "30m".to_string()
    } else if from.contains("6h") {
        "5m".to_string()
    } else if from.contains("1h") {
        "30s".to_string()
    } else {
        "1m".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{VariableHide, VariableOption, VariableRefresh, VariableType};

    fn make_var(name: &str, value: &str) -> Variable {
        Variable {
            name: name.to_string(),
            label: None,
            var_type: VariableType::Custom,
            query: None,
            options: vec![VariableOption {
                text: value.to_string(),
                value: value.to_string(),
                selected: true,
            }],
            current: Some(VariableOption {
                text: value.to_string(),
                value: value.to_string(),
                selected: true,
            }),
            multi: false,
            include_all: false,
            refresh: VariableRefresh::Never,
            hide: VariableHide::DontHide,
            description: None,
        }
    }

    #[test]
    fn test_simple_dollar_substitution() {
        let vars = vec![make_var("env", "production")];
        let out = interpolate("SELECT * FROM logs WHERE env = '$env'", &vars, None);
        assert_eq!(out, "SELECT * FROM logs WHERE env = 'production'");
    }

    #[test]
    fn test_brace_substitution() {
        let vars = vec![make_var("cluster", "us-east-1")];
        let out = interpolate("cluster=${cluster}", &vars, None);
        assert_eq!(out, "cluster=us-east-1");
    }

    #[test]
    fn test_multiple_variables() {
        let vars = vec![make_var("ns", "default"), make_var("pod", "web-1")];
        let out = interpolate("namespace=$ns pod=$pod", &vars, None);
        assert_eq!(out, "namespace=default pod=web-1");
    }

    #[test]
    fn test_unknown_variable_kept() {
        let vars: Vec<Variable> = vec![];
        let out = interpolate("rate($unknown[5m])", &vars, None);
        assert!(out.contains("$unknown"), "unknown variable should remain");
    }

    #[test]
    fn test_interval_special() {
        let time = TimeRange { from: "now-6h".to_string(), to: "now".to_string() };
        let out = interpolate("rate(http_requests[$__interval])", &[], Some(&time));
        assert!(out.contains("5m"), "6h range should yield 5m interval");
    }

    #[test]
    fn test_from_to_special() {
        let time =
            TimeRange { from: "2024-01-01T00:00:00Z".to_string(), to: "2024-01-02T00:00:00Z".to_string() };
        let out = interpolate("from=$__from to=$__to", &[], Some(&time));
        assert!(out.contains("2024-01-01T00:00:00Z"));
        assert!(out.contains("2024-01-02T00:00:00Z"));
    }

    #[test]
    fn test_legacy_bracket_syntax() {
        let vars = vec![make_var("region", "eu-west-1")];
        let out = interpolate("region=[[region]]", &vars, None);
        assert_eq!(out, "region=eu-west-1");
    }
}
