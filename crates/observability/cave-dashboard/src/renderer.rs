// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HTML dashboard renderer — produces self-contained Bootstrap-based HTML
//! for embedding or standalone viewing. Mirrors Grafana's dark-theme aesthetics.

use crate::models::{Dashboard, Panel, PanelType, Variable, VariableHide, VariableType};

const DARK_BG: &str = "#111217";
const PANEL_BG: &str = "#1f1f2e";
const BORDER_COLOR: &str = "#2d2d44";
const TEXT_COLOR: &str = "#d8d9da";
const MUTED_COLOR: &str = "#8e8ea8";
const ACCENT_COLOR: &str = "#5794f2";

/// Render a full dashboard to self-contained HTML.
pub fn render_dashboard(dashboard: &Dashboard) -> String {
    let title = escape_html(&dashboard.title);
    let uid = &dashboard.uid;
    let version = dashboard.version;
    let from = &dashboard.time.from;
    let to = &dashboard.time.to;

    let tags_html = render_tags(&dashboard.tags);
    let variables_html = render_variables_bar(&dashboard.templating.list);
    let panels_html = render_panels(&dashboard.panels);
    let annotations_script = render_annotations_script();

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>{title} — CAVE Dashboard</title>
  <style>
    *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      background: {DARK_BG};
      color: {TEXT_COLOR};
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, Helvetica, Arial, sans-serif;
      font-size: 14px;
      min-height: 100vh;
    }}
    .db-header {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 12px 16px;
      border-bottom: 1px solid {BORDER_COLOR};
      background: #0f1117;
    }}
    .db-title {{
      font-size: 18px;
      font-weight: 500;
      color: #fff;
    }}
    .db-meta {{
      font-size: 12px;
      color: {MUTED_COLOR};
      margin-top: 2px;
    }}
    .db-time {{
      font-size: 12px;
      color: {ACCENT_COLOR};
      background: rgba(87,148,242,0.1);
      border: 1px solid rgba(87,148,242,0.3);
      border-radius: 4px;
      padding: 4px 8px;
    }}
    .tags {{
      display: flex;
      gap: 4px;
      flex-wrap: wrap;
      margin-top: 4px;
    }}
    .tag {{
      font-size: 11px;
      background: rgba(87,148,242,0.15);
      color: {ACCENT_COLOR};
      border-radius: 3px;
      padding: 1px 6px;
    }}
    .variables-bar {{
      display: flex;
      align-items: center;
      gap: 16px;
      padding: 8px 16px;
      background: #161621;
      border-bottom: 1px solid {BORDER_COLOR};
      flex-wrap: wrap;
    }}
    .var-group {{
      display: flex;
      align-items: center;
      gap: 6px;
    }}
    .var-label {{
      font-size: 12px;
      color: {MUTED_COLOR};
    }}
    .var-select, .var-input {{
      background: #22232e;
      color: {TEXT_COLOR};
      border: 1px solid {BORDER_COLOR};
      border-radius: 4px;
      padding: 4px 8px;
      font-size: 12px;
      min-width: 80px;
    }}
    .panels-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(380px, 1fr));
      gap: 8px;
      padding: 12px;
    }}
    .panel {{
      background: {PANEL_BG};
      border: 1px solid {BORDER_COLOR};
      border-radius: 6px;
      overflow: hidden;
      display: flex;
      flex-direction: column;
      min-height: 200px;
    }}
    .panel-row {{
      grid-column: 1 / -1;
      background: transparent;
      border: 1px solid {BORDER_COLOR};
      min-height: auto;
    }}
    .panel-header {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 8px 12px 6px;
      border-bottom: 1px solid {BORDER_COLOR};
    }}
    .panel-title {{
      font-size: 13px;
      font-weight: 500;
      color: {TEXT_COLOR};
    }}
    .panel-type-badge {{
      font-size: 10px;
      color: {MUTED_COLOR};
      background: #2d2d44;
      border-radius: 3px;
      padding: 1px 5px;
    }}
    .panel-body {{
      flex: 1;
      padding: 12px;
      display: flex;
      align-items: center;
      justify-content: center;
      color: {MUTED_COLOR};
      font-size: 12px;
    }}
    .panel-stat-value {{
      font-size: 40px;
      font-weight: 700;
      color: #fff;
      line-height: 1;
    }}
    .panel-stat-unit {{
      font-size: 16px;
      color: {MUTED_COLOR};
      margin-left: 4px;
    }}
    .panel-stat-title {{
      font-size: 12px;
      color: {MUTED_COLOR};
      margin-top: 4px;
    }}
    .panel-description {{
      font-size: 11px;
      color: {MUTED_COLOR};
      padding: 4px 12px 0;
    }}
    .alert-badge {{
      font-size: 10px;
      padding: 1px 5px;
      border-radius: 3px;
    }}
    .alert-ok {{ background: rgba(50,200,100,0.2); color: #32c864; }}
    .alert-firing {{ background: rgba(235,60,60,0.2); color: #eb3c3c; }}
    .gauge-bar {{
      width: 100%;
      height: 12px;
      background: #2d2d44;
      border-radius: 6px;
      overflow: hidden;
    }}
    .gauge-fill {{
      height: 100%;
      border-radius: 6px;
      background: linear-gradient(90deg, #1a6340, #32c864);
      transition: width 0.3s;
    }}
    .table-wrapper {{
      width: 100%;
      overflow-x: auto;
    }}
    .panel-table {{
      width: 100%;
      border-collapse: collapse;
      font-size: 12px;
    }}
    .panel-table th {{
      color: {MUTED_COLOR};
      font-weight: 500;
      padding: 6px 8px;
      border-bottom: 1px solid {BORDER_COLOR};
      text-align: left;
    }}
    .panel-table td {{
      padding: 5px 8px;
      border-bottom: 1px solid rgba(45,45,68,0.5);
    }}
    .text-panel {{ padding: 16px; font-size: 13px; line-height: 1.6; }}
    .logs-entry {{
      display: flex;
      gap: 8px;
      padding: 3px 0;
      border-bottom: 1px solid rgba(45,45,68,0.3);
      font-family: "JetBrains Mono", "Fira Code", Consolas, monospace;
      font-size: 11px;
    }}
    .logs-time {{ color: {MUTED_COLOR}; white-space: nowrap; }}
    .logs-line {{ color: {TEXT_COLOR}; word-break: break-all; }}
    .chart-placeholder {{
      width: 100%;
      height: 140px;
      background: linear-gradient(180deg, rgba(87,148,242,0.05) 0%, rgba(87,148,242,0) 100%);
      border-radius: 4px;
      display: flex;
      align-items: flex-end;
      padding: 8px;
      gap: 3px;
    }}
    .chart-bar {{
      background: {ACCENT_COLOR};
      opacity: 0.7;
      border-radius: 2px 2px 0 0;
      flex: 1;
    }}
    footer {{
      text-align: center;
      padding: 16px;
      font-size: 11px;
      color: {MUTED_COLOR};
      border-top: 1px solid {BORDER_COLOR};
      margin-top: 8px;
    }}
  </style>
</head>
<body>
  <header class="db-header">
    <div>
      <div class="db-title">{title}</div>
      <div class="db-meta">UID: {uid} &nbsp;·&nbsp; v{version}</div>
      {tags_html}
    </div>
    <div class="db-time">{from} → {to}</div>
  </header>

  {variables_html}

  <main class="panels-grid">
    {panels_html}
  </main>

  <footer>CAVE Dashboard · Grafana-compatible rendering engine · Generated at {}</footer>
  {annotations_script}
</body>
</html>"#,
        chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC"),
    )
}

fn render_tags(tags: &[String]) -> String {
    if tags.is_empty() {
        return String::new();
    }
    let inner: String = tags
        .iter()
        .map(|t| format!(r#"<span class="tag">{}</span>"#, escape_html(t)))
        .collect::<Vec<_>>()
        .join("");
    format!(r#"<div class="tags">{inner}</div>"#)
}

fn render_variables_bar(vars: &[Variable]) -> String {
    let visible: Vec<&Variable> = vars
        .iter()
        .filter(|v| !matches!(v.hide, VariableHide::HideVariable))
        .collect();

    if visible.is_empty() {
        return String::new();
    }

    let inner: String = visible
        .iter()
        .map(|v| render_variable(v))
        .collect::<Vec<_>>()
        .join("\n");
    format!(r#"<div class="variables-bar">{inner}</div>"#)
}

fn render_variable(var: &Variable) -> String {
    let label = if !var.label.is_empty() {
        escape_html(&var.label)
    } else {
        escape_html(&var.name)
    };

    let current_val = match &var.current.value {
        serde_json::Value::String(s) => escape_html(s),
        serde_json::Value::Array(arr) => arr
            .iter()
            .filter_map(|v| v.as_str())
            .map(escape_html)
            .collect::<Vec<_>>()
            .join(", "),
        other => escape_html(&other.to_string()),
    };

    match var.var_type {
        VariableType::Textbox => {
            format!(
                r#"<div class="var-group"><span class="var-label">{label}</span><input class="var-input" type="text" value="{current_val}" /></div>"#
            )
        }
        _ => {
            let options_html: String = var
                .options
                .iter()
                .map(|opt| {
                    let val = match &opt.value {
                        serde_json::Value::String(s) => escape_html(s),
                        other => escape_html(&other.to_string()),
                    };
                    let txt = match &opt.text {
                        serde_json::Value::String(s) => escape_html(s),
                        other => escape_html(&other.to_string()),
                    };
                    let selected = if opt.selected { " selected" } else { "" };
                    format!(r#"<option value="{val}"{selected}>{txt}</option>"#)
                })
                .collect::<Vec<_>>()
                .join("");

            let current_opt =
                format!(r#"<option value="{current_val}" selected>{current_val}</option>"#);
            let opts = if options_html.is_empty() {
                current_opt
            } else {
                options_html
            };

            format!(
                r#"<div class="var-group"><span class="var-label">{label}</span><select class="var-select">{opts}</select></div>"#
            )
        }
    }
}

fn render_panels(panels: &[Panel]) -> String {
    panels
        .iter()
        .map(render_panel)
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_panel(panel: &Panel) -> String {
    let title = escape_html(&panel.title);
    let type_name = panel.panel_type.to_string();
    let type_icon = panel_type_icon(panel.panel_type);

    let description_html = if !panel.description.is_empty() {
        format!(
            r#"<div class="panel-description">{}</div>"#,
            escape_html(&panel.description)
        )
    } else {
        String::new()
    };

    let alert_badge = if let Some(ref alert) = panel.alert {
        let (class, label) = match alert.state {
            crate::models::AlertState::Normal => ("alert-ok", "OK"),
            crate::models::AlertState::Firing => ("alert-firing", "FIRING"),
            _ => ("alert-ok", "OK"),
        };
        format!(r#" <span class="alert-badge {class}">{label}</span>"#)
    } else {
        String::new()
    };

    let extra_class = if panel.panel_type == PanelType::Row {
        " panel-row"
    } else {
        ""
    };
    let body = render_panel_body(panel);

    format!(
        r#"<div class="panel{extra_class}">
  <div class="panel-header">
    <span class="panel-title">{type_icon} {title}{alert_badge}</span>
    <span class="panel-type-badge">{type_name}</span>
  </div>
  {description_html}
  {body}
</div>"#
    )
}

fn render_panel_body(panel: &Panel) -> String {
    match panel.panel_type {
        PanelType::Stat => render_stat_panel(panel),
        PanelType::Gauge => render_gauge_panel(panel),
        PanelType::Graph => render_graph_panel(panel),
        PanelType::BarGauge => render_bar_gauge_panel(panel),
        PanelType::Table => render_table_panel(panel),
        PanelType::Text => render_text_panel(panel),
        PanelType::Logs => render_logs_panel(panel),
        PanelType::AlertList => render_alert_list_panel(panel),
        PanelType::DashboardList => render_dashboard_list_panel(panel),
        PanelType::Row => String::new(),
        PanelType::Traces => render_traces_panel(panel),
        PanelType::PieChart => render_pie_chart_panel(panel),
        PanelType::Heatmap => render_heatmap_panel(panel),
        _ => render_generic_panel(panel),
    }
}

fn render_stat_panel(panel: &Panel) -> String {
    let unit = panel.field_config.defaults.unit.as_str();
    let target_expr = panel
        .targets
        .first()
        .map(|t| escape_html(&t.expr))
        .unwrap_or_default();
    format!(
        r#"<div class="panel-body" style="flex-direction:column;">
  <div class="panel-stat-value">—<span class="panel-stat-unit">{unit}</span></div>
  <div class="panel-stat-title">{target_expr}</div>
</div>"#
    )
}

fn render_gauge_panel(panel: &Panel) -> String {
    let max = panel.field_config.defaults.max.unwrap_or(100.0);
    let min = panel.field_config.defaults.min.unwrap_or(0.0);
    format!(
        r#"<div class="panel-body" style="flex-direction:column;gap:8px;width:100%;padding:16px;">
  <div class="panel-stat-value">—</div>
  <div class="gauge-bar"><div class="gauge-fill" style="width:0%"></div></div>
  <div style="display:flex;justify-content:space-between;font-size:11px;color:{MUTED_COLOR}">
    <span>{min}</span><span>{max}</span>
  </div>
</div>"#
    )
}

fn render_graph_panel(panel: &Panel) -> String {
    // Render a decorative bar chart placeholder
    let bars: String = (0..20)
        .map(|i| {
            let h = 20 + (i * 7 + i * i / 4) % 80;
            format!(r#"<div class="chart-bar" style="height:{h}%"></div>"#)
        })
        .collect::<Vec<_>>()
        .join("");

    format!(
        r#"<div class="panel-body" style="flex-direction:column;width:100%;padding:8px 12px;">
  <div class="chart-placeholder">{bars}</div>
</div>"#
    )
}

fn render_bar_gauge_panel(panel: &Panel) -> String {
    let targets: String = panel.targets.iter().enumerate().map(|(i, t)| {
        let pct = (30 + i * 15) % 100;
        let label = escape_html(&t.legend_format.replace("{{", "").replace("}}", ""));
        let label = if label.is_empty() { format!("Series {}", (b'A' + i as u8) as char) } else { label };
        format!(
            r#"<div style="display:flex;align-items:center;gap:8px;margin-bottom:6px;">
  <span style="font-size:11px;color:{MUTED_COLOR};min-width:80px;overflow:hidden;text-overflow:ellipsis">{label}</span>
  <div class="gauge-bar" style="flex:1"><div class="gauge-fill" style="width:{pct}%"></div></div>
  <span style="font-size:12px;min-width:30px;text-align:right">{pct}</span>
</div>"#
        )
    }).collect();

    if targets.is_empty() {
        return format!(r#"<div class="panel-body">No targets configured</div>"#);
    }

    format!(
        r#"<div class="panel-body" style="flex-direction:column;width:100%;padding:12px;">{targets}</div>"#
    )
}

fn render_table_panel(panel: &Panel) -> String {
    let cols = vec!["Time", "Value", "Label"];
    let header: String = cols.iter().map(|c| format!("<th>{c}</th>")).collect();
    let rows: String = (0..3)
        .map(|_| {
            let cells: String = cols.iter().map(|_| "<td>—</td>").collect();
            format!("<tr>{cells}</tr>")
        })
        .collect();

    format!(
        r#"<div class="panel-body" style="align-items:flex-start;padding:0;">
  <div class="table-wrapper">
    <table class="panel-table">
      <thead><tr>{header}</tr></thead>
      <tbody>{rows}</tbody>
    </table>
  </div>
</div>"#
    )
}

fn render_text_panel(panel: &Panel) -> String {
    let content = panel
        .options
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("*No content configured*");
    format!(
        r#"<div class="panel-body text-panel">{}</div>"#,
        escape_html(content)
    )
}

fn render_logs_panel(panel: &Panel) -> String {
    let entries: String = (0..5)
        .map(|i| {
            format!(
                r#"<div class="logs-entry">
  <span class="logs-time">2026-04-12 00:0{i}:00</span>
  <span class="logs-line">— no live data —</span>
</div>"#
            )
        })
        .collect();
    format!(
        r#"<div class="panel-body" style="flex-direction:column;align-items:flex-start;overflow:auto;width:100%;padding:8px 12px;">
{entries}
</div>"#
    )
}

fn render_traces_panel(panel: &Panel) -> String {
    format!(r#"<div class="panel-body">🔍 Trace viewer — connect a Jaeger/Tempo datasource</div>"#)
}

fn render_pie_chart_panel(panel: &Panel) -> String {
    format!(r#"<div class="panel-body">🥧 Pie chart — awaiting data</div>"#)
}

fn render_heatmap_panel(panel: &Panel) -> String {
    format!(r#"<div class="panel-body">🗺 Heatmap — awaiting data</div>"#)
}

fn render_alert_list_panel(panel: &Panel) -> String {
    format!(
        r#"<div class="panel-body" style="flex-direction:column;align-items:flex-start;width:100%;padding:12px;">
  <div style="color:{MUTED_COLOR};font-size:12px;">No alerts to display</div>
</div>"#
    )
}

fn render_dashboard_list_panel(panel: &Panel) -> String {
    format!(
        r#"<div class="panel-body" style="flex-direction:column;align-items:flex-start;width:100%;padding:12px;">
  <div style="color:{MUTED_COLOR};font-size:12px;">No dashboards to display</div>
</div>"#
    )
}

fn render_generic_panel(panel: &Panel) -> String {
    let target_expr = panel
        .targets
        .first()
        .map(|t| escape_html(&t.expr))
        .unwrap_or_default();
    format!(
        r#"<div class="panel-body">
  <div style="text-align:center;">
    <div style="font-size:24px;margin-bottom:8px;">{}</div>
    <div style="font-size:11px;color:{MUTED_COLOR}">{}</div>
    {}
  </div>
</div>"#,
        panel_type_icon(panel.panel_type),
        panel.panel_type,
        if !target_expr.is_empty() {
            format!(r#"<code style="font-size:10px;color:{MUTED_COLOR}">{target_expr}</code>"#)
        } else {
            String::new()
        }
    )
}

fn panel_type_icon(pt: PanelType) -> &'static str {
    match pt {
        PanelType::Graph => "📈",
        PanelType::Stat => "🔢",
        PanelType::Gauge => "🌡",
        PanelType::Table => "📋",
        PanelType::BarGauge => "📊",
        PanelType::PieChart => "🥧",
        PanelType::Heatmap => "🗺",
        PanelType::Logs => "📜",
        PanelType::Traces => "🔍",
        PanelType::Text => "📝",
        PanelType::AlertList => "🚨",
        PanelType::DashboardList => "🗂",
        PanelType::Row => "─",
        PanelType::Histogram => "📊",
        PanelType::StateTimeline => "⏳",
        PanelType::StatusHistory => "📅",
        PanelType::Candlestick => "🕯",
        PanelType::Flamegraph => "🔥",
        _ => "▪",
    }
}

fn render_annotations_script() -> &'static str {
    r#"<script>
// CAVE Dashboard — annotation markers would render here with live data
console.info('CAVE Dashboard renderer loaded — connect datasources for live data');
</script>"#
}

/// Escape HTML special characters to prevent XSS.
pub fn escape_html(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '&' => "&amp;".to_string(),
            '<' => "&lt;".to_string(),
            '>' => "&gt;".to_string(),
            '"' => "&quot;".to_string(),
            '\'' => "&#39;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Dashboard;

    #[test]
    fn test_escape_html() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("foo & bar"), "foo &amp; bar");
        assert_eq!(escape_html(r#"he said "hi""#), "he said &quot;hi&quot;");
    }

    #[test]
    fn test_render_empty_dashboard() {
        let db = Dashboard::new(1, 1, "Test Dashboard");
        let html = render_dashboard(&db);
        assert!(html.contains("Test Dashboard"));
        assert!(html.contains("CAVE Dashboard"));
        assert!(html.contains("<!DOCTYPE html>"));
    }

    #[test]
    fn test_render_tags() {
        let html = render_tags(&["prod".to_string(), "infra".to_string()]);
        assert!(html.contains("prod"));
        assert!(html.contains("infra"));
    }

    #[test]
    fn test_panel_type_icons_no_panic() {
        // Ensure all enum variants have an icon mapping
        for pt in [
            PanelType::Graph,
            PanelType::Stat,
            PanelType::Gauge,
            PanelType::Table,
            PanelType::BarGauge,
            PanelType::Logs,
            PanelType::Text,
            PanelType::AlertList,
        ] {
            let _ = panel_type_icon(pt);
        }
    }
}
