// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LogQL recursive-descent parser.
//! Converts a token stream (from `lexer`) into a `Query` AST node.

use std::time::Duration;

use super::ast::{
    self, BinOp, BinaryExpr, CompareOp, Decolorize, DropKeepLabel, DropLabels, Grouping,
    KeepLabels, LabelFilter, LabelFilterValue, LabelFormat, LabelMatcher, LineFilter, LineFormat,
    LogQuery, LogRangeAggregation, MatchCardinality, MatchOp, MetricQuery, PipelineStage, Query,
    RangeAgg, StreamSelector, UnwrapExpr, VectorAgg, VectorAggregation, VectorMatchGrouping,
};
use super::lexer::{Lexer, Token};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("lex error: {0}")]
    Lex(String),
    #[error("unexpected token {got:?}, expected {expected}")]
    Unexpected { got: String, expected: String },
    #[error("unexpected end of input, expected {0}")]
    Eof(String),
    #[error("{0}")]
    Other(String),
}

pub struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>) -> Self {
        Self { tokens, pos: 0 }
    }

    // ── public API ──────────────────────────────────────────────────────────

    /// Parse a full LogQL query string.
    pub fn parse_query(input: &str) -> Result<Query, ParseError> {
        let tokens = Lexer::new(input)
            .tokenize()
            .map_err(|e| ParseError::Lex(e.to_string()))?;
        let mut parser = Self::new(tokens);
        parser.parse()
    }

    // ── helpers ─────────────────────────────────────────────────────────────

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn peek2(&self) -> Option<&Token> {
        self.tokens.get(self.pos + 1)
    }

    fn advance(&mut self) -> Option<&Token> {
        let t = self.tokens.get(self.pos);
        self.pos += 1;
        t
    }

    fn expect(&mut self, expected: &Token) -> Result<(), ParseError> {
        match self.advance() {
            Some(t) if t == expected => Ok(()),
            Some(t) => Err(ParseError::Unexpected {
                got: format!("{:?}", t),
                expected: format!("{:?}", expected),
            }),
            None => Err(ParseError::Eof(format!("{:?}", expected))),
        }
    }

    fn expect_ident(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Some(Token::Ident(s)) => Ok(s.clone()),
            Some(t) => Err(ParseError::Unexpected {
                got: format!("{:?}", t),
                expected: "identifier".into(),
            }),
            None => Err(ParseError::Eof("identifier".into())),
        }
    }

    fn expect_str(&mut self) -> Result<String, ParseError> {
        match self.advance() {
            Some(Token::Str(s)) => Ok(s.clone()),
            Some(Token::Ident(s)) => Ok(s.clone()),
            Some(t) => Err(ParseError::Unexpected {
                got: format!("{:?}", t),
                expected: "string".into(),
            }),
            None => Err(ParseError::Eof("string".into())),
        }
    }

    // ── parsing ──────────────────────────────────────────────────────────────

    fn parse(&mut self) -> Result<Query, ParseError> {
        let query = match self.peek() {
            Some(Token::LBrace) => {
                let log_query = self.parse_log_query()?;
                // If followed by range agg wrapper, it was probably invoked differently.
                Query::Log(log_query)
            }
            Some(Token::Rate)
            | Some(Token::CountOverTime)
            | Some(Token::BytesOverTime)
            | Some(Token::BytesRate)
            | Some(Token::AbsentOverTime)
            | Some(Token::SumOverTime)
            | Some(Token::AvgOverTime)
            | Some(Token::MaxOverTime)
            | Some(Token::MinOverTime)
            | Some(Token::FirstOverTime)
            | Some(Token::LastOverTime)
            | Some(Token::StddevOverTime)
            | Some(Token::StdvarOverTime)
            | Some(Token::QuantileOverTime) => Query::Metric(self.parse_metric_query()?),
            Some(Token::Sum)
            | Some(Token::Avg)
            | Some(Token::Max)
            | Some(Token::Min)
            | Some(Token::Count)
            | Some(Token::Stddev)
            | Some(Token::Stdvar)
            | Some(Token::Topk)
            | Some(Token::Bottomk)
            | Some(Token::Quantile) => Query::Metric(self.parse_metric_query()?),
            Some(Token::Number(_)) => {
                let n = if let Some(Token::Number(n)) = self.advance() {
                    *n
                } else {
                    unreachable!()
                };
                Query::Metric(MetricQuery::Literal(n))
            }
            Some(t) => {
                return Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "stream selector `{` or metric function".into(),
                });
            }
            None => return Err(ParseError::Eof("query".into())),
        };
        Ok(query)
    }

    // ── Stream selector ──────────────────────────────────────────────────────

    fn parse_stream_selector(&mut self) -> Result<StreamSelector, ParseError> {
        self.expect(&Token::LBrace)?;
        let mut matchers = Vec::new();
        loop {
            match self.peek() {
                Some(Token::RBrace) => {
                    self.advance();
                    break;
                }
                Some(Token::Comma) => {
                    self.advance();
                    continue;
                }
                Some(Token::Ident(_)) => {
                    let name = self.expect_ident()?;
                    let op = match self.advance() {
                        Some(Token::Eq) => MatchOp::Eq,
                        Some(Token::Neq) => MatchOp::Neq,
                        Some(Token::Re) => MatchOp::Re,
                        Some(Token::NotRe) => MatchOp::NotRe,
                        Some(t) => {
                            return Err(ParseError::Unexpected {
                                got: format!("{:?}", t),
                                expected: "matcher operator (= != =~ !~)".into(),
                            });
                        }
                        None => return Err(ParseError::Eof("matcher operator".into())),
                    };
                    let value = self.expect_str()?;
                    matchers.push(LabelMatcher { name, op, value });
                }
                Some(t) => {
                    return Err(ParseError::Unexpected {
                        got: format!("{:?}", t),
                        expected: "label name or `}`".into(),
                    });
                }
                None => return Err(ParseError::Eof("`}`".into())),
            }
        }
        Ok(StreamSelector { matchers })
    }

    // ── Log query ────────────────────────────────────────────────────────────

    fn parse_log_query(&mut self) -> Result<LogQuery, ParseError> {
        let selector = self.parse_stream_selector()?;
        let pipeline = self.parse_pipeline()?;
        Ok(LogQuery { selector, pipeline })
    }

    fn parse_pipeline(&mut self) -> Result<Vec<PipelineStage>, ParseError> {
        let mut stages = Vec::new();
        loop {
            match self.peek() {
                Some(Token::PipeEq) => {
                    self.advance();
                    let s = self.expect_str()?;
                    stages.push(PipelineStage::LineFilter(LineFilter::Contains(s)));
                }
                Some(Token::PipeNeq) => {
                    self.advance();
                    let s = self.expect_str()?;
                    stages.push(PipelineStage::LineFilter(LineFilter::NotContains(s)));
                }
                Some(Token::PipeTilde) => {
                    self.advance();
                    let s = self.expect_str()?;
                    stages.push(PipelineStage::LineFilter(LineFilter::Matches(s)));
                }
                Some(Token::PipeBangTilde) => {
                    self.advance();
                    let s = self.expect_str()?;
                    stages.push(PipelineStage::LineFilter(LineFilter::NotMatches(s)));
                }
                Some(Token::Pipe) => {
                    self.advance();
                    let stage = self.parse_pipeline_stage()?;
                    stages.push(stage);
                }
                // `!= "text"` used as a pipeline filter (Loki 2.x shorthand)
                Some(Token::Neq) => {
                    self.advance();
                    let s = self.expect_str()?;
                    stages.push(PipelineStage::LineFilter(LineFilter::NotContains(s)));
                }
                Some(Token::NotRe) => {
                    self.advance();
                    let s = self.expect_str()?;
                    stages.push(PipelineStage::LineFilter(LineFilter::NotMatches(s)));
                }
                _ => break,
            }
        }
        Ok(stages)
    }

    fn parse_pipeline_stage(&mut self) -> Result<PipelineStage, ParseError> {
        match self.peek() {
            Some(Token::Json) => {
                self.advance();
                Ok(PipelineStage::Parser(ast::Parser::Json))
            }
            Some(Token::Logfmt) => {
                self.advance();
                Ok(PipelineStage::Parser(ast::Parser::Logfmt))
            }
            Some(Token::Regexp) => {
                self.advance();
                let pat = self.expect_str()?;
                Ok(PipelineStage::Parser(ast::Parser::Regexp(pat)))
            }
            Some(Token::Pattern) => {
                self.advance();
                let pat = self.expect_str()?;
                Ok(PipelineStage::Parser(ast::Parser::Pattern(pat)))
            }
            Some(Token::Unpack) => {
                self.advance();
                Ok(PipelineStage::Parser(ast::Parser::Unpack))
            }
            Some(Token::Unwrap) => {
                self.advance();
                let (label, converter) = self.parse_unwrap()?;
                Ok(PipelineStage::Unwrap(UnwrapExpr { label, converter }))
            }
            Some(Token::LineFormat) => {
                self.advance();
                let tmpl = self.expect_str()?;
                Ok(PipelineStage::LineFormat(LineFormat { template: tmpl }))
            }
            Some(Token::LabelFormat) => {
                self.advance();
                let mappings = self.parse_label_format_mappings()?;
                Ok(PipelineStage::LabelFormat(LabelFormat { mappings }))
            }
            Some(Token::Decolorize) => {
                self.advance();
                Ok(PipelineStage::Decolorize(Decolorize))
            }
            Some(Token::Drop) => {
                self.advance();
                let labels = self.parse_drop_keep_list()?;
                Ok(PipelineStage::Drop(DropLabels { labels }))
            }
            Some(Token::Keep) => {
                self.advance();
                let labels = self.parse_drop_keep_list()?;
                Ok(PipelineStage::Keep(KeepLabels { labels }))
            }
            // Label filter: `| label op value`
            Some(Token::Ident(_)) => {
                let lf = self.parse_label_filter()?;
                Ok(PipelineStage::LabelFilter(lf))
            }
            // Line filter shortcuts after `|`
            Some(Token::PipeEq) => {
                self.advance();
                let s = self.expect_str()?;
                Ok(PipelineStage::LineFilter(LineFilter::Contains(s)))
            }
            Some(t) => Err(ParseError::Unexpected {
                got: format!("{:?}", t),
                expected: "pipeline stage".into(),
            }),
            None => Err(ParseError::Eof("pipeline stage".into())),
        }
    }

    fn parse_label_filter(&mut self) -> Result<LabelFilter, ParseError> {
        let label = self.expect_ident()?;
        let op = match self.advance() {
            Some(Token::EqEq) | Some(Token::Eq) => CompareOp::Eq,
            Some(Token::Neq) => CompareOp::Neq,
            Some(Token::Gt) => CompareOp::Gt,
            Some(Token::Gte) => CompareOp::Gte,
            Some(Token::Lt) => CompareOp::Lt,
            Some(Token::Lte) => CompareOp::Lte,
            Some(Token::Re) => CompareOp::Re,
            Some(Token::NotRe) => CompareOp::NotRe,
            Some(t) => {
                return Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "comparison operator".into(),
                });
            }
            None => return Err(ParseError::Eof("comparison operator".into())),
        };
        let value = match self.peek() {
            Some(Token::Str(_)) => {
                let s = if let Some(Token::Str(s)) = self.advance() {
                    s.clone()
                } else {
                    unreachable!()
                };
                if op == CompareOp::Re || op == CompareOp::NotRe {
                    LabelFilterValue::String(s)
                } else {
                    LabelFilterValue::String(s)
                }
            }
            Some(Token::Number(_)) => {
                let n = if let Some(Token::Number(n)) = self.advance() {
                    *n
                } else {
                    unreachable!()
                };
                LabelFilterValue::Float(n)
            }
            Some(Token::DurationLit(_)) => {
                let ns = if let Some(Token::DurationLit(ns)) = self.advance() {
                    *ns
                } else {
                    unreachable!()
                };
                LabelFilterValue::Duration(Duration::from_nanos(ns))
            }
            Some(Token::Ident(_)) => {
                let s = self.expect_ident()?;
                LabelFilterValue::String(s)
            }
            Some(t) => {
                return Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "filter value".into(),
                });
            }
            None => return Err(ParseError::Eof("filter value".into())),
        };
        Ok(LabelFilter { label, op, value })
    }

    fn parse_unwrap(&mut self) -> Result<(String, Option<String>), ParseError> {
        // `duration(label)` or `bytes(label)` or just `label`
        match self.peek() {
            Some(Token::Duration) | Some(Token::Bytes) => {
                let conv = match self.advance() {
                    Some(Token::Duration) => "duration".to_owned(),
                    Some(Token::Bytes) => "bytes".to_owned(),
                    _ => unreachable!(),
                };
                self.expect(&Token::LParen)?;
                let label = self.expect_ident()?;
                self.expect(&Token::RParen)?;
                Ok((label, Some(conv)))
            }
            _ => {
                let label = self.expect_ident()?;
                Ok((label, None))
            }
        }
    }

    fn parse_label_format_mappings(&mut self) -> Result<Vec<(String, String)>, ParseError> {
        let mut mappings = Vec::new();
        loop {
            let new_name = self.expect_ident()?;
            self.expect(&Token::Eq)?;
            let old_name = self.expect_ident()?;
            mappings.push((new_name, old_name));
            if self.peek() == Some(&Token::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        Ok(mappings)
    }

    // ── Metric query ──────────────────────────────────────────────────────────

    fn parse_metric_query(&mut self) -> Result<MetricQuery, ParseError> {
        let lhs = self.parse_metric_atom()?;
        // Binary operation?
        let lhs = self.parse_binary_rhs(lhs, 0)?;
        Ok(lhs)
    }

    fn parse_metric_atom(&mut self) -> Result<MetricQuery, ParseError> {
        match self.peek() {
            Some(Token::Number(_)) => {
                let n = if let Some(Token::Number(n)) = self.advance() {
                    *n
                } else {
                    unreachable!()
                };
                Ok(MetricQuery::Literal(n))
            }
            Some(Token::LParen) => {
                self.advance();
                let inner = self.parse_metric_query()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::Rate)
            | Some(Token::CountOverTime)
            | Some(Token::BytesOverTime)
            | Some(Token::BytesRate)
            | Some(Token::AbsentOverTime)
            | Some(Token::SumOverTime)
            | Some(Token::AvgOverTime)
            | Some(Token::MaxOverTime)
            | Some(Token::MinOverTime)
            | Some(Token::FirstOverTime)
            | Some(Token::LastOverTime)
            | Some(Token::StddevOverTime)
            | Some(Token::StdvarOverTime)
            | Some(Token::QuantileOverTime) => Ok(MetricQuery::RangeAgg(self.parse_range_agg()?)),
            Some(Token::Sum)
            | Some(Token::Avg)
            | Some(Token::Max)
            | Some(Token::Min)
            | Some(Token::Count)
            | Some(Token::Stddev)
            | Some(Token::Stdvar)
            | Some(Token::Topk)
            | Some(Token::Bottomk)
            | Some(Token::Quantile) => Ok(MetricQuery::VectorAgg(self.parse_vector_agg()?)),
            Some(t) => Err(ParseError::Unexpected {
                got: format!("{:?}", t),
                expected: "metric expression".into(),
            }),
            None => Err(ParseError::Eof("metric expression".into())),
        }
    }

    /// After the `[range]`, optionally consume an `offset <duration>` modifier.
    fn try_parse_offset(&mut self) -> Result<Option<Duration>, ParseError> {
        if let Some(Token::Offset) = self.peek() {
            self.advance();
            match self.advance() {
                Some(Token::DurationLit(ns)) => Ok(Some(Duration::from_nanos(*ns))),
                Some(t) => Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "duration after offset".into(),
                }),
                None => Err(ParseError::Eof("duration after offset".into())),
            }
        } else {
            Ok(None)
        }
    }

    fn parse_range_agg(&mut self) -> Result<LogRangeAggregation, ParseError> {
        let agg = match self.advance() {
            Some(Token::Rate) => RangeAgg::Rate,
            Some(Token::CountOverTime) => RangeAgg::CountOverTime,
            Some(Token::BytesOverTime) => RangeAgg::BytesOverTime,
            Some(Token::BytesRate) => RangeAgg::BytesRate,
            Some(Token::AbsentOverTime) => RangeAgg::AbsentOverTime,
            Some(Token::SumOverTime) => RangeAgg::SumOverTime,
            Some(Token::AvgOverTime) => RangeAgg::AvgOverTime,
            Some(Token::MaxOverTime) => RangeAgg::MaxOverTime,
            Some(Token::MinOverTime) => RangeAgg::MinOverTime,
            Some(Token::FirstOverTime) => RangeAgg::FirstOverTime,
            Some(Token::LastOverTime) => RangeAgg::LastOverTime,
            Some(Token::StddevOverTime) => RangeAgg::StddevOverTime,
            Some(Token::StdvarOverTime) => RangeAgg::StdvarOverTime,
            Some(Token::QuantileOverTime) => {
                self.expect(&Token::LParen)?;
                let q = match self.advance() {
                    Some(Token::Number(n)) => *n,
                    _ => {
                        return Err(ParseError::Other(
                            "quantile requires numeric first arg".into(),
                        ));
                    }
                };
                self.expect(&Token::Comma)?;
                // Continue parsing the rest below with already-consumed `(`
                let query = self.parse_log_query()?;
                self.expect(&Token::LBracket)?;
                let range_ns = match self.advance() {
                    Some(Token::DurationLit(ns)) => *ns,
                    Some(t) => {
                        return Err(ParseError::Unexpected {
                            got: format!("{:?}", t),
                            expected: "duration".into(),
                        });
                    }
                    None => return Err(ParseError::Eof("duration".into())),
                };
                self.expect(&Token::RBracket)?;
                let offset = self.try_parse_offset()?;
                self.expect(&Token::RParen)?;
                let grouping = self.try_parse_grouping()?;
                return Ok(LogRangeAggregation {
                    agg: RangeAgg::QuantileOverTime(q),
                    query,
                    range: Duration::from_nanos(range_ns),
                    grouping,
                    offset,
                });
            }
            Some(t) => {
                return Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "range agg function".into(),
                });
            }
            None => return Err(ParseError::Eof("range agg function".into())),
        };

        self.expect(&Token::LParen)?;
        let query = self.parse_log_query()?;
        self.expect(&Token::LBracket)?;
        let range_ns = match self.advance() {
            Some(Token::DurationLit(ns)) => *ns,
            Some(t) => {
                return Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "duration".into(),
                });
            }
            None => return Err(ParseError::Eof("duration".into())),
        };
        self.expect(&Token::RBracket)?;
        let offset = self.try_parse_offset()?;
        self.expect(&Token::RParen)?;
        let grouping = self.try_parse_grouping()?;

        Ok(LogRangeAggregation {
            agg,
            query,
            range: Duration::from_nanos(range_ns),
            grouping,
            offset,
        })
    }

    fn parse_vector_agg(&mut self) -> Result<VectorAggregation, ParseError> {
        let agg = match self.advance() {
            Some(Token::Sum) => VectorAgg::Sum,
            Some(Token::Avg) => VectorAgg::Avg,
            Some(Token::Max) => VectorAgg::Max,
            Some(Token::Min) => VectorAgg::Min,
            Some(Token::Count) => VectorAgg::Count,
            Some(Token::Stddev) => VectorAgg::Stddev,
            Some(Token::Stdvar) => VectorAgg::Stdvar,
            Some(Token::Topk) => {
                self.expect(&Token::LParen)?;
                let k = match self.advance() {
                    Some(Token::Number(n)) => *n as u64,
                    _ => return Err(ParseError::Other("topk requires integer k".into())),
                };
                self.expect(&Token::Comma)?;
                let inner = self.parse_metric_query()?;
                self.expect(&Token::RParen)?;
                let grouping = self.try_parse_grouping()?;
                return Ok(VectorAggregation {
                    agg: VectorAgg::Topk(k),
                    grouping,
                    inner: Box::new(inner),
                });
            }
            Some(Token::Bottomk) => {
                self.expect(&Token::LParen)?;
                let k = match self.advance() {
                    Some(Token::Number(n)) => *n as u64,
                    _ => return Err(ParseError::Other("bottomk requires integer k".into())),
                };
                self.expect(&Token::Comma)?;
                let inner = self.parse_metric_query()?;
                self.expect(&Token::RParen)?;
                let grouping = self.try_parse_grouping()?;
                return Ok(VectorAggregation {
                    agg: VectorAgg::Bottomk(k),
                    grouping,
                    inner: Box::new(inner),
                });
            }
            Some(Token::Quantile) => {
                self.expect(&Token::LParen)?;
                let q = match self.advance() {
                    Some(Token::Number(n)) => *n,
                    _ => return Err(ParseError::Other("quantile requires numeric q".into())),
                };
                self.expect(&Token::Comma)?;
                let inner = self.parse_metric_query()?;
                self.expect(&Token::RParen)?;
                let grouping = self.try_parse_grouping()?;
                return Ok(VectorAggregation {
                    agg: VectorAgg::Quantile(q),
                    grouping,
                    inner: Box::new(inner),
                });
            }
            Some(t) => {
                return Err(ParseError::Unexpected {
                    got: format!("{:?}", t),
                    expected: "vector agg function".into(),
                });
            }
            None => return Err(ParseError::Eof("vector agg function".into())),
        };

        // Optional grouping before `(`
        let grouping = self.try_parse_grouping()?;
        self.expect(&Token::LParen)?;
        let inner = self.parse_metric_query()?;
        self.expect(&Token::RParen)?;
        // Also allow grouping after `)`
        let grouping = if grouping.is_some() {
            grouping
        } else {
            self.try_parse_grouping()?
        };

        Ok(VectorAggregation {
            agg,
            grouping,
            inner: Box::new(inner),
        })
    }

    fn try_parse_grouping(&mut self) -> Result<Option<Grouping>, ParseError> {
        match self.peek() {
            Some(Token::By) => {
                self.advance();
                let labels = self.parse_label_list()?;
                Ok(Some(Grouping {
                    without: false,
                    labels,
                }))
            }
            Some(Token::Without) => {
                self.advance();
                let labels = self.parse_label_list()?;
                Ok(Some(Grouping {
                    without: true,
                    labels,
                }))
            }
            _ => Ok(None),
        }
    }

    /// Parse a comma-separated `drop`/`keep` list. Each entry is a bare label
    /// name or `name <op> value` (Loki only allows `=`/`!=`/`=~`/`!~` here).
    fn parse_drop_keep_list(&mut self) -> Result<Vec<DropKeepLabel>, ParseError> {
        let mut out = Vec::new();
        loop {
            let name = self.expect_ident()?;
            let op = match self.peek() {
                Some(Token::Eq) | Some(Token::EqEq) => Some(MatchOp::Eq),
                Some(Token::Neq) => Some(MatchOp::Neq),
                Some(Token::Re) => Some(MatchOp::Re),
                Some(Token::NotRe) => Some(MatchOp::NotRe),
                _ => None,
            };
            match op {
                Some(op) => {
                    self.advance();
                    let value = self.expect_str()?;
                    out.push(DropKeepLabel::Matcher(LabelMatcher { name, op, value }));
                }
                None => out.push(DropKeepLabel::Name(name)),
            }
            if let Some(Token::Comma) = self.peek() {
                self.advance();
            } else {
                break;
            }
        }
        Ok(out)
    }

    fn parse_label_list(&mut self) -> Result<Vec<String>, ParseError> {
        self.expect(&Token::LParen)?;
        let mut labels = Vec::new();
        loop {
            match self.peek() {
                Some(Token::RParen) => {
                    self.advance();
                    break;
                }
                Some(Token::Comma) => {
                    self.advance();
                }
                Some(Token::Ident(_)) => {
                    labels.push(self.expect_ident()?);
                }
                Some(t) => {
                    return Err(ParseError::Unexpected {
                        got: format!("{:?}", t),
                        expected: "label or `)`".into(),
                    });
                }
                None => return Err(ParseError::Eof("`)`".into())),
            }
        }
        Ok(labels)
    }

    fn token_precedence(t: &Token) -> Option<u8> {
        match t {
            Token::Or => Some(1),
            Token::And | Token::Unless => Some(2),
            Token::EqEq | Token::Neq | Token::Gt | Token::Gte | Token::Lt | Token::Lte => Some(3),
            Token::Plus | Token::Minus => Some(4),
            Token::Star | Token::Slash | Token::Percent => Some(5),
            Token::Caret => Some(6),
            _ => None,
        }
    }

    fn parse_binary_rhs(
        &mut self,
        mut lhs: MetricQuery,
        min_prec: u8,
    ) -> Result<MetricQuery, ParseError> {
        loop {
            let prec = match self.peek().and_then(Self::token_precedence) {
                Some(p) if p >= min_prec => p,
                _ => break,
            };

            let op_tok = self.advance().unwrap().clone();
            let bool_modifier = self.peek() == Some(&Token::Bool);
            if bool_modifier {
                self.advance();
            }

            let grouping = self.parse_vector_match_grouping()?;
            let mut rhs = self.parse_metric_atom()?;
            rhs = self.parse_binary_rhs(rhs, prec + 1)?;

            let bm = bool_modifier;
            let op = match &op_tok {
                Token::Plus => BinOp::Add,
                Token::Minus => BinOp::Sub,
                Token::Star => BinOp::Mul,
                Token::Slash => BinOp::Div,
                Token::Percent => BinOp::Mod,
                Token::Caret => BinOp::Pow,
                Token::And => BinOp::And,
                Token::Or => BinOp::Or,
                Token::Unless => BinOp::Unless,
                Token::EqEq => BinOp::CmpEq(bm),
                Token::Neq => BinOp::CmpNeq(bm),
                Token::Gt => BinOp::CmpGt(bm),
                Token::Gte => BinOp::CmpGte(bm),
                Token::Lt => BinOp::CmpLt(bm),
                Token::Lte => BinOp::CmpLte(bm),
                _ => break,
            };

            lhs = MetricQuery::BinaryExpr(BinaryExpr {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                grouping,
            });
        }
        Ok(lhs)
    }

    fn parse_vector_match_grouping(&mut self) -> Result<Option<VectorMatchGrouping>, ParseError> {
        let card = match self.peek() {
            Some(Token::On) => {
                self.advance();
                MatchCardinality::OneToOne
            }
            Some(Token::Ignoring) => {
                self.advance();
                MatchCardinality::OneToOne
            }
            Some(Token::GroupLeft) => {
                self.advance();
                MatchCardinality::ManyToOne
            }
            Some(Token::GroupRight) => {
                self.advance();
                MatchCardinality::OneToMany
            }
            _ => return Ok(None),
        };
        let labels = if self.peek() == Some(&Token::LParen) {
            self.parse_label_list()?
        } else {
            Vec::new()
        };
        let include = if self.peek() == Some(&Token::LParen) {
            self.parse_label_list()?
        } else {
            Vec::new()
        };
        Ok(Some(VectorMatchGrouping {
            card,
            labels,
            include,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_stream_selector_only() {
        let q = Parser::parse_query(r#"{app="nginx"}"#).unwrap();
        assert!(matches!(q, Query::Log(_)));
    }

    #[test]
    fn parse_log_query_with_pipeline() {
        let q = Parser::parse_query(r#"{app="api"} |= "error" | json | status >= 500"#).unwrap();
        if let Query::Log(lq) = q {
            assert_eq!(lq.pipeline.len(), 3);
        } else {
            panic!("expected log query");
        }
    }

    #[test]
    fn parse_rate() {
        let q = Parser::parse_query(r#"rate({job="varlogs"}[5m])"#).unwrap();
        assert!(matches!(q, Query::Metric(MetricQuery::RangeAgg(_))));
    }

    #[test]
    fn parse_sum_by() {
        let q = Parser::parse_query(r#"sum by (app) (rate({job="x"}[1m]))"#).unwrap();
        if let Query::Metric(MetricQuery::VectorAgg(va)) = q {
            assert_eq!(va.agg, VectorAgg::Sum);
            assert!(va.grouping.is_some());
        } else {
            panic!("expected vector agg");
        }
    }

    #[test]
    fn parse_topk() {
        let q = Parser::parse_query(r#"topk(5, rate({app="x"}[1m]))"#).unwrap();
        assert!(matches!(q, Query::Metric(MetricQuery::VectorAgg(_))));
    }

    #[test]
    fn parse_label_filter() {
        let q = Parser::parse_query(r#"{app="x"} | json | status_code >= 400"#).unwrap();
        if let Query::Log(lq) = q {
            assert_eq!(lq.pipeline.len(), 2);
        } else {
            panic!();
        }
    }

    #[test]
    fn parse_ip_line_filter_match() {
        let q = Parser::parse_query(r#"{app="x"} |= ip("192.168.0.0/16")"#).unwrap();
        if let Query::Log(lq) = q {
            assert_eq!(lq.pipeline.len(), 1);
            match &lq.pipeline[0] {
                PipelineStage::LineFilter(LineFilter::IpMatch(p)) => {
                    assert_eq!(
                        *p,
                        super::super::ip::IpPattern::parse("192.168.0.0/16").unwrap()
                    );
                }
                other => panic!("expected ip line filter, got {other:?}"),
            }
        } else {
            panic!("expected log query");
        }
    }

    #[test]
    fn parse_ip_line_filter_not_match() {
        let q = Parser::parse_query(r#"{app="x"} != ip("10.0.0.1")"#).unwrap();
        if let Query::Log(lq) = q {
            assert!(matches!(
                lq.pipeline[0],
                PipelineStage::LineFilter(LineFilter::IpNotMatch(_))
            ));
        } else {
            panic!("expected log query");
        }
    }

    #[test]
    fn parse_ip_bad_pattern_errs() {
        assert!(Parser::parse_query(r#"{app="x"} |= ip("nonsense")"#).is_err());
    }

    #[test]
    fn parse_plain_eq_filter_still_works() {
        // `ip` detection must not break ordinary `|= "text"` filters.
        let q = Parser::parse_query(r#"{app="x"} |= "ip something""#).unwrap();
        if let Query::Log(lq) = q {
            assert!(matches!(
                lq.pipeline[0],
                PipelineStage::LineFilter(LineFilter::Contains(_))
            ));
        } else {
            panic!("expected log query");
        }
    }

    #[test]
    fn parse_line_format() {
        let q =
            Parser::parse_query(r#"{app="x"} | line_format "{{.method}} {{.status}}""#).unwrap();
        if let Query::Log(lq) = q {
            assert!(matches!(&lq.pipeline[0], PipelineStage::LineFormat(_)));
        } else {
            panic!();
        }
    }

    #[test]
    fn parse_range_with_offset() {
        let q = Parser::parse_query(r#"rate({app="x"}[5m] offset 1h)"#).unwrap();
        if let Query::Metric(MetricQuery::RangeAgg(ra)) = q {
            assert_eq!(ra.range, Duration::from_secs(5 * 60));
            assert_eq!(ra.offset, Some(Duration::from_secs(3600)));
        } else {
            panic!("expected range agg");
        }
    }

    #[test]
    fn parse_range_without_offset_is_none() {
        let q = Parser::parse_query(r#"count_over_time({app="x"}[5m])"#).unwrap();
        if let Query::Metric(MetricQuery::RangeAgg(ra)) = q {
            assert_eq!(ra.offset, None);
        } else {
            panic!("expected range agg");
        }
    }

    #[test]
    fn parse_quantile_over_time_with_offset() {
        let q =
            Parser::parse_query(r#"quantile_over_time(0.9, {a="b"}[5m] offset 30m)"#).unwrap();
        if let Query::Metric(MetricQuery::RangeAgg(ra)) = q {
            assert_eq!(ra.offset, Some(Duration::from_secs(1800)));
        } else {
            panic!("expected range agg");
        }
    }

    #[test]
    fn parse_binary_expr() {
        let q = Parser::parse_query(r#"rate({a="b"}[1m]) + rate({a="c"}[1m])"#).unwrap();
        assert!(matches!(q, Query::Metric(MetricQuery::BinaryExpr(_))));
    }

    #[test]
    fn parse_drop_labels_bare_names() {
        let q = Parser::parse_query(r#"{app="x"} | json | drop level, status"#).unwrap();
        if let Query::Log(lq) = q {
            // json + drop = two stages.
            assert_eq!(lq.pipeline.len(), 2);
            if let PipelineStage::Drop(d) = lq.pipeline.last().unwrap() {
                assert_eq!(d.labels.len(), 2);
                assert!(matches!(&d.labels[0], DropKeepLabel::Name(n) if n == "level"));
                assert!(matches!(&d.labels[1], DropKeepLabel::Name(n) if n == "status"));
            } else {
                panic!("expected Drop stage, got {:?}", lq.pipeline.last());
            }
        } else {
            panic!("expected log query");
        }
    }

    #[test]
    fn parse_drop_labels_with_matcher() {
        let q = Parser::parse_query(r#"{app="x"} | drop level="debug", trace_id"#).unwrap();
        if let Query::Log(lq) = q {
            if let PipelineStage::Drop(d) = &lq.pipeline[0] {
                assert_eq!(d.labels.len(), 2);
                match &d.labels[0] {
                    DropKeepLabel::Matcher(m) => {
                        assert_eq!(m.name, "level");
                        assert_eq!(m.op, MatchOp::Eq);
                        assert_eq!(m.value, "debug");
                    }
                    other => panic!("expected matcher, got {:?}", other),
                }
                assert!(matches!(&d.labels[1], DropKeepLabel::Name(n) if n == "trace_id"));
            } else {
                panic!("expected Drop stage");
            }
        } else {
            panic!("expected log query");
        }
    }

    #[test]
    fn parse_keep_labels_names_and_matcher() {
        let q = Parser::parse_query(r#"{app="x"} | logfmt | keep level, status="500""#).unwrap();
        if let Query::Log(lq) = q {
            assert_eq!(lq.pipeline.len(), 2); // logfmt + keep
            if let PipelineStage::Keep(k) = lq.pipeline.last().unwrap() {
                assert_eq!(k.labels.len(), 2);
                assert!(matches!(&k.labels[0], DropKeepLabel::Name(n) if n == "level"));
                match &k.labels[1] {
                    DropKeepLabel::Matcher(m) => {
                        assert_eq!(m.name, "status");
                        assert_eq!(m.op, MatchOp::Eq);
                        assert_eq!(m.value, "500");
                    }
                    other => panic!("expected matcher, got {:?}", other),
                }
            } else {
                panic!("expected Keep stage, got {:?}", lq.pipeline.last());
            }
        } else {
            panic!("expected log query");
        }
    }
}
