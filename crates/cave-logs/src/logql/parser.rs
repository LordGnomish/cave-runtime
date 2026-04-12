//! Recursive-descent LogQL parser.
//!
//! Grammar (simplified):
//! ```text
//! expr          = metric_expr | log_expr
//! log_expr      = stream_selector pipeline_stage*
//! stream_selector = "{" (label_matcher ("," label_matcher)*)? "}"
//! label_matcher = IDENT op string
//! op            = "=" | "!=" | "=~" | "!~"
//! pipeline_stage = filter_stage | parser_stage | label_filter | line_format | label_fmt
//! filter_stage  = ("|=" | "!=" | "|~" | "!~") STRING
//! parser_stage  = "|" ("json" | "logfmt" | "regexp" STRING | "pattern" STRING | "unpack")
//! label_filter  = "|" IDENT cmp_op value
//! line_format   = "|" "line_format" STRING
//! label_fmt     = "|" "label_format" rename_spec ("," rename_spec)*
//! rename_spec   = IDENT "=" IDENT
//! metric_expr   = range_agg | vector_agg
//! range_agg     = range_agg_op "(" log_expr "[" duration "]" ")" grouping?
//! vector_agg    = vector_agg_op "(" [param ","] metric_expr ")" grouping?
//! grouping      = ("by" | "without") "(" label_list ")"
//! ```

use super::{
    ast::*,
    lexer::{Lexer, Token},
};
use crate::models::{LabelMatcher, MatchOp};

pub fn parse(input: &str) -> Result<Expr, String> {
    let tokens = Lexer::new(input).tokenize()?;
    let mut p = Parser { tokens, pos: 0 };
    let expr = p.parse_expr()?;
    if !p.at_eof() {
        return Err(format!("unexpected token after expression: {:?}", p.peek()));
    }
    Ok(expr)
}

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

// ─── Token helpers ────────────────────────────────────────────────────────────

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.pos]
    }

    fn peek2(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1)
    }

    fn advance(&mut self) -> &Token {
        let t = &self.tokens[self.pos];
        if self.pos + 1 < self.tokens.len() {
            self.pos += 1;
        }
        t
    }

    fn at_eof(&self) -> bool {
        matches!(self.peek(), Token::Eof)
    }

    fn expect(&mut self, expected: &Token) -> Result<(), String> {
        if self.peek() == expected {
            self.advance();
            Ok(())
        } else {
            Err(format!("expected {:?}, got {:?}", expected, self.peek()))
        }
    }

    fn expect_ident(&mut self) -> Result<String, String> {
        match self.advance().clone() {
            Token::Ident(s) => Ok(s),
            t => Err(format!("expected identifier, got {t:?}")),
        }
    }

    fn expect_str(&mut self) -> Result<String, String> {
        match self.advance().clone() {
            Token::Str(s) => Ok(s),
            t => Err(format!("expected string literal, got {t:?}")),
        }
    }

    fn expect_dur(&mut self) -> Result<std::time::Duration, String> {
        match self.advance().clone() {
            Token::Dur(d) => Ok(d),
            t => Err(format!("expected duration (e.g. 5m), got {t:?}")),
        }
    }
}

// ─── Top-level ────────────────────────────────────────────────────────────────

impl Parser {
    fn parse_expr(&mut self) -> Result<Expr, String> {
        // If the next ident is a vector aggregation operator, parse metric.
        if let Token::Ident(name) = self.peek() {
            if is_vector_agg_op(name) {
                return Ok(Expr::Metric(MetricExpr::VectorAgg(self.parse_vector_agg()?)));
            }
            if is_range_agg_op(name) {
                return Ok(Expr::Metric(MetricExpr::RangeAgg(self.parse_range_agg()?)));
            }
        }
        Ok(Expr::Log(self.parse_log_stream()?))
    }

    // ── Log stream ────────────────────────────────────────────────────────────

    fn parse_log_stream(&mut self) -> Result<LogStreamExpr, String> {
        let matchers = self.parse_stream_selector()?;
        let mut pipeline = vec![];
        loop {
            match self.peek() {
                Token::PipeEq | Token::PipeTilde => {
                    pipeline.push(self.parse_pipe_filter()?);
                }
                Token::Ne | Token::NRe => {
                    pipeline.push(self.parse_bang_filter()?);
                }
                Token::Pipe => {
                    // Could be parser stage, label filter, line_format, label_format, decolorize
                    let stage = self.parse_pipe_stage()?;
                    pipeline.push(stage);
                }
                _ => break,
            }
        }
        Ok(LogStreamExpr { matchers, pipeline })
    }

    fn parse_stream_selector(&mut self) -> Result<Vec<LabelMatcher>, String> {
        self.expect(&Token::LBrace)?;
        let mut matchers = vec![];
        loop {
            match self.peek() {
                Token::RBrace | Token::Eof => break,
                _ => {
                    matchers.push(self.parse_label_matcher()?);
                    if self.peek() == &Token::Comma {
                        self.advance();
                    }
                }
            }
        }
        self.expect(&Token::RBrace)?;
        Ok(matchers)
    }

    fn parse_label_matcher(&mut self) -> Result<LabelMatcher, String> {
        let name = self.expect_ident()?;
        let op = match self.advance().clone() {
            Token::Eq => MatchOp::Eq,
            Token::Ne => MatchOp::Ne,
            Token::Re => MatchOp::Re,
            Token::NRe => MatchOp::NRe,
            t => return Err(format!("expected label matcher op, got {t:?}")),
        };
        let value = self.expect_str()?;
        Ok(LabelMatcher { name, op, value })
    }

    // ── Pipeline stages ───────────────────────────────────────────────────────

    fn parse_pipe_filter(&mut self) -> Result<PipelineStage, String> {
        let op = match self.advance().clone() {
            Token::PipeEq => FilterOp::Contains,
            Token::PipeTilde => FilterOp::Re,
            t => return Err(format!("expected |= or |~, got {t:?}")),
        };
        let value = self.expect_str()?;
        Ok(PipelineStage::Filter(FilterStage { op, value }))
    }

    fn parse_bang_filter(&mut self) -> Result<PipelineStage, String> {
        let op = match self.advance().clone() {
            Token::Ne => FilterOp::NotContains,
            Token::NRe => FilterOp::NotRe,
            t => return Err(format!("expected != or !~, got {t:?}")),
        };
        let value = self.expect_str()?;
        Ok(PipelineStage::Filter(FilterStage { op, value }))
    }

    fn parse_pipe_stage(&mut self) -> Result<PipelineStage, String> {
        self.expect(&Token::Pipe)?;
        match self.peek().clone() {
            Token::Ident(name) => {
                match name.as_str() {
                    "json" => {
                        self.advance();
                        Ok(PipelineStage::Parser(ParserStage::Json))
                    }
                    "logfmt" => {
                        self.advance();
                        Ok(PipelineStage::Parser(ParserStage::Logfmt))
                    }
                    "regexp" => {
                        self.advance();
                        let re = self.expect_str()?;
                        Ok(PipelineStage::Parser(ParserStage::Regexp(re)))
                    }
                    "pattern" => {
                        self.advance();
                        let pat = self.expect_str()?;
                        Ok(PipelineStage::Parser(ParserStage::Pattern(pat)))
                    }
                    "unpack" => {
                        self.advance();
                        Ok(PipelineStage::Parser(ParserStage::Unpack))
                    }
                    "line_format" => {
                        self.advance();
                        let tmpl = self.expect_str()?;
                        Ok(PipelineStage::LineFormat(tmpl))
                    }
                    "label_format" => {
                        self.advance();
                        let pairs = self.parse_label_format_pairs()?;
                        Ok(PipelineStage::LabelFmt(pairs))
                    }
                    "decolorize" => {
                        self.advance();
                        Ok(PipelineStage::Decolorize)
                    }
                    _ => {
                        // label filter: | label_name op value
                        self.parse_label_filter_stage()
                    }
                }
            }
            t => Err(format!("expected pipeline keyword or label name, got {t:?}")),
        }
    }

    fn parse_label_format_pairs(&mut self) -> Result<Vec<(String, String)>, String> {
        let mut pairs = vec![];
        loop {
            let from = self.expect_ident()?;
            self.expect(&Token::Eq)?;
            let to = self.expect_ident()?;
            pairs.push((from, to));
            if self.peek() == &Token::Comma {
                self.advance();
            } else {
                break;
            }
        }
        Ok(pairs)
    }

    fn parse_label_filter_stage(&mut self) -> Result<PipelineStage, String> {
        let label = self.expect_ident()?;
        let op = self.parse_label_filter_op()?;
        let value = self.parse_label_filter_value()?;
        Ok(PipelineStage::LabelFilter(LabelFilterStage { label, op, value }))
    }

    fn parse_label_filter_op(&mut self) -> Result<LabelFilterOp, String> {
        match self.advance().clone() {
            Token::EqEq | Token::Eq => Ok(LabelFilterOp::Eq),
            Token::Ne => Ok(LabelFilterOp::Ne),
            Token::Gt => Ok(LabelFilterOp::Gt),
            Token::Gte => Ok(LabelFilterOp::Gte),
            Token::Lt => Ok(LabelFilterOp::Lt),
            Token::Lte => Ok(LabelFilterOp::Lte),
            Token::Re => Ok(LabelFilterOp::Re),
            Token::NRe => Ok(LabelFilterOp::NRe),
            t => Err(format!("expected comparison operator, got {t:?}")),
        }
    }

    fn parse_label_filter_value(&mut self) -> Result<LabelFilterValue, String> {
        match self.peek().clone() {
            Token::Str(s) => { self.advance(); Ok(LabelFilterValue::String(s)) }
            Token::Integer(n) => { self.advance(); Ok(LabelFilterValue::Float(n as f64)) }
            Token::Float(f) => { self.advance(); Ok(LabelFilterValue::Float(f)) }
            Token::Dur(d) => { self.advance(); Ok(LabelFilterValue::Duration(d)) }
            t => Err(format!("expected filter value, got {t:?}")),
        }
    }

    // ── Metric expressions ────────────────────────────────────────────────────

    fn parse_range_agg(&mut self) -> Result<RangeAggExpr, String> {
        let op_name = self.expect_ident()?;
        let op = parse_range_agg_op(&op_name)?;
        self.expect(&Token::LParen)?;
        let stream = self.parse_log_stream()?;
        self.expect(&Token::LBracket)?;
        let range = self.expect_dur()?;
        self.expect(&Token::RBracket)?;
        self.expect(&Token::RParen)?;
        let grouping = self.try_parse_grouping()?;
        Ok(RangeAggExpr { op, stream, range, grouping })
    }

    fn parse_vector_agg(&mut self) -> Result<VectorAggExpr, String> {
        let op_name = self.expect_ident()?;
        let op = parse_vector_agg_op(&op_name)?;

        // Grouping may appear BEFORE or AFTER the parenthesised inner expression:
        //   sum by (labels) (inner)   ← before
        //   sum(inner) by (labels)    ← after
        let pre_grouping = self.try_parse_grouping()?;

        let is_topk_like = matches!(op, VectorAggOp::Topk | VectorAggOp::Bottomk);
        self.expect(&Token::LParen)?;

        let param = if is_topk_like {
            if let Token::Integer(n) = self.peek().clone() {
                self.advance();
                self.expect(&Token::Comma)?;
                Some(n as u64)
            } else {
                None
            }
        } else {
            None
        };

        let inner = self.parse_metric_inner()?;
        self.expect(&Token::RParen)?;
        let post_grouping = self.try_parse_grouping()?;
        let grouping = pre_grouping.or(post_grouping);
        Ok(VectorAggExpr { op, expr: Box::new(inner), grouping, param })
    }

    fn parse_metric_inner(&mut self) -> Result<MetricExpr, String> {
        if let Token::Ident(name) = self.peek() {
            let n = name.clone();
            if is_range_agg_op(&n) {
                return Ok(MetricExpr::RangeAgg(self.parse_range_agg()?));
            }
            if is_vector_agg_op(&n) {
                return Ok(MetricExpr::VectorAgg(self.parse_vector_agg()?));
            }
        }
        Err(format!("expected metric expression, got {:?}", self.peek()))
    }

    fn try_parse_grouping(&mut self) -> Result<Option<Grouping>, String> {
        match self.peek() {
            Token::Ident(s) if s == "by" || s == "without" => {
                let by = self.expect_ident()? == "by";
                self.expect(&Token::LParen)?;
                let mut labels = vec![];
                loop {
                    match self.peek() {
                        Token::RParen | Token::Eof => break,
                        _ => {
                            labels.push(self.expect_ident()?);
                            if self.peek() == &Token::Comma {
                                self.advance();
                            }
                        }
                    }
                }
                self.expect(&Token::RParen)?;
                Ok(Some(Grouping { by, labels }))
            }
            _ => Ok(None),
        }
    }
}

// ─── Operator classification helpers ─────────────────────────────────────────

fn is_range_agg_op(s: &str) -> bool {
    matches!(
        s,
        "rate" | "count_over_time" | "bytes_over_time" | "bytes_rate"
            | "sum_over_time" | "avg_over_time" | "max_over_time" | "min_over_time"
            | "first_over_time" | "last_over_time" | "quantile_over_time"
    )
}

fn is_vector_agg_op(s: &str) -> bool {
    matches!(s, "sum" | "avg" | "max" | "min" | "count" | "topk" | "bottomk")
}

fn parse_range_agg_op(s: &str) -> Result<RangeAggOp, String> {
    match s {
        "rate" => Ok(RangeAggOp::Rate),
        "count_over_time" => Ok(RangeAggOp::CountOverTime),
        "bytes_over_time" => Ok(RangeAggOp::BytesOverTime),
        "bytes_rate" => Ok(RangeAggOp::BytesRate),
        "sum_over_time" => Ok(RangeAggOp::SumOverTime("__line__".into())),
        "avg_over_time" => Ok(RangeAggOp::AvgOverTime("__line__".into())),
        "max_over_time" => Ok(RangeAggOp::MaxOverTime("__line__".into())),
        "min_over_time" => Ok(RangeAggOp::MinOverTime("__line__".into())),
        "first_over_time" => Ok(RangeAggOp::FirstOverTime("__line__".into())),
        "last_over_time" => Ok(RangeAggOp::LastOverTime("__line__".into())),
        "quantile_over_time" => Ok(RangeAggOp::QuantileOverTime("__line__".into(), 50)),
        other => Err(format!("unknown range aggregation: {other}")),
    }
}

fn parse_vector_agg_op(s: &str) -> Result<VectorAggOp, String> {
    match s {
        "sum" => Ok(VectorAggOp::Sum),
        "avg" => Ok(VectorAggOp::Avg),
        "max" => Ok(VectorAggOp::Max),
        "min" => Ok(VectorAggOp::Min),
        "count" => Ok(VectorAggOp::Count),
        "topk" => Ok(VectorAggOp::Topk),
        "bottomk" => Ok(VectorAggOp::Bottomk),
        other => Err(format!("unknown vector aggregation: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::logql::ast::{Expr, FilterOp, ParserStage, PipelineStage};

    fn parse_ok(s: &str) -> Expr {
        parse(s).unwrap_or_else(|e| panic!("parse failed: {e}"))
    }

    #[test]
    fn parse_stream_selector() {
        let e = parse_ok(r#"{app="foo"}"#);
        let Expr::Log(ls) = e else { panic!("expected log stream") };
        assert_eq!(ls.matchers.len(), 1);
        assert_eq!(ls.matchers[0].name, "app");
    }

    #[test]
    fn parse_multiple_matchers() {
        let e = parse_ok(r#"{app="foo", env=~"prod|staging", region!="us-west"}"#);
        let Expr::Log(ls) = e else { panic!() };
        assert_eq!(ls.matchers.len(), 3);
    }

    #[test]
    fn parse_filter_contains() {
        let e = parse_ok(r#"{app="x"} |= "error""#);
        let Expr::Log(ls) = e else { panic!() };
        assert_eq!(ls.pipeline.len(), 1);
        let PipelineStage::Filter(f) = &ls.pipeline[0] else { panic!() };
        assert_eq!(f.op, FilterOp::Contains);
        assert_eq!(f.value, "error");
    }

    #[test]
    fn parse_filter_not_contains() {
        let e = parse_ok(r#"{app="x"} != "timeout""#);
        let Expr::Log(ls) = e else { panic!() };
        let PipelineStage::Filter(f) = &ls.pipeline[0] else { panic!() };
        assert_eq!(f.op, FilterOp::NotContains);
    }

    #[test]
    fn parse_filter_regex() {
        let e = parse_ok(r#"{app="x"} |~ "err.*" !~ "debug.*""#);
        let Expr::Log(ls) = e else { panic!() };
        assert_eq!(ls.pipeline.len(), 2);
        let PipelineStage::Filter(f0) = &ls.pipeline[0] else { panic!() };
        assert_eq!(f0.op, FilterOp::Re);
        let PipelineStage::Filter(f1) = &ls.pipeline[1] else { panic!() };
        assert_eq!(f1.op, FilterOp::NotRe);
    }

    #[test]
    fn parse_parser_json() {
        let e = parse_ok(r#"{app="x"} | json"#);
        let Expr::Log(ls) = e else { panic!() };
        let PipelineStage::Parser(ParserStage::Json) = &ls.pipeline[0] else { panic!() };
    }

    #[test]
    fn parse_parser_logfmt() {
        let e = parse_ok(r#"{app="x"} | logfmt"#);
        let Expr::Log(ls) = e else { panic!() };
        let PipelineStage::Parser(ParserStage::Logfmt) = &ls.pipeline[0] else { panic!() };
    }

    #[test]
    fn parse_parser_regexp() {
        let e = parse_ok(r#"{app="x"} | regexp "(?P<status>[0-9]+)""#);
        let Expr::Log(ls) = e else { panic!() };
        let PipelineStage::Parser(ParserStage::Regexp(re)) = &ls.pipeline[0] else { panic!() };
        assert!(re.contains("status"));
    }

    #[test]
    fn parse_label_filter() {
        let e = parse_ok(r#"{app="x"} | json | status >= 400"#);
        let Expr::Log(ls) = e else { panic!() };
        assert_eq!(ls.pipeline.len(), 2);
        let PipelineStage::LabelFilter(lf) = &ls.pipeline[1] else { panic!() };
        assert_eq!(lf.label, "status");
        assert_eq!(lf.op, LabelFilterOp::Gte);
    }

    #[test]
    fn parse_line_format() {
        let e = parse_ok(r#"{app="x"} | line_format "{{.status}} {{.method}}""#);
        let Expr::Log(ls) = e else { panic!() };
        let PipelineStage::LineFormat(tmpl) = &ls.pipeline[0] else { panic!() };
        assert!(tmpl.contains("status"));
    }

    #[test]
    fn parse_rate() {
        let e = parse_ok(r#"rate({app="foo"}[5m])"#);
        let Expr::Metric(MetricExpr::RangeAgg(ra)) = e else { panic!() };
        assert_eq!(ra.op, RangeAggOp::Rate);
        assert_eq!(ra.range.as_secs(), 300);
    }

    #[test]
    fn parse_count_over_time() {
        let e = parse_ok(r#"count_over_time({app="foo"}[1h])"#);
        let Expr::Metric(MetricExpr::RangeAgg(ra)) = e else { panic!() };
        assert_eq!(ra.op, RangeAggOp::CountOverTime);
        assert_eq!(ra.range.as_secs(), 3600);
    }

    #[test]
    fn parse_sum_by() {
        let e = parse_ok(r#"sum by (app) (count_over_time({job="test"}[5m]))"#);
        let Expr::Metric(MetricExpr::VectorAgg(va)) = e else { panic!() };
        assert_eq!(va.op, VectorAggOp::Sum);
        let g = va.grouping.as_ref().unwrap();
        assert!(g.by);
        assert_eq!(g.labels, vec!["app"]);
    }

    #[test]
    fn parse_topk() {
        let e = parse_ok(r#"topk(5, count_over_time({app="foo"}[1m]))"#);
        let Expr::Metric(MetricExpr::VectorAgg(va)) = e else { panic!() };
        assert_eq!(va.op, VectorAggOp::Topk);
        assert_eq!(va.param, Some(5));
    }
}
