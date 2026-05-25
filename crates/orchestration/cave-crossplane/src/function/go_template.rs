// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! function-go-templating — handlebars-like subset over Go template syntax.
//!
//! Upstream: function-go-templating/fn.go
//!
//! Supports:
//!   - `{{ .field }}` and `{{ .nested.field }}` substitution
//!   - `{{ if .field }}A{{ else }}B{{ end }}` conditionals
//!   - `{{ range .list }}<item={{ . }}>{{ end }}` iteration
//!   - Plain text passthrough
//!
//! Not supported: pipelines, function calls, comments inside actions, define/template,
//! sprig helpers — those go through cave-llm-gateway in Phase 2.

use serde_json::Value;

pub fn render_go_template(template: &str, ctx: &Value) -> Result<String, String> {
    let mut out = String::new();
    let mut cursor = 0;
    let bytes = template.as_bytes();
    while cursor < bytes.len() {
        if let Some(start) = find_open(template, cursor) {
            out.push_str(&template[cursor..start]);
            let Some(end) = find_close(template, start + 2) else {
                return Err(format!("unterminated action at byte {}", start));
            };
            let action = template[start + 2..end].trim();
            cursor = end + 2;
            if let Some(cond) = action.strip_prefix("if ") {
                // Find matching {{ else }} or {{ end }}
                let (true_body, false_body, after) = split_if(template, cursor)?;
                let truthy = is_truthy(&resolve(ctx, cond.trim()));
                let body = if truthy { true_body } else { false_body };
                out.push_str(&render_go_template(&body, ctx)?);
                cursor = after;
                continue;
            }
            if let Some(loop_expr) = action.strip_prefix("range ") {
                let (body, after) = split_range(template, cursor)?;
                let list = resolve(ctx, loop_expr.trim());
                if let Value::Array(arr) = list {
                    for item in arr {
                        out.push_str(&render_go_template(&body, &item)?);
                    }
                }
                cursor = after;
                continue;
            }
            if action == "end" || action == "else" {
                return Err(format!("stray `{}` at byte {}", action, start));
            }
            // simple substitution
            let v = resolve(ctx, action);
            out.push_str(&value_to_string(&v));
        } else {
            out.push_str(&template[cursor..]);
            break;
        }
    }
    Ok(out)
}

fn find_open(s: &str, from: usize) -> Option<usize> {
    s[from..].find("{{").map(|i| i + from)
}

fn find_close(s: &str, from: usize) -> Option<usize> {
    s[from..].find("}}").map(|i| i + from)
}

fn split_if(s: &str, from: usize) -> Result<(String, String, usize), String> {
    let (consumed, mut depth) = (from, 1usize);
    let mut cursor = consumed;
    let mut true_body = String::new();
    let mut false_body = String::new();
    let mut in_else = false;
    while let Some(o) = find_open(s, cursor) {
        if let Some(c) = find_close(s, o + 2) {
            let pre = &s[cursor..o];
            if in_else {
                false_body.push_str(pre);
            } else {
                true_body.push_str(pre);
            }
            let action = s[o + 2..c].trim();
            if action.starts_with("if ") {
                depth += 1;
                if in_else {
                    false_body.push_str(&s[o..c + 2]);
                } else {
                    true_body.push_str(&s[o..c + 2]);
                }
            } else if action == "end" {
                depth -= 1;
                if depth == 0 {
                    return Ok((true_body, false_body, c + 2));
                }
                if in_else {
                    false_body.push_str(&s[o..c + 2]);
                } else {
                    true_body.push_str(&s[o..c + 2]);
                }
            } else if action == "else" && depth == 1 {
                in_else = true;
            } else {
                if in_else {
                    false_body.push_str(&s[o..c + 2]);
                } else {
                    true_body.push_str(&s[o..c + 2]);
                }
            }
            cursor = c + 2;
        } else {
            return Err("unterminated action inside if".to_string());
        }
    }
    Err("unterminated {{ if ... }} block".to_string())
}

fn split_range(s: &str, from: usize) -> Result<(String, usize), String> {
    let mut depth = 1usize;
    let mut cursor = from;
    let mut body = String::new();
    while let Some(o) = find_open(s, cursor) {
        if let Some(c) = find_close(s, o + 2) {
            let pre = &s[cursor..o];
            body.push_str(pre);
            let action = s[o + 2..c].trim();
            if action.starts_with("range ") || action.starts_with("if ") {
                depth += 1;
                body.push_str(&s[o..c + 2]);
            } else if action == "end" {
                depth -= 1;
                if depth == 0 {
                    return Ok((body, c + 2));
                }
                body.push_str(&s[o..c + 2]);
            } else {
                body.push_str(&s[o..c + 2]);
            }
            cursor = c + 2;
        } else {
            return Err("unterminated action inside range".to_string());
        }
    }
    Err("unterminated {{ range ... }} block".to_string())
}

fn resolve(ctx: &Value, expr: &str) -> Value {
    let e = expr.trim();
    if e == "." {
        return ctx.clone();
    }
    let path = e.strip_prefix('.').unwrap_or(e);
    if path.is_empty() {
        return ctx.clone();
    }
    let mut cur = ctx;
    for seg in path.split('.') {
        match cur {
            Value::Object(m) => match m.get(seg) {
                Some(v) => cur = v,
                None => return Value::Null,
            },
            _ => return Value::Null,
        }
    }
    cur.clone()
}

fn is_truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64().unwrap_or(0.0) != 0.0,
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plain_text_passthrough() {
        assert_eq!(render_go_template("hello", &json!({})).unwrap(), "hello");
    }

    #[test]
    fn substitution() {
        assert_eq!(
            render_go_template("hi {{ .name }}", &json!({"name":"alice"})).unwrap(),
            "hi alice"
        );
    }

    #[test]
    fn nested_path() {
        assert_eq!(
            render_go_template("{{ .spec.size }}", &json!({"spec":{"size":10}})).unwrap(),
            "10"
        );
    }

    #[test]
    fn missing_path_empty() {
        assert_eq!(
            render_go_template("[{{ .x }}]", &json!({})).unwrap(),
            "[]"
        );
    }

    #[test]
    fn if_truthy() {
        let r = render_go_template(
            "{{ if .active }}YES{{ else }}NO{{ end }}",
            &json!({"active": true}),
        )
        .unwrap();
        assert_eq!(r, "YES");
    }

    #[test]
    fn if_falsy() {
        let r = render_go_template(
            "{{ if .active }}YES{{ else }}NO{{ end }}",
            &json!({"active": false}),
        )
        .unwrap();
        assert_eq!(r, "NO");
    }

    #[test]
    fn if_no_else_truthy() {
        let r =
            render_go_template("{{ if .x }}HIT{{ end }}", &json!({"x": "y"})).unwrap();
        assert_eq!(r, "HIT");
    }

    #[test]
    fn range_array() {
        let r =
            render_go_template("{{ range .l }}[{{ . }}]{{ end }}", &json!({"l":["a","b","c"]}))
                .unwrap();
        assert_eq!(r, "[a][b][c]");
    }

    #[test]
    fn range_empty_array() {
        let r = render_go_template("{{ range .l }}X{{ end }}", &json!({"l":[]})).unwrap();
        assert_eq!(r, "");
    }

    #[test]
    fn unterminated_action_errors() {
        assert!(render_go_template("{{ .x", &json!({})).is_err());
    }

    #[test]
    fn stray_end_errors() {
        assert!(render_go_template("{{ end }}", &json!({})).is_err());
    }

    #[test]
    fn nested_if_in_range() {
        let r = render_go_template(
            "{{ range .l }}{{ if . }}T{{ else }}F{{ end }}{{ end }}",
            &json!({"l":[true, false, true]}),
        )
        .unwrap();
        assert_eq!(r, "TFT");
    }

    #[test]
    fn truthy_helper_cases() {
        assert!(!is_truthy(&json!(null)));
        assert!(!is_truthy(&json!("")));
        assert!(!is_truthy(&json!(0)));
        assert!(is_truthy(&json!("x")));
        assert!(is_truthy(&json!(1)));
        assert!(is_truthy(&json!([1])));
        assert!(is_truthy(&json!({"k":"v"})));
    }
}
