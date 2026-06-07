// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! launchd LaunchAgent plist generation.
//!
//! [`AgentSpec::render`] is a pure `String`-producing function so the
//! exact XML is unit-testable (and `plutil -lint`-able in the real run)
//! without ever touching `~/Library/LaunchAgents`. The binary's
//! `install-agent` subcommand renders one of [`daily_report_agent`] /
//! [`metrics_serve_agent`] and writes it.

/// A LaunchAgent specification.
#[derive(Debug, Clone)]
pub struct AgentSpec {
    pub label: String,
    /// `ProgramArguments` — argv of the launched process.
    pub program: Vec<String>,
    pub working_dir: String,
    /// `EnvironmentVariables`, rendered in the given order.
    pub env: Vec<(String, String)>,
    /// `StartCalendarInterval` (hour, minute), local time per `TZ` env.
    pub calendar: Option<(u8, u8)>,
    pub run_at_load: bool,
    pub keep_alive: bool,
    pub stdout_path: String,
    pub stderr_path: String,
}

/// XML-escape a text node / element value (`&`, `<`, `>`).
fn xml(v: &str) -> String {
    v.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

impl AgentSpec {
    /// Render the full `.plist` XML document.
    pub fn render(&self) -> String {
        let mut s = String::new();
        s.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        s.push_str(
            "<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
             \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n",
        );
        s.push_str("<plist version=\"1.0\">\n<dict>\n");

        s.push_str(&format!(
            "\t<key>Label</key>\n\t<string>{}</string>\n",
            xml(&self.label)
        ));
        s.push_str("\t<key>Disabled</key>\n\t<false/>\n");
        s.push_str(&format!(
            "\t<key>WorkingDirectory</key>\n\t<string>{}</string>\n",
            xml(&self.working_dir)
        ));

        if !self.env.is_empty() {
            s.push_str("\t<key>EnvironmentVariables</key>\n\t<dict>\n");
            for (k, v) in &self.env {
                s.push_str(&format!(
                    "\t\t<key>{}</key>\n\t\t<string>{}</string>\n",
                    xml(k),
                    xml(v)
                ));
            }
            s.push_str("\t</dict>\n");
        }

        s.push_str("\t<key>ProgramArguments</key>\n\t<array>\n");
        for arg in &self.program {
            s.push_str(&format!("\t\t<string>{}</string>\n", xml(arg)));
        }
        s.push_str("\t</array>\n");

        if let Some((h, m)) = self.calendar {
            s.push_str("\t<key>StartCalendarInterval</key>\n\t<dict>\n");
            s.push_str(&format!("\t\t<key>Hour</key>\n\t\t<integer>{h}</integer>\n"));
            s.push_str(&format!("\t\t<key>Minute</key>\n\t\t<integer>{m}</integer>\n"));
            s.push_str("\t</dict>\n");
        }

        s.push_str(&format!(
            "\t<key>RunAtLoad</key>\n\t<{}/>\n",
            if self.run_at_load { "true" } else { "false" }
        ));
        s.push_str(&format!(
            "\t<key>KeepAlive</key>\n\t<{}/>\n",
            if self.keep_alive { "true" } else { "false" }
        ));
        s.push_str("\t<key>LowPriorityIO</key>\n\t<true/>\n");
        s.push_str("\t<key>Nice</key>\n\t<integer>10</integer>\n");
        s.push_str(&format!(
            "\t<key>StandardOutPath</key>\n\t<string>{}</string>\n",
            xml(&self.stdout_path)
        ));
        s.push_str(&format!(
            "\t<key>StandardErrorPath</key>\n\t<string>{}</string>\n",
            xml(&self.stderr_path)
        ));

        s.push_str("</dict>\n</plist>\n");
        s
    }
}

/// Shared env: HOME, a sane PATH (so `git`/`tokei` resolve), and TZ so
/// the calendar hour is interpreted as Europe/Berlin local time.
fn base_env(home: &str) -> Vec<(String, String)> {
    vec![
        ("HOME".to_string(), home.to_string()),
        (
            "PATH".to_string(),
            format!("{home}/.cargo/bin:{home}/.local/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin"),
        ),
        ("TZ".to_string(), "Europe/Berlin".to_string()),
    ]
}

/// The daily drift+LOC report agent — `report --measure` at 06:30 local
/// (30 min after the cave-home tracker, per the isolation note).
pub fn daily_report_agent(home: &str, bin: &str, support_dir: &str) -> AgentSpec {
    AgentSpec {
        label: "com.gnomish.cave-runtime-tracker".to_string(),
        program: vec![bin.to_string(), "report".to_string(), "--measure".to_string()],
        working_dir: home.to_string(),
        env: base_env(home),
        calendar: Some((6, 30)),
        run_at_load: true,
        keep_alive: false,
        stdout_path: format!("{support_dir}/runtime-tracker-daily.log"),
        stderr_path: format!("{support_dir}/runtime-tracker-daily.err"),
    }
}

/// The metrics daemon agent — `serve` kept alive, exposing `/metrics`.
/// Defaults to port 9103 (9101/9102 are taken by the cave autopilots).
pub fn metrics_serve_agent(home: &str, bin: &str, support_dir: &str, port: u16) -> AgentSpec {
    AgentSpec {
        label: "com.gnomish.cave-runtime-tracker-metrics".to_string(),
        program: vec![
            bin.to_string(),
            "serve".to_string(),
            "--port".to_string(),
            port.to_string(),
        ],
        working_dir: home.to_string(),
        env: base_env(home),
        calendar: None,
        run_at_load: true,
        keep_alive: true,
        stdout_path: format!("{support_dir}/runtime-tracker-metrics.log"),
        stderr_path: format!("{support_dir}/runtime-tracker-metrics.err"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn daily_agent_has_label_schedule_and_argv() {
        let p = daily_report_agent("/Users/x", "/Users/x/.local/bin/cave-runtime-tracker", "/Users/x/Library/Application Support/cave-runtime")
            .render();
        assert!(p.starts_with("<?xml version=\"1.0\""));
        assert!(p.contains("<string>com.gnomish.cave-runtime-tracker</string>"));
        assert!(p.contains("<key>Hour</key>\n\t\t<integer>6</integer>"));
        assert!(p.contains("<key>Minute</key>\n\t\t<integer>30</integer>"));
        assert!(p.contains("<string>report</string>"));
        assert!(p.contains("<string>--measure</string>"));
        assert!(p.contains("<key>RunAtLoad</key>\n\t<true/>"));
        // TZ pins the calendar interpretation.
        assert!(p.contains("<string>Europe/Berlin</string>"));
        // Well-formed: balanced plist envelope.
        assert!(p.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn metrics_agent_is_keepalive_with_no_calendar() {
        let p = metrics_serve_agent("/Users/x", "/Users/x/.local/bin/cave-runtime-tracker", "/Users/x/Library/Application Support/cave-runtime", 9103);
        assert_eq!(p.label, "com.gnomish.cave-runtime-tracker-metrics");
        assert!(p.calendar.is_none());
        assert!(p.keep_alive);
        let xml = p.render();
        assert!(xml.contains("<key>KeepAlive</key>\n\t<true/>"));
        assert!(xml.contains("<string>serve</string>"));
        assert!(xml.contains("<string>9103</string>"));
        assert!(!xml.contains("StartCalendarInterval"));
    }

    #[test]
    fn values_are_xml_escaped() {
        let mut spec = daily_report_agent("/Users/x", "/bin/t", "/sup");
        spec.program.push("a & b < c > d".to_string());
        let xml = spec.render();
        assert!(xml.contains("a &amp; b &lt; c &gt; d"));
        assert!(!xml.contains("a & b"));
    }
}
