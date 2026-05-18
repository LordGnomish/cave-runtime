// SPDX-License-Identifier: AGPL-3.0-or-later
//! Grafana wrap — variable resolution + native SVG renderer.
//!
//! Layered atop [`super::grafana`]. The basic wrap holds dashboard
//! definitions; this module renders them. Variables (`$env`, `$service`)
//! are substituted in queries at render time, and panels are turned into
//! tiny inline SVG that the page template drops into the `<main>`.
//!
//! No JavaScript runtime, no Grafana iframe — fully native server-rendered.

use super::grafana::{DashboardDef, PanelDef, PanelKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Series {
    pub label: String,
    pub points: Vec<(f64, f64)>, // (x, y)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RendererError {
    #[error("missing variable: {0}")]
    MissingVariable(String),
    #[error("invalid variable name: {0:?}")]
    InvalidVarName(String),
    #[error("dimension overflow")]
    DimensionOverflow,
    #[error("panel kind unsupported by renderer: {0:?}")]
    Unsupported(PanelKind),
    #[error("query mismatch")]
    QueryMismatch,
}

/// Substitute `$var` references in a query string with values from `vars`.
/// Variable names must match `[A-Za-z_][A-Za-z0-9_]*`. Missing variables
/// return [`RendererError::MissingVariable`].
pub fn resolve_query(query: &str, vars: &HashMap<String, String>) -> Result<String, RendererError> {
    let mut out = String::with_capacity(query.len());
    let mut chars = query.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            out.push(ch);
            continue;
        }
        // Parse identifier
        let mut name = String::new();
        match chars.peek() {
            Some(c) if c.is_ascii_alphabetic() || *c == '_' => {}
            _ => {
                // not a variable, emit '$' literally
                out.push('$');
                continue;
            }
        }
        while let Some(c) = chars.peek() {
            if c.is_ascii_alphanumeric() || *c == '_' {
                name.push(*c);
                chars.next();
            } else {
                break;
            }
        }
        if name.is_empty() {
            return Err(RendererError::InvalidVarName(name));
        }
        match vars.get(&name) {
            Some(v) => out.push_str(v),
            None => return Err(RendererError::MissingVariable(name)),
        }
    }
    Ok(out)
}

/// All variable names referenced by `$name` in `query`.
pub fn referenced_variables(query: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut chars = query.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '$' {
            continue;
        }
        let mut name = String::new();
        while let Some(c) = chars.peek() {
            if name.is_empty() {
                if c.is_ascii_alphabetic() || *c == '_' {
                    name.push(*c);
                    chars.next();
                } else {
                    break;
                }
            } else if c.is_ascii_alphanumeric() || *c == '_' {
                name.push(*c);
                chars.next();
            } else {
                break;
            }
        }
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// Render a series as a minimal inline SVG. Width/height are in user units;
/// the SVG includes a viewport, axes, polyline path, and a label.
pub fn render_timeseries_svg(panel: &PanelDef, series: &[Series], width: u32, height: u32) -> Result<String, RendererError> {
    if !matches!(panel.kind, PanelKind::Timeseries) {
        return Err(RendererError::Unsupported(panel.kind));
    }
    if width < 50 || width > 4000 || height < 30 || height > 4000 {
        return Err(RendererError::DimensionOverflow);
    }
    let title = escape_xml(&panel.title);
    let unit = escape_xml(&panel.unit);
    let mut paths = String::new();

    let (mut min_x, mut max_x, mut min_y, mut max_y) =
        (f64::INFINITY, f64::NEG_INFINITY, f64::INFINITY, f64::NEG_INFINITY);
    for s in series {
        for (x, y) in &s.points {
            if *x < min_x { min_x = *x; }
            if *x > max_x { max_x = *x; }
            if *y < min_y { min_y = *y; }
            if *y > max_y { max_y = *y; }
        }
    }
    if min_x.is_infinite() {
        // empty
        min_x = 0.0; max_x = 1.0; min_y = 0.0; max_y = 1.0;
    }
    if (max_x - min_x).abs() < f64::EPSILON {
        max_x = min_x + 1.0;
    }
    if (max_y - min_y).abs() < f64::EPSILON {
        max_y = min_y + 1.0;
    }
    let margin = 20.0;
    let plot_w = width as f64 - 2.0 * margin;
    let plot_h = height as f64 - 2.0 * margin;
    for (idx, s) in series.iter().enumerate() {
        let mut d = String::new();
        for (i, (x, y)) in s.points.iter().enumerate() {
            let px = margin + (x - min_x) / (max_x - min_x) * plot_w;
            let py = margin + plot_h - (y - min_y) / (max_y - min_y) * plot_h;
            if i == 0 {
                d.push_str(&format!("M {:.1} {:.1}", px, py));
            } else {
                d.push_str(&format!(" L {:.1} {:.1}", px, py));
            }
        }
        let stroke = palette(idx);
        paths.push_str(&format!(
            "<path d=\"{d}\" stroke=\"{stroke}\" fill=\"none\" stroke-width=\"1.5\"/>"
        ));
        let label = escape_xml(&s.label);
        let ly = margin + (idx as f64 * 14.0);
        paths.push_str(&format!(
            "<text x=\"{lx}\" y=\"{ly}\" fill=\"{stroke}\" font-size=\"10\">{label}</text>",
            lx = width as f64 - margin - 60.0
        ));
    }
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">\
         <rect x=\"0\" y=\"0\" width=\"{width}\" height=\"{height}\" fill=\"#1a1a1a\"/>\
         <text x=\"6\" y=\"12\" fill=\"#ddd\" font-size=\"11\">{title} ({unit})</text>\
         {paths}\
         </svg>"
    ))
}

/// Single big number — for `Stat` panels.
pub fn render_stat_svg(panel: &PanelDef, value: f64) -> Result<String, RendererError> {
    if !matches!(panel.kind, PanelKind::Stat) {
        return Err(RendererError::Unsupported(panel.kind));
    }
    let title = escape_xml(&panel.title);
    let unit = escape_xml(&panel.unit);
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"160\" height=\"80\">\
         <rect width=\"160\" height=\"80\" fill=\"#1a1a1a\"/>\
         <text x=\"80\" y=\"30\" fill=\"#aaa\" font-size=\"10\" text-anchor=\"middle\">{title}</text>\
         <text x=\"80\" y=\"60\" fill=\"#fff\" font-size=\"22\" text-anchor=\"middle\">{value:.2} {unit}</text>\
         </svg>"
    ))
}

/// Gauge — value relative to [min, max].
pub fn render_gauge_svg(
    panel: &PanelDef,
    value: f64,
    min: f64,
    max: f64,
) -> Result<String, RendererError> {
    if !matches!(panel.kind, PanelKind::Gauge) {
        return Err(RendererError::Unsupported(panel.kind));
    }
    if max <= min {
        return Err(RendererError::DimensionOverflow);
    }
    let pct = ((value - min) / (max - min)).clamp(0.0, 1.0);
    let title = escape_xml(&panel.title);
    let bar = (pct * 140.0) as u32;
    Ok(format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"160\" height=\"40\">\
         <rect width=\"160\" height=\"40\" fill=\"#1a1a1a\"/>\
         <text x=\"6\" y=\"12\" fill=\"#aaa\" font-size=\"10\">{title}</text>\
         <rect x=\"10\" y=\"22\" width=\"140\" height=\"10\" fill=\"#333\"/>\
         <rect x=\"10\" y=\"22\" width=\"{bar}\" height=\"10\" fill=\"#4c1\"/>\
         </svg>"
    ))
}

fn palette(idx: usize) -> &'static str {
    const COLORS: &[&str] = &[
        "#7eb6ff", "#ff7e88", "#9bda9b", "#ffb37e", "#c08eff", "#7effd9",
    ];
    COLORS[idx % COLORS.len()]
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Resolve all variable references for a dashboard, producing a
/// dashboard with concrete query strings. Useful for the page render path.
pub fn render_dashboard_resolved(
    dashboard: &DashboardDef,
    vars: &HashMap<String, String>,
) -> Result<DashboardDef, RendererError> {
    let mut resolved = dashboard.clone();
    for panel in &mut resolved.panels {
        panel.query = resolve_query(&panel.query, vars)?;
    }
    Ok(resolved)
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::grafana::PanelDef;

    fn vars(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn panel(kind: PanelKind, title: &str, query: &str) -> PanelDef {
        PanelDef::new("p1", title, kind, query)
    }

    #[test]
    fn resolve_no_vars_passthrough() {
        let v = HashMap::new();
        assert_eq!(resolve_query("rate(req[5m])", &v).unwrap(), "rate(req[5m])");
    }

    #[test]
    fn resolve_simple_var() {
        let v = vars(&[("env", "prod")]);
        let out = resolve_query("up{env=$env}", &v).unwrap();
        assert_eq!(out, "up{env=prod}");
    }

    #[test]
    fn resolve_multiple_vars() {
        let v = vars(&[("env", "prod"), ("svc", "web")]);
        let out = resolve_query("$svc.$env", &v).unwrap();
        assert_eq!(out, "web.prod");
    }

    #[test]
    fn resolve_underscore_var() {
        let v = vars(&[("my_var", "X")]);
        let out = resolve_query("$my_var-trailing", &v).unwrap();
        assert_eq!(out, "X-trailing");
    }

    #[test]
    fn resolve_var_starting_underscore() {
        let v = vars(&[("_x", "val")]);
        let out = resolve_query("$_x", &v).unwrap();
        assert_eq!(out, "val");
    }

    #[test]
    fn resolve_dollar_followed_by_digit_is_literal() {
        let v = HashMap::new();
        let out = resolve_query("$1 raw", &v).unwrap();
        assert_eq!(out, "$1 raw");
    }

    #[test]
    fn resolve_missing_variable_errors() {
        let v = HashMap::new();
        let err = resolve_query("$missing", &v).unwrap_err();
        assert!(matches!(err, RendererError::MissingVariable(n) if n == "missing"));
    }

    #[test]
    fn resolve_var_terminated_by_brace() {
        let v = vars(&[("env", "prod")]);
        let out = resolve_query("up{env=$env}", &v).unwrap();
        assert_eq!(out, "up{env=prod}");
    }

    #[test]
    fn referenced_variables_finds_all() {
        let refs = referenced_variables("$a + $b - $a");
        assert_eq!(refs, vec!["a".to_string(), "b".to_string()]);
    }

    #[test]
    fn referenced_variables_empty_when_none() {
        let refs = referenced_variables("plain query");
        assert!(refs.is_empty());
    }

    #[test]
    fn referenced_variables_ignores_dollar_digit() {
        let refs = referenced_variables("$1 $env");
        assert_eq!(refs, vec!["env".to_string()]);
    }

    #[test]
    fn render_timeseries_includes_dimensions() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let s = vec![Series { label: "a".into(), points: vec![(0.0, 0.0), (1.0, 1.0)] }];
        let svg = render_timeseries_svg(&p, &s, 200, 100).unwrap();
        assert!(svg.contains("width=\"200\""));
        assert!(svg.contains("height=\"100\""));
    }

    #[test]
    fn render_timeseries_includes_path() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let s = vec![Series { label: "a".into(), points: vec![(0.0, 0.0), (1.0, 1.0)] }];
        let svg = render_timeseries_svg(&p, &s, 200, 100).unwrap();
        assert!(svg.contains("<path d=\""));
    }

    #[test]
    fn render_timeseries_handles_empty_series() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let svg = render_timeseries_svg(&p, &[], 200, 100).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn render_timeseries_rejects_wrong_panel_kind() {
        let p = panel(PanelKind::Stat, "T", "x");
        let err = render_timeseries_svg(&p, &[], 200, 100).unwrap_err();
        assert!(matches!(err, RendererError::Unsupported(_)));
    }

    #[test]
    fn render_timeseries_rejects_too_small() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let err = render_timeseries_svg(&p, &[], 10, 10).unwrap_err();
        assert_eq!(err, RendererError::DimensionOverflow);
    }

    #[test]
    fn render_timeseries_rejects_too_large() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let err = render_timeseries_svg(&p, &[], 5000, 5000).unwrap_err();
        assert_eq!(err, RendererError::DimensionOverflow);
    }

    #[test]
    fn render_timeseries_escapes_title_xml() {
        let p = panel(PanelKind::Timeseries, "<bad>", "x");
        let svg = render_timeseries_svg(&p, &[], 200, 100).unwrap();
        assert!(!svg.contains("<bad>"));
        assert!(svg.contains("&lt;bad&gt;"));
    }

    #[test]
    fn render_timeseries_escapes_unit() {
        let mut p = panel(PanelKind::Timeseries, "T", "x");
        p.unit = "ms<".into();
        let svg = render_timeseries_svg(&p, &[], 200, 100).unwrap();
        assert!(svg.contains("ms&lt;"));
    }

    #[test]
    fn render_timeseries_constant_y_no_div_zero() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let s = vec![Series { label: "a".into(), points: vec![(0.0, 5.0), (1.0, 5.0)] }];
        let svg = render_timeseries_svg(&p, &s, 200, 100).unwrap();
        assert!(svg.contains("<svg"));
    }

    #[test]
    fn render_stat_includes_value() {
        let p = panel(PanelKind::Stat, "Latency", "x");
        let svg = render_stat_svg(&p, 42.5).unwrap();
        assert!(svg.contains("42.50"));
    }

    #[test]
    fn render_stat_rejects_wrong_kind() {
        let p = panel(PanelKind::Timeseries, "T", "x");
        let err = render_stat_svg(&p, 1.0).unwrap_err();
        assert!(matches!(err, RendererError::Unsupported(_)));
    }

    #[test]
    fn render_gauge_pct_fills_bar() {
        let p = panel(PanelKind::Gauge, "Use", "x");
        let svg = render_gauge_svg(&p, 50.0, 0.0, 100.0).unwrap();
        // 50% of 140 = 70
        assert!(svg.contains("width=\"70\""));
    }

    #[test]
    fn render_gauge_clamps_above_max() {
        let p = panel(PanelKind::Gauge, "Use", "x");
        let svg = render_gauge_svg(&p, 200.0, 0.0, 100.0).unwrap();
        // 100% of 140
        assert!(svg.contains("width=\"140\""));
    }

    #[test]
    fn render_gauge_clamps_below_min() {
        let p = panel(PanelKind::Gauge, "Use", "x");
        let svg = render_gauge_svg(&p, -10.0, 0.0, 100.0).unwrap();
        assert!(svg.contains("width=\"0\""));
    }

    #[test]
    fn render_gauge_rejects_inverted_bounds() {
        let p = panel(PanelKind::Gauge, "Use", "x");
        let err = render_gauge_svg(&p, 50.0, 100.0, 0.0).unwrap_err();
        assert_eq!(err, RendererError::DimensionOverflow);
    }

    #[test]
    fn render_gauge_rejects_wrong_kind() {
        let p = panel(PanelKind::Stat, "Use", "x");
        let err = render_gauge_svg(&p, 50.0, 0.0, 100.0).unwrap_err();
        assert!(matches!(err, RendererError::Unsupported(_)));
    }

    #[test]
    fn render_dashboard_resolved_substitutes_all() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(PanelDef::new("p1", "p", PanelKind::Timeseries, "up{env=$env}")).unwrap();
        d.add_panel(PanelDef::new("p2", "p", PanelKind::Stat, "$svc.req")).unwrap();
        let v = vars(&[("env", "prod"), ("svc", "web")]);
        let resolved = render_dashboard_resolved(&d, &v).unwrap();
        assert_eq!(resolved.panels[0].query, "up{env=prod}");
        assert_eq!(resolved.panels[1].query, "web.req");
    }

    #[test]
    fn render_dashboard_resolved_propagates_missing_var() {
        let mut d = DashboardDef::new("d1", "acme", "T");
        d.add_panel(PanelDef::new("p1", "p", PanelKind::Timeseries, "$ghost")).unwrap();
        let v = HashMap::new();
        let err = render_dashboard_resolved(&d, &v).unwrap_err();
        assert!(matches!(err, RendererError::MissingVariable(_)));
    }

    #[test]
    fn palette_cycles() {
        for i in 0..18 {
            let _ = palette(i);
        }
    }
}
