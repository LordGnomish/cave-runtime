// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Alert annotation templating — Go `text/template` subset.
//!
//! upstream: prometheus/prometheus — pkg/template/template.go
//!
//! Prometheus expands alert annotations and labels through Go's
//! `text/template`. Cave Runtime has historically used `tera` for
//! cave-alerts; this module ports the subset of Go text/template that
//! shows up in real-world alert rules so PromQL-native annotation maps
//! "just work" without translation. Supported:
//!
//!   * `{{ $variable }}` substitution
//!   * `{{ .Field }}` access on the implicit `data` map
//!   * `{{ if cond }}…{{ end }}` and `{{ if cond }}…{{ else }}…{{ end }}`
//!   * `{{ range $i, $v := list }}…{{ end }}`
//!   * `{{ printf "%.2f" $value }}`
//!   * `{{ .Labels.<key> }}` shortcut into a `labels` HashMap
//!
//! Anything else expands to the raw `{{ … }}` literal so the original
//! string is never silently dropped.

use std::collections::HashMap;

/// Per-render context provided to the templater.
#[derive(Default, Clone, Debug)]
pub struct TemplateContext {
    pub vars: HashMap<String, String>,
    pub labels: HashMap<String, String>,
    pub value: Option<f64>,
    /// `range` iteration source — a list of (key, value) pairs.
    pub list: Vec<(String, String)>,
}

impl TemplateContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_var(mut self, k: &str, v: &str) -> Self {
        self.vars.insert(k.to_string(), v.to_string());
        self
    }

    pub fn set_label(mut self, k: &str, v: &str) -> Self {
        self.labels.insert(k.to_string(), v.to_string());
        self
    }

    pub fn with_value(mut self, v: f64) -> Self {
        self.value = Some(v);
        self
    }
}

/// Expand a template against a context. Always returns a string — bad
/// templates fall through as the literal `{{ … }}` so the operator can
/// see what went wrong without losing the annotation.
pub fn render(template: &str, ctx: &TemplateContext) -> String {
    let mut out = String::new();
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if i + 1 < bytes.len() && bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            // find matching `}}`
            if let Some(end) = find_close(bytes, start) {
                let inner = std::str::from_utf8(&bytes[start..end]).unwrap_or("").trim();
                let consumed = end + 2;
                if let Some(piece) = expand_directive(inner, ctx, &template[consumed..]) {
                    out.push_str(&piece.expanded);
                    i = consumed + piece.consumed_extra;
                    continue;
                }
                // Unparseable — keep literal.
                out.push_str(&template[i..consumed]);
                i = consumed;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

struct ExpandedDirective {
    expanded: String,
    /// Number of additional bytes consumed past the closing `}}`.
    consumed_extra: usize,
}

fn expand_directive(inner: &str, ctx: &TemplateContext, rest: &str) -> Option<ExpandedDirective> {
    // ──── if … end ────
    if let Some(stripped) = inner.strip_prefix("if ") {
        let cond = stripped.trim();
        let (then_body, else_body, consumed_extra) = scan_if_blocks(rest)?;
        let take_then = eval_truthy(cond, ctx);
        let body = if take_then { then_body } else { else_body };
        return Some(ExpandedDirective {
            expanded: render(body, ctx),
            consumed_extra,
        });
    }
    if inner == "end" || inner == "else" {
        return Some(ExpandedDirective {
            expanded: String::new(),
            consumed_extra: 0,
        });
    }
    // ──── range $k, $v := list ────
    if let Some(stripped) = inner.strip_prefix("range ") {
        let parts: Vec<&str> = stripped.splitn(2, ":=").collect();
        if parts.len() == 2 {
            // we ignore variable names; iterate ctx.list.
            let (body, consumed_extra) = scan_block_until_end(rest)?;
            let mut buf = String::new();
            for (k, v) in &ctx.list {
                let mut child = ctx.clone();
                child.vars.insert("_k".into(), k.clone());
                child.vars.insert("_v".into(), v.clone());
                buf.push_str(&render(body, &child));
            }
            return Some(ExpandedDirective {
                expanded: buf,
                consumed_extra,
            });
        }
    }
    // ──── printf "..." args ────
    if let Some(stripped) = inner.strip_prefix("printf ") {
        let formatted = printf_render(stripped.trim(), ctx);
        return Some(ExpandedDirective {
            expanded: formatted,
            consumed_extra: 0,
        });
    }
    // ──── $var ────
    if let Some(name) = inner.strip_prefix('$') {
        if let Some(v) = ctx.vars.get(name) {
            return Some(ExpandedDirective {
                expanded: v.clone(),
                consumed_extra: 0,
            });
        }
        if name == "value" {
            return Some(ExpandedDirective {
                expanded: ctx.value.map(|v| format_float(v)).unwrap_or_default(),
                consumed_extra: 0,
            });
        }
    }
    // ──── .Field / .Labels.<key> / .Value ────
    if let Some(field) = inner.strip_prefix('.') {
        if let Some(label) = field.strip_prefix("Labels.") {
            return Some(ExpandedDirective {
                expanded: ctx.labels.get(label).cloned().unwrap_or_default(),
                consumed_extra: 0,
            });
        }
        if field == "Value" {
            return Some(ExpandedDirective {
                expanded: ctx.value.map(|v| format_float(v)).unwrap_or_default(),
                consumed_extra: 0,
            });
        }
        if let Some(v) = ctx.labels.get(field) {
            return Some(ExpandedDirective {
                expanded: v.clone(),
                consumed_extra: 0,
            });
        }
        if let Some(v) = ctx.vars.get(field) {
            return Some(ExpandedDirective {
                expanded: v.clone(),
                consumed_extra: 0,
            });
        }
    }
    None
}

fn find_close(bytes: &[u8], from: usize) -> Option<usize> {
    let mut i = from;
    while i + 1 < bytes.len() {
        if bytes[i] == b'}' && bytes[i + 1] == b'}' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn scan_if_blocks(rest: &str) -> Option<(&str, &str, usize)> {
    // Returns (then_body, else_body, consumed_extra) measured from `rest`.
    let bytes = rest.as_bytes();
    let mut depth = 1;
    let mut i = 0;
    let mut else_pos: Option<usize> = None;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(end) = find_close(bytes, start) {
                let inner = std::str::from_utf8(&bytes[start..end]).unwrap_or("").trim();
                if inner.starts_with("if ") {
                    depth += 1;
                } else if inner == "end" {
                    depth -= 1;
                    if depth == 0 {
                        let then_end = else_pos.unwrap_or(i);
                        let then = &rest[..then_end];
                        let else_part = if let Some(ep) = else_pos {
                            // find next directive to skip `{{else}}`
                            let after = ep + 2;
                            // skip the `else` directive itself
                            let after_close = find_close(bytes, after)?;
                            &rest[after_close + 2..i]
                        } else {
                            ""
                        };
                        return Some((then, else_part, end + 2));
                    }
                } else if inner == "else" && depth == 1 {
                    else_pos = Some(i);
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    None
}

fn scan_block_until_end(rest: &str) -> Option<(&str, usize)> {
    let bytes = rest.as_bytes();
    let mut depth = 1;
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let start = i + 2;
            if let Some(end) = find_close(bytes, start) {
                let inner = std::str::from_utf8(&bytes[start..end]).unwrap_or("").trim();
                if inner.starts_with("range ") || inner.starts_with("if ") {
                    depth += 1;
                } else if inner == "end" {
                    depth -= 1;
                    if depth == 0 {
                        return Some((&rest[..i], end + 2));
                    }
                }
                i = end + 2;
                continue;
            }
        }
        i += 1;
    }
    None
}

fn eval_truthy(cond: &str, ctx: &TemplateContext) -> bool {
    let cond = cond.trim();
    if let Some(name) = cond.strip_prefix('$') {
        return ctx.vars.get(name).map(|v| !v.is_empty()).unwrap_or(false);
    }
    if let Some(field) = cond.strip_prefix('.') {
        if field == "Value" {
            return ctx.value.unwrap_or(0.0) != 0.0;
        }
        if let Some(label) = field.strip_prefix("Labels.") {
            return ctx
                .labels
                .get(label)
                .map(|v| !v.is_empty())
                .unwrap_or(false);
        }
    }
    // numeric literal
    if let Ok(n) = cond.parse::<f64>() {
        return n != 0.0;
    }
    false
}

fn printf_render(args: &str, ctx: &TemplateContext) -> String {
    // Crude: "%.2f" $value | "%s" $foo. We parse the first quoted format
    // string and apply it to subsequent args.
    let chars: Vec<char> = args.chars().collect();
    if chars.is_empty() || chars[0] != '"' {
        return String::new();
    }
    let mut i = 1;
    let mut fmt = String::new();
    while i < chars.len() && chars[i] != '"' {
        if chars[i] == '\\' && i + 1 < chars.len() {
            fmt.push(chars[i + 1]);
            i += 2;
            continue;
        }
        fmt.push(chars[i]);
        i += 1;
    }
    i += 1;
    // collect remaining args
    let rest: String = chars[i..].iter().collect();
    let mut rest_args: Vec<&str> = rest.split_whitespace().collect();
    apply_printf(&fmt, &mut rest_args, ctx)
}

fn apply_printf(fmt: &str, args: &mut Vec<&str>, ctx: &TemplateContext) -> String {
    let mut out = String::new();
    let chars: Vec<char> = fmt.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' && i + 1 < chars.len() {
            // collect verb (width + precision + verb)
            let start = i;
            i += 1;
            while i < chars.len() && !"fdseg%".contains(chars[i]) {
                i += 1;
            }
            if i >= chars.len() {
                out.push_str(&fmt[start..]);
                break;
            }
            let verb = chars[i];
            let spec: String = chars[start..=i].iter().collect();
            i += 1;
            if verb == '%' {
                out.push('%');
                continue;
            }
            if args.is_empty() {
                out.push_str(&spec);
                continue;
            }
            let raw = args.remove(0);
            let value: f64 = if let Some(name) = raw.strip_prefix('$') {
                if name == "value" {
                    ctx.value.unwrap_or(0.0)
                } else {
                    ctx.vars
                        .get(name)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0)
                }
            } else if let Some(field) = raw.strip_prefix('.') {
                if field == "Value" {
                    ctx.value.unwrap_or(0.0)
                } else {
                    ctx.labels
                        .get(field)
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0.0)
                }
            } else {
                raw.parse().unwrap_or(0.0)
            };
            if verb == 'f' || verb == 'g' || verb == 'e' {
                let precision = parse_precision(&spec).unwrap_or(6);
                out.push_str(&format!("{:.*}", precision, value));
            } else if verb == 'd' {
                out.push_str(&format!("{}", value as i64));
            } else if verb == 's' {
                if let Some(name) = raw.strip_prefix('$') {
                    out.push_str(ctx.vars.get(name).map(String::as_str).unwrap_or(""));
                } else if let Some(field) = raw.strip_prefix('.') {
                    out.push_str(ctx.labels.get(field).map(String::as_str).unwrap_or(""));
                } else {
                    out.push_str(raw);
                }
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn parse_precision(spec: &str) -> Option<usize> {
    let dot = spec.find('.')?;
    let rest = &spec[dot + 1..];
    let mut p = String::new();
    for c in rest.chars() {
        if c.is_ascii_digit() {
            p.push(c);
        } else {
            break;
        }
    }
    p.parse().ok()
}

fn format_float(v: f64) -> String {
    if v.fract() == 0.0 && v.is_finite() {
        format!("{}", v as i64)
    } else {
        format!("{}", v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_dollar_variable() {
        let ctx = TemplateContext::new().set_var("foo", "bar");
        assert_eq!(render("hello {{ $foo }}", &ctx), "hello bar");
    }

    #[test]
    fn substitutes_dot_label_via_shortcut() {
        let ctx = TemplateContext::new().set_label("severity", "critical");
        assert_eq!(
            render("severity={{ .Labels.severity }}", &ctx),
            "severity=critical"
        );
    }

    #[test]
    fn substitutes_dot_value() {
        let ctx = TemplateContext::new().with_value(42.0);
        assert_eq!(render("value={{ .Value }}", &ctx), "value=42");
    }

    #[test]
    fn printf_format_float() {
        let ctx = TemplateContext::new().with_value(3.14159);
        assert_eq!(render("{{ printf \"%.2f\" $value }}", &ctx), "3.14");
    }

    #[test]
    fn if_branch_taken_when_label_present() {
        let ctx = TemplateContext::new().set_label("env", "prod");
        let t = "{{ if .Labels.env }}prod={{ .Labels.env }}{{ end }}";
        assert_eq!(render(t, &ctx), "prod=prod");
    }

    #[test]
    fn if_else_branch_taken_when_value_zero() {
        let ctx = TemplateContext::new().with_value(0.0);
        let t = "{{ if .Value }}up{{ else }}down{{ end }}";
        assert_eq!(render(t, &ctx), "down");
    }

    #[test]
    fn range_iterates_list_pairs() {
        let mut ctx = TemplateContext::new();
        ctx.list.push(("a".into(), "1".into()));
        ctx.list.push(("b".into(), "2".into()));
        let t = "{{ range $k, $v := list }}{{ $_k }}={{ $_v }};{{ end }}";
        assert_eq!(render(t, &ctx), "a=1;b=2;");
    }

    #[test]
    fn unknown_directive_left_literal() {
        let ctx = TemplateContext::new();
        let t = "hi {{ nope }} bye";
        assert_eq!(render(t, &ctx), "hi {{ nope }} bye");
    }

    #[test]
    fn nested_if_inside_range_renders_correctly() {
        let mut ctx = TemplateContext::new();
        ctx.list.push(("a".into(), "1".into()));
        ctx.list.push(("b".into(), "".into()));
        let t = "{{ range $k, $v := list }}{{ if $_v }}{{ $_k }}={{ $_v }};{{ end }}{{ end }}";
        assert_eq!(render(t, &ctx), "a=1;");
    }

    #[test]
    fn printf_int_verb() {
        let ctx = TemplateContext::new().set_var("n", "7");
        assert_eq!(render("{{ printf \"%d items\" $n }}", &ctx), "7 items");
    }
}
