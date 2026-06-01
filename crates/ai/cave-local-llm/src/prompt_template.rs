// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Prompt template engine — substitution-based, no Jinja runtime.
//!
//! Cite: ollama/ollama `docs/template.md` v0.3.0 — the template language is
//! a tiny mustache-style subset (`{{ var }}`, `{{ if cond }}` / `{{ end }}`,
//! `{{ range .List }}` / `{{ end }}`, `{{ .Field }}`). cave implements the
//! subset cave-runtime actually uses to render Qwen-amele prompts:
//!
//!   * `{{ var }}`   — bare variable substitution from a [`PromptContext`].
//!   * `{{ if x }}…{{ end }}` — emit the block only when `x` is truthy.
//!   * `{{ range items }}…{{ end }}` — loop over a list, binding `it`.
//!
//! For full Go-template parity we would need `text/template` semantics;
//! that's deliberately out of scope. The narrow subset is sufficient to
//! render every prompt template the daemon ships today.

use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum TemplateError {
    #[error("unbalanced template: {0}")]
    Unbalanced(String),
    #[error("unknown variable: {0}")]
    UnknownVariable(String),
    #[error("variable {0} is not a list (cannot range over it)")]
    NotAList(String),
}

pub type TemplateResult<T> = Result<T, TemplateError>;

/// A value bound to a template variable. cave keeps it dead-simple: a
/// scalar string OR a list of strings. Full nested JSON would need a
/// real template engine; we deliberately don't go there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TemplateValue {
    Scalar(String),
    List(Vec<String>),
    /// Bool/truthy marker — used by `{{ if x }}` to render the block.
    /// `Truthy(false)` is the "skip block" sentinel.
    Truthy(bool),
}

impl TemplateValue {
    pub fn is_truthy(&self) -> bool {
        match self {
            Self::Scalar(s) => !s.is_empty(),
            Self::List(v) => !v.is_empty(),
            Self::Truthy(b) => *b,
        }
    }

    pub fn as_scalar(&self) -> Option<&str> {
        match self {
            Self::Scalar(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[String]> {
        match self {
            Self::List(v) => Some(v.as_slice()),
            _ => None,
        }
    }
}

/// The variable binding map handed to `render`.
#[derive(Debug, Clone, Default)]
pub struct PromptContext {
    vars: HashMap<String, TemplateValue>,
}

impl PromptContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, name: impl Into<String>, v: TemplateValue) -> &mut Self {
        self.vars.insert(name.into(), v);
        self
    }

    pub fn set_scalar(&mut self, name: impl Into<String>, v: impl Into<String>) -> &mut Self {
        self.set(name, TemplateValue::Scalar(v.into()))
    }

    pub fn set_list<S: Into<String>>(
        &mut self,
        name: impl Into<String>,
        items: impl IntoIterator<Item = S>,
    ) -> &mut Self {
        self.set(
            name,
            TemplateValue::List(items.into_iter().map(Into::into).collect()),
        )
    }

    pub fn set_bool(&mut self, name: impl Into<String>, b: bool) -> &mut Self {
        self.set(name, TemplateValue::Truthy(b))
    }

    pub fn get(&self, name: &str) -> Option<&TemplateValue> {
        self.vars.get(name)
    }
}

/// Parse + render a template against the given context.
pub fn render(template: &str, ctx: &PromptContext) -> TemplateResult<String> {
    let normalized = apply_trim_markers(template)?;
    render_impl(&normalized, ctx, None)
}

/// Lexer-level whitespace-trim pass mirroring Go text/template. A `{{-`
/// trims all whitespace at the end of the text immediately preceding the
/// action; a `-}}` trims all whitespace at the start of the text immediately
/// following it. Cite ollama/ollama docs/template.md (every shipped template
/// uses `{{-` / `-}}` to keep indented if/range blocks tidy). The pass also
/// re-emits each action canonically as `{{ inner }}`, dash-free, so the
/// downstream [`render_impl`] never sees a trim marker.
fn apply_trim_markers(t: &str) -> TemplateResult<String> {
    let mut out = String::with_capacity(t.len());
    let mut rest = t;
    let mut trim_next_ws = false;
    while !rest.is_empty() {
        match rest.find("{{") {
            Some(open) => {
                let mut lit = &rest[..open];
                if trim_next_ws {
                    lit = lit.trim_start();
                }
                out.push_str(lit);

                let body = &rest[open + 2..];
                let close = body
                    .find("}}")
                    .ok_or_else(|| TemplateError::Unbalanced("missing '}}'".into()))?;
                let raw = body[..close].trim();
                let (trim_left, raw) = match raw.strip_prefix('-') {
                    Some(r) => (true, r),
                    None => (false, raw),
                };
                let (trim_right, raw) = match raw.strip_suffix('-') {
                    Some(r) => (true, r),
                    None => (false, raw),
                };
                if trim_left {
                    let kept = out.trim_end_matches(|c: char| c.is_whitespace()).len();
                    out.truncate(kept);
                }
                out.push_str("{{ ");
                out.push_str(raw.trim());
                out.push_str(" }}");
                trim_next_ws = trim_right;
                rest = &body[close + 2..];
            }
            None => {
                let mut lit = rest;
                if trim_next_ws {
                    lit = lit.trim_start();
                }
                out.push_str(lit);
                break;
            }
        }
    }
    Ok(out)
}

fn render_impl(
    template: &str,
    ctx: &PromptContext,
    range_binding: Option<(&str, &str)>,
) -> TemplateResult<String> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Locate `}}`
            let close = template[i + 2..]
                .find("}}")
                .ok_or_else(|| TemplateError::Unbalanced("missing '}}'".into()))?;
            let directive = template[i + 2..i + 2 + close].trim();
            i = i + 2 + close + 2;

            if let Some(cond_name) = directive.strip_prefix("if ") {
                let cond_name = cond_name.trim();
                let (else_at, end_pos) = find_else_and_end(&template[i..])?;
                let (then_body, else_body): (&str, &str) = match else_at {
                    Some((else_pos, else_len)) => (
                        &template[i..i + else_pos],
                        &template[i + else_pos + else_len..i + end_pos],
                    ),
                    None => (&template[i..i + end_pos], ""),
                };
                let cond = lookup(ctx, range_binding, cond_name)?;
                let branch = if cond.is_truthy() { then_body } else { else_body };
                out.push_str(&render_impl(branch, ctx, range_binding)?);
                i += end_pos + "{{ end }}".len();
                continue;
            }
            if let Some(loop_name) = directive.strip_prefix("range ") {
                let loop_name = loop_name.trim();
                let block_end = find_matching_end(&template[i..])?;
                let body = &template[i..i + block_end];
                let v = lookup(ctx, range_binding, loop_name)?;
                let items = v
                    .as_list()
                    .ok_or_else(|| TemplateError::NotAList(loop_name.to_string()))?;
                for item in items {
                    out.push_str(&render_impl(body, ctx, Some(("it", item)))?);
                }
                i += block_end + "{{ end }}".len();
                continue;
            }
            if directive == "end" {
                return Err(TemplateError::Unbalanced("stray '{{ end }}'".into()));
            }
            if directive == "else" {
                return Err(TemplateError::Unbalanced("stray '{{ else }}'".into()));
            }

            // Bare variable substitution
            let v = lookup(ctx, range_binding, directive)?;
            match v {
                TemplateValue::Scalar(s) => out.push_str(&s),
                TemplateValue::List(_) => {
                    return Err(TemplateError::NotAList(format!(
                        "cannot substitute list '{directive}' as bare scalar"
                    )));
                }
                TemplateValue::Truthy(b) => out.push_str(if b { "true" } else { "false" }),
            }
            continue;
        }
        // Literal char
        let ch = bytes[i] as char;
        out.push(ch);
        i += 1;
    }
    Ok(out)
}

fn lookup(
    ctx: &PromptContext,
    range_binding: Option<(&str, &str)>,
    name: &str,
) -> TemplateResult<TemplateValue> {
    if let Some((bind_name, bind_value)) = range_binding {
        if name == bind_name {
            return Ok(TemplateValue::Scalar(bind_value.to_string()));
        }
    }
    ctx.get(name)
        .cloned()
        .ok_or_else(|| TemplateError::UnknownVariable(name.to_string()))
}

/// Like [`find_matching_end`], but for an `{{ if }}` block: also reports the
/// depth-1 `{{ else }}` separating the then/else branches, if present. Returns
/// `(else, end_pos)` where `else` is `(offset, token_len)` of the `{{` opening
/// the `{{ else }}` directive and `end_pos` is the `{{` offset of the closing
/// `{{ end }}`. A nested inner `{{ else }}` (depth > 1) is ignored so the
/// branch binds to the innermost enclosing `if`, matching Go text/template.
fn find_else_and_end(s: &str) -> TemplateResult<(Option<(usize, usize)>, usize)> {
    let mut depth = 1i32;
    let mut i = 0usize;
    let mut else_at: Option<(usize, usize)> = None;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let close = s[i + 2..]
                .find("}}")
                .ok_or_else(|| TemplateError::Unbalanced("missing '}}'".into()))?;
            let directive = s[i + 2..i + 2 + close].trim();
            let token_len = (i + 2 + close + 2) - i;
            if directive.starts_with("if ") || directive.starts_with("range ") {
                depth += 1;
            } else if directive == "end" {
                depth -= 1;
                if depth == 0 {
                    return Ok((else_at, i));
                }
            } else if directive == "else" && depth == 1 && else_at.is_none() {
                else_at = Some((i, token_len));
            }
            i = i + 2 + close + 2;
        } else {
            i += 1;
        }
    }
    Err(TemplateError::Unbalanced("missing '{{ end }}'".into()))
}

/// Walk from the start of `s` finding the `{{ end }}` that closes the
/// current block (nested `{{ if }}` / `{{ range }}` increment depth).
fn find_matching_end(s: &str) -> TemplateResult<usize> {
    let mut depth = 1i32;
    let mut i = 0usize;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            let close = s[i + 2..]
                .find("}}")
                .ok_or_else(|| TemplateError::Unbalanced("missing '}}'".into()))?;
            let directive = s[i + 2..i + 2 + close].trim();
            if directive.starts_with("if ") || directive.starts_with("range ") {
                depth += 1;
            } else if directive == "end" {
                depth -= 1;
                if depth == 0 {
                    return Ok(i);
                }
            }
            i = i + 2 + close + 2;
        } else {
            i += 1;
        }
    }
    Err(TemplateError::Unbalanced("missing '{{ end }}'".into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn substitutes_bare_variable() {
        let mut ctx = PromptContext::new();
        ctx.set_scalar("name", "qwen");
        assert_eq!(render("hello {{ name }}!", &ctx).unwrap(), "hello qwen!");
    }

    #[test]
    fn unknown_variable_errors() {
        let ctx = PromptContext::new();
        assert!(matches!(
            render("{{ x }}", &ctx),
            Err(TemplateError::UnknownVariable(_))
        ));
    }

    #[test]
    fn if_block_renders_when_truthy() {
        let mut ctx = PromptContext::new();
        ctx.set_bool("flag", true);
        ctx.set_scalar("msg", "yes");
        assert_eq!(
            render("{{ if flag }}{{ msg }}{{ end }}", &ctx).unwrap(),
            "yes"
        );
    }

    #[test]
    fn if_block_skipped_when_falsy() {
        let mut ctx = PromptContext::new();
        ctx.set_bool("flag", false);
        ctx.set_scalar("msg", "no");
        assert_eq!(
            render("[{{ if flag }}{{ msg }}{{ end }}]", &ctx).unwrap(),
            "[]"
        );
    }

    #[test]
    fn range_emits_per_item() {
        let mut ctx = PromptContext::new();
        ctx.set_list("items", ["a", "b", "c"]);
        let out = render("{{ range items }}{{ it }};{{ end }}", &ctx).unwrap();
        assert_eq!(out, "a;b;c;");
    }

    #[test]
    fn nested_if_inside_range() {
        let mut ctx = PromptContext::new();
        ctx.set_list("items", ["a", "b"]);
        ctx.set_bool("flag", true);
        let out =
            render("{{ range items }}({{ if flag }}{{ it }}{{ end }}){{ end }}", &ctx).unwrap();
        assert_eq!(out, "(a)(b)");
    }

    #[test]
    fn range_on_non_list_errors() {
        let mut ctx = PromptContext::new();
        ctx.set_scalar("items", "oops");
        assert!(matches!(
            render("{{ range items }}{{ it }}{{ end }}", &ctx),
            Err(TemplateError::NotAList(_))
        ));
    }

    #[test]
    fn unbalanced_block_errors() {
        let ctx = PromptContext::new();
        assert!(matches!(
            render("{{ if x }}", &ctx),
            Err(TemplateError::Unbalanced(_))
        ));
    }

    #[test]
    fn list_value_truthy_when_nonempty() {
        assert!(TemplateValue::List(vec!["a".into()]).is_truthy());
        assert!(!TemplateValue::List(vec![]).is_truthy());
    }

    #[test]
    fn scalar_truthy_when_nonempty() {
        assert!(TemplateValue::Scalar("x".into()).is_truthy());
        assert!(!TemplateValue::Scalar("".into()).is_truthy());
    }

    #[test]
    fn truthy_bool_value() {
        let mut ctx = PromptContext::new();
        ctx.set_bool("on", true);
        ctx.set_bool("off", false);
        assert_eq!(render("{{ on }}/{{ off }}", &ctx).unwrap(), "true/false");
    }

    #[test]
    fn if_else_renders_then_branch_when_truthy() {
        let mut ctx = PromptContext::new();
        ctx.set_bool("flag", true);
        assert_eq!(
            render("{{ if flag }}yes{{ else }}no{{ end }}", &ctx).unwrap(),
            "yes"
        );
    }

    #[test]
    fn if_else_renders_else_branch_when_falsy() {
        let mut ctx = PromptContext::new();
        ctx.set_bool("flag", false);
        assert_eq!(
            render("{{ if flag }}yes{{ else }}no{{ end }}", &ctx).unwrap(),
            "no"
        );
    }

    #[test]
    fn nested_if_with_else_picks_inner_else() {
        // The {{ else }} must bind to the *inner* if at depth 1, not the outer.
        let mut ctx = PromptContext::new();
        ctx.set_bool("outer", true);
        ctx.set_bool("inner", false);
        let out = render(
            "{{ if outer }}A{{ if inner }}B{{ else }}C{{ end }}D{{ else }}E{{ end }}",
            &ctx,
        )
        .unwrap();
        assert_eq!(out, "ACD");
    }

    #[test]
    fn else_outside_if_errors() {
        let ctx = PromptContext::new();
        assert!(matches!(
            render("{{ else }}", &ctx),
            Err(TemplateError::Unbalanced(_))
        ));
    }

    #[test]
    fn trim_right_marker_strips_following_whitespace() {
        let mut ctx = PromptContext::new();
        ctx.set_scalar("x", "A");
        assert_eq!(render("{{ x -}}   \n   B", &ctx).unwrap(), "AB");
    }

    #[test]
    fn trim_left_marker_strips_preceding_whitespace() {
        let mut ctx = PromptContext::new();
        ctx.set_scalar("x", "B");
        assert_eq!(render("A   \n  {{- x }}", &ctx).unwrap(), "AB");
    }

    #[test]
    fn trim_markers_in_if_block_ollama_pattern() {
        // Mirrors ollama docs/template.md: an indented {{- if }} with a
        // trailing {{- end }} so the rendered prompt has no stray blank lines.
        let mut ctx = PromptContext::new();
        ctx.set_scalar("system", "SYS");
        let tmpl = "{{- if system }}\n  {{ system }}\n{{- end }}";
        assert_eq!(render(tmpl, &ctx).unwrap(), "\n  SYS");
    }

    #[test]
    fn trim_both_markers_collapse_to_empty_when_falsy() {
        let mut ctx = PromptContext::new();
        ctx.set_bool("system", false);
        let tmpl = "X\n{{- if system }}\n  {{ system }}\n{{- end -}}\n  Y";
        assert_eq!(render(tmpl, &ctx).unwrap(), "XY");
    }
}
