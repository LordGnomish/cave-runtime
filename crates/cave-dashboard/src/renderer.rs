//! Embedded HTML renderer for CAVE Dashboard.
//!
//! Generates a self-contained Bootstrap-based HTML page for each dashboard
//! so that cave-portal can embed it in an iframe.

use crate::models::{Dashboard, PanelType};

/// Render a full dashboard as a standalone HTML document.
pub fn render_dashboard_html(dashboard: &Dashboard) -> String {
    let title = html_escape(&dashboard.title);
    let panels_html = render_panels(dashboard);
    let vars_html = render_variables(dashboard);

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>CAVE Dashboard — {title}</title>
  <style>
    *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
    body {{
      font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
      background: #111217;
      color: #d8d9da;
      padding: 16px;
    }}
    .dashboard-header {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      margin-bottom: 16px;
      padding-bottom: 12px;
      border-bottom: 1px solid #22252b;
    }}
    .dashboard-title {{
      font-size: 20px;
      font-weight: 500;
      color: #e0e0e0;
    }}
    .dashboard-meta {{
      font-size: 12px;
      color: #6e6e6e;
    }}
    .tags {{
      display: flex;
      gap: 6px;
      flex-wrap: wrap;
      margin-top: 4px;
    }}
    .tag {{
      background: #1e2028;
      border: 1px solid #333;
      border-radius: 3px;
      padding: 1px 8px;
      font-size: 11px;
      color: #a0a0a0;
    }}
    .variables-bar {{
      display: flex;
      gap: 12px;
      flex-wrap: wrap;
      margin-bottom: 12px;
      padding: 8px 12px;
      background: #1a1c21;
      border-radius: 4px;
      border: 1px solid #22252b;
    }}
    .variable {{
      display: flex;
      align-items: center;
      gap: 6px;
      font-size: 12px;
    }}
    .variable label {{ color: #8e8e8e; }}
    .variable select, .variable input {{
      background: #111217;
      border: 1px solid #333;
      color: #d8d9da;
      border-radius: 3px;
      padding: 2px 8px;
      font-size: 12px;
    }}
    .panels-grid {{
      display: grid;
      grid-template-columns: repeat(auto-fill, minmax(400px, 1fr));
      gap: 12px;
    }}
    .panel {{
      background: #1a1c21;
      border: 1px solid #22252b;
      border-radius: 4px;
      overflow: hidden;
      display: flex;
      flex-direction: column;
    }}
    .panel-header {{
      display: flex;
      align-items: center;
      justify-content: space-between;
      padding: 8px 12px;
      border-bottom: 1px solid #22252b;
    }}
    .panel-title {{
      font-size: 13px;
      font-weight: 500;
      color: #c7c7c7;
    }}
    .panel-type-badge {{
      font-size: 10px;
      padding: 2px 6px;
      border-radius: 10px;
      background: #22252b;
      color: #6c6c6c;
      text-transform: uppercase;
      letter-spacing: 0.05em;
    }}
    .panel-body {{
      flex: 1;
      padding: 12px;
      min-height: 100px;
      display: flex;
      align-items: center;
      justify-content: center;
      color: #5c5c5c;
      font-size: 12px;
    }}
    .panel-placeholder {{
      width: 100%;
      text-align: center;
    }}
    .panel-placeholder .icon {{
      font-size: 32px;
      margin-bottom: 8px;
    }}
    .panel-alert-badge {{
      font-size: 10px;
      padding: 2px 6px;
      border-radius: 3px;
      background: #1a3a1a;
      color: #4caf50;
      border: 1px solid #2e5c2e;
    }}
    .panel-alert-badge.firing {{
      background: #3a1a1a;
      color: #f44336;
      border-color: #5c2e2e;
    }}
    .time-range {{
      font-size: 12px;
      color: #6e6e6e;
    }}
    .footer {{
      margin-top: 24px;
      padding-top: 12px;
      border-top: 1px solid #22252b;
      font-size: 11px;
      color: #444;
      text-align: center;
    }}
  </style>
</head>
<body>
  <div class="dashboard-header">
    <div>
      <div class="dashboard-title">{title}</div>
      {tags_html}
    </div>
    <div class="dashboard-meta">
      <div class="time-range">⏱ {time_from} → {time_to}</div>
      <div>v{version} · uid: {uid}</div>
    </div>
  </div>

  {vars_html}

  <div class="panels-grid">
    {panels_html}
  </div>

  <div class="footer">
    Rendered by CAVE Dashboard · Grafana-compatible · <a href="/api/dashboards/uid/{uid}" style="color:#555">API</a>
  </div>
</body>
</html>"#,
        title = title,
        tags_html = render_tags(dashboard),
        time_from = html_escape(&dashboard.time.from),
        time_to = html_escape(&dashboard.time.to),
        version = dashboard.version,
        uid = html_escape(&dashboard.uid),
        vars_html = vars_html,
        panels_html = panels_html,
    )
}

fn render_tags(dashboard: &Dashboard) -> String {
    if dashboard.tags.is_empty() {
        return String::new();
    }
    let tags: String = dashboard
        .tags
        .iter()
        .map(|t| format!(r#"<span class="tag">{}</span>"#, html_escape(t)))
        .collect::<Vec<_>>()
        .join("\n        ");
    format!(r#"<div class="tags">{tags}</div>"#)
}

fn render_variables(dashboard: &Dashboard) -> String {
    if dashboard.variables.is_empty() {
        return String::new();
    }
    let items: String = dashboard
        .variables
        .iter()
        .map(|v| {
            let label = v.label.as_deref().unwrap_or(&v.name);
            let current = v.current_value().unwrap_or("");
            if v.options.len() > 1 {
                let opts: String = v
                    .options
                    .iter()
                    .map(|o| {
                        let sel = if o.selected { " selected" } else { "" };
                        format!(r#"<option value="{}" {}>{}</option>"#, html_escape(&o.value), sel, html_escape(&o.text))
                    })
                    .collect::<Vec<_>>()
                    .join("");
                format!(
                    r#"<div class="variable"><label>{label}:</label><select>{opts}</select></div>"#,
                    label = html_escape(label),
                    opts = opts
                )
            } else {
                format!(
                    r#"<div class="variable"><label>{label}:</label><input type="text" value="{val}"></div>"#,
                    label = html_escape(label),
                    val = html_escape(current)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n    ");
    format!(r#"<div class="variables-bar">{items}</div>"#)
}

fn render_panels(dashboard: &Dashboard) -> String {
    let mut html = String::new();

    for panel in &dashboard.panels {
        let type_badge = panel_type_label(&panel.panel_type);
        let icon = panel_icon(&panel.panel_type);
        let desc = panel.description.as_deref().unwrap_or("No data loaded — connect a data source");
        let alert_html = panel.alert.as_ref().map(|a| {
            let cls = match a.state {
                crate::models::AlertState::Alerting => "firing",
                _ => "",
            };
            format!(r#"<span class="panel-alert-badge {cls}">⚡ {name}</span>"#, cls = cls, name = html_escape(&a.name))
        }).unwrap_or_default();

        html.push_str(&format!(
            r#"<div class="panel">
      <div class="panel-header">
        <span class="panel-title">{title}</span>
        <div style="display:flex;gap:6px;align-items:center">
          {alert_html}
          <span class="panel-type-badge">{type_badge}</span>
        </div>
      </div>
      <div class="panel-body">
        <div class="panel-placeholder">
          <div class="icon">{icon}</div>
          <div>{desc}</div>
        </div>
      </div>
    </div>
"#,
            title = html_escape(&panel.title),
            type_badge = type_badge,
            icon = icon,
            desc = html_escape(desc),
            alert_html = alert_html,
        ));
    }

    if html.is_empty() {
        html.push_str(r#"<div style="grid-column:1/-1;text-align:center;color:#444;padding:48px">No panels added yet</div>"#);
    }

    html
}

fn panel_type_label(pt: &PanelType) -> &'static str {
    match pt {
        PanelType::Graph => "time series",
        PanelType::Stat => "stat",
        PanelType::Gauge => "gauge",
        PanelType::Table => "table",
        PanelType::BarChart => "bar chart",
        PanelType::PieChart => "pie chart",
        PanelType::Heatmap => "heatmap",
        PanelType::Logs => "logs",
        PanelType::AlertList => "alert list",
    }
}

fn panel_icon(pt: &PanelType) -> &'static str {
    match pt {
        PanelType::Graph => "📈",
        PanelType::Stat => "🔢",
        PanelType::Gauge => "🌡️",
        PanelType::Table => "📋",
        PanelType::BarChart => "📊",
        PanelType::PieChart => "🥧",
        PanelType::Heatmap => "🟥",
        PanelType::Logs => "📜",
        PanelType::AlertList => "🚨",
    }
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}
