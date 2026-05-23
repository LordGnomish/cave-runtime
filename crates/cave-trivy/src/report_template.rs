// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Template report writer (`--format template --template @tpl`).
//!
//! Mirrors trivy's `pkg/report/template.go` for a curated Go-template
//! subset: `{{ .ArtifactName }}`, `{{ .ArtifactType }}`,
//! `{{ range .Results }}…{{ end }}`, `{{ range .Vulnerabilities }}…
//! {{ end }}`, `{{ .Severity }}`, `{{ .ID }}`, `{{ .PkgName }}`,
//! `{{ .InstalledVersion }}`. Full Go template parser is a scope cut.

use crate::error::{TrivyError, TrivyResult};
use crate::models::Report;

pub fn render(report: &Report, template: &str) -> TrivyResult<String> {
    let mut out = String::new();
    let mut s = template;
    while let Some(start) = s.find("{{") {
        out.push_str(&s[..start]);
        let rest = &s[start..];
        let end = rest
            .find("}}")
            .ok_or_else(|| TrivyError::Report("unterminated template tag".into()))?;
        let tag = &rest[2..end];
        s = &rest[end + 2..];
        let trim = tag.trim();
        if trim == "range .Results" {
            let (body, after) = split_block(s, "range .Results")?;
            for r in &report.results {
                let body_with_vulns = expand_results_body(body, r)?;
                out.push_str(&body_with_vulns);
            }
            s = after;
        } else {
            out.push_str(&substitute_top_level(trim, report));
        }
    }
    out.push_str(s);
    Ok(out)
}

fn split_block<'a>(s: &'a str, _name: &str) -> TrivyResult<(&'a str, &'a str)> {
    let mut depth: i32 = 1;
    let mut i = 0;
    while i < s.len() {
        let rest = &s[i..];
        if let Some(open) = rest.find("{{") {
            let tag_start = i + open + 2;
            let tag_end = match s[tag_start..].find("}}") {
                Some(e) => tag_start + e,
                None => return Err(TrivyError::Report("unterminated tag in block".into())),
            };
            let tag = s[tag_start..tag_end].trim();
            if tag == "end" {
                depth -= 1;
                if depth == 0 {
                    let body = &s[..i + open];
                    let after = &s[tag_end + 2..];
                    return Ok((body, after));
                }
            } else if tag.starts_with("range ") {
                depth += 1;
            }
            i = tag_end + 2;
        } else {
            break;
        }
    }
    Err(TrivyError::Report("missing {{end}}".into()))
}

fn substitute_top_level(tag: &str, r: &Report) -> String {
    match tag {
        ".ArtifactName" => r.artifact_name.clone(),
        ".ArtifactType" => r.artifact_type.clone(),
        ".SchemaVersion" => r.schema_version.to_string(),
        ".TotalVulns" => r.total_vulns().to_string(),
        ".TotalMisconfigs" => r.total_misconfigs().to_string(),
        ".CreatedAt" => r.created_at.clone(),
        _ => format!("{{{{ {} }}}}", tag),
    }
}

fn expand_results_body(body: &str, r: &crate::models::ScanResult) -> TrivyResult<String> {
    let mut out = String::new();
    let mut s = body;
    while let Some(start) = s.find("{{") {
        out.push_str(&s[..start]);
        let rest = &s[start..];
        let end = rest
            .find("}}")
            .ok_or_else(|| TrivyError::Report("unterminated nested tag".into()))?;
        let tag = rest[2..end].trim().to_string();
        s = &rest[end + 2..];
        if tag == "range .Vulnerabilities" {
            let (inner, after) = split_block(s, "range .Vulnerabilities")?;
            for v in &r.vulnerabilities {
                let mut piece = inner.to_string();
                piece = piece.replace("{{ .ID }}", &v.id);
                piece = piece.replace("{{ .Severity }}", v.severity.as_str());
                piece = piece.replace("{{ .PkgName }}", &v.pkg_name);
                piece = piece.replace("{{ .InstalledVersion }}", &v.installed_version);
                piece = piece.replace(
                    "{{ .FixedVersion }}",
                    v.fixed_version.as_deref().unwrap_or("-"),
                );
                out.push_str(&piece);
            }
            s = after;
        } else {
            match tag.as_str() {
                ".Target" => out.push_str(&r.target),
                ".Class" => out.push_str(&r.class),
                _ => out.push_str(&format!("{{{{ {} }}}}", tag)),
            }
        }
    }
    out.push_str(s);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Report, ScanResult, Vulnerability};
    use crate::severity::Severity;

    #[test]
    fn substitutes_artifact_name() {
        let r = Report::new("x", "y");
        let t = "name={{ .ArtifactName }} type={{ .ArtifactType }}";
        let s = render(&r, t).unwrap();
        assert_eq!(s, "name=x type=y");
    }

    #[test]
    fn range_results_and_vulns() {
        let mut r = Report::new("x", "y");
        r.results.push(ScanResult {
            target: "t".into(),
            class: "os".into(),
            vulnerabilities: vec![Vulnerability {
                id: "CVE-A".into(),
                pkg_name: "p".into(),
                installed_version: "1".into(),
                fixed_version: Some("2".into()),
                severity: Severity::High,
                references: vec![],
                title: None,
            }],
            ..Default::default()
        });
        let t = "{{ range .Results }}* {{ .Target }}\n{{ range .Vulnerabilities }} - {{ .ID }} [{{ .Severity }}] {{ .PkgName }}@{{ .InstalledVersion }} -> {{ .FixedVersion }}\n{{ end }}{{ end }}";
        let s = render(&r, t).unwrap();
        assert!(s.contains("* t"));
        assert!(s.contains("CVE-A [HIGH]"));
        assert!(s.contains("p@1 -> 2"));
    }

    #[test]
    fn unterminated_tag_errors() {
        let r = Report::new("x", "y");
        assert!(render(&r, "broken {{ .X").is_err());
    }

    #[test]
    fn unknown_tag_passthrough() {
        let r = Report::new("x", "y");
        let s = render(&r, "{{ .Mystery }}").unwrap();
        assert!(s.contains(".Mystery"));
    }

    #[test]
    fn totals_field() {
        let mut r = Report::new("x", "y");
        r.results.push(ScanResult {
            target: "t".into(),
            class: "os".into(),
            vulnerabilities: vec![Vulnerability::new("C", "p", "1", Severity::Low)],
            ..Default::default()
        });
        let s = render(&r, "total={{ .TotalVulns }}").unwrap();
        assert_eq!(s, "total=1");
    }
}
