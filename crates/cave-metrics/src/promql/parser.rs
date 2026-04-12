//! PromQL recursive-descent / Pratt parser.

#![allow(dead_code)]

use crate::error::{MetricsError, MetricsResult};
use crate::model::LabelMatcher;
use super::ast::*;
use super::lexer::{Lexer, Token};

pub fn parse(input: &str) -> MetricsResult<Expr> {
    let mut p = Parser { lexer: Lexer::new(input) };
    let expr = p.parse_expr(0)?;
    // Ensure nothing left
    let tok = p.lexer.next()?;
    if tok != Token::Eof {
        return Err(MetricsError::Parse(format!("unexpected token at end: {:?}", tok)));
    }
    Ok(expr)
}

struct Parser {
    lexer: Lexer,
}

impl Parser {
    fn parse_expr(&mut self, min_prec: u8) -> MetricsResult<Expr> {
        let mut lhs = self.parse_unary_or_primary()?;
        loop {
            let op = match self.peek_binary_op()? {
                Some(op) => op,
                None => break,
            };
            let prec = op.precedence();
            if prec <= min_prec {
                break;
            }
            // Consume the operator token(s)
            self.consume_binary_op(op)?;
            // Parse optional bool modifier
            let mut return_bool = false;
            if matches!(self.lexer.peek()?, Token::Bool) {
                self.lexer.next()?;
                return_bool = true;
            }
            // Parse optional matching modifiers
            let mut matching = VectorMatching::default();
            loop {
                match self.lexer.peek()? {
                    Token::On => {
                        self.lexer.next()?;
                        matching.labels = self.parse_label_list()?;
                    }
                    Token::Ignoring => {
                        self.lexer.next()?;
                        matching.labels = self.parse_label_list()?;
                    }
                    Token::GroupLeft => {
                        self.lexer.next()?;
                        matching.card = MatchingCard::ManyToOne;
                        if matches!(self.lexer.peek()?, Token::LParen) {
                            matching.include = self.parse_label_list()?;
                        }
                    }
                    Token::GroupRight => {
                        self.lexer.next()?;
                        matching.card = MatchingCard::OneToMany;
                        if matches!(self.lexer.peek()?, Token::LParen) {
                            matching.include = self.parse_label_list()?;
                        }
                    }
                    _ => break,
                }
            }
            let next_min = if op.is_right_assoc() { prec - 1 } else { prec };
            let rhs = self.parse_expr(next_min)?;
            lhs = Expr::Binary {
                op,
                lhs: Box::new(lhs),
                rhs: Box::new(rhs),
                matching,
                return_bool,
            };
        }
        Ok(lhs)
    }

    fn peek_binary_op(&mut self) -> MetricsResult<Option<BinaryOp>> {
        let tok = self.lexer.peek()?;
        let op = match tok {
            Token::Add => Some(BinaryOp::Add),
            Token::Sub => Some(BinaryOp::Sub),
            Token::Mul => Some(BinaryOp::Mul),
            Token::Div => Some(BinaryOp::Div),
            Token::Mod => Some(BinaryOp::Mod),
            Token::Pow => Some(BinaryOp::Pow),
            Token::Eq => Some(BinaryOp::Eql),
            Token::NotEq => Some(BinaryOp::Neq),
            Token::Lt => Some(BinaryOp::Lss),
            Token::Gt => Some(BinaryOp::Gtr),
            Token::Lte => Some(BinaryOp::Lte),
            Token::Gte => Some(BinaryOp::Gte),
            Token::And => Some(BinaryOp::And),
            Token::Or => Some(BinaryOp::Or),
            Token::Unless => Some(BinaryOp::Unless),
            Token::Atan2 => Some(BinaryOp::Atan2),
            _ => None,
        };
        Ok(op)
    }

    fn consume_binary_op(&mut self, _op: BinaryOp) -> MetricsResult<()> {
        self.lexer.next()?;
        Ok(())
    }

    fn parse_unary_or_primary(&mut self) -> MetricsResult<Expr> {
        match self.lexer.peek()? {
            Token::Sub => {
                self.lexer.next()?;
                let e = self.parse_primary()?;
                Ok(Expr::Unary { op: UnaryOp::Neg, expr: Box::new(e) })
            }
            Token::Add => {
                self.lexer.next()?;
                let e = self.parse_primary()?;
                Ok(Expr::Unary { op: UnaryOp::Pos, expr: Box::new(e) })
            }
            _ => self.parse_primary(),
        }
    }

    fn parse_primary(&mut self) -> MetricsResult<Expr> {
        let tok = self.lexer.peek()?.clone();
        match tok {
            Token::Number(n) => {
                self.lexer.next()?;
                Ok(Expr::NumberLiteral(n))
            }
            Token::Str(s) => {
                self.lexer.next()?;
                Ok(Expr::StringLiteral(s))
            }
            Token::LParen => {
                self.lexer.next()?;
                let inner = self.parse_expr(0)?;
                self.expect(Token::RParen)?;
                Ok(Expr::Paren(Box::new(inner)))
            }
            Token::Ident(name) => {
                // Could be: metric selector, function call, or aggregation
                let lower = name.to_lowercase();
                // Check for aggregation ops
                if let Some(agg_op) = parse_aggregate_op(&lower) {
                    self.lexer.next()?; // consume ident
                    return self.parse_aggregation(agg_op);
                }
                self.lexer.next()?; // consume ident
                // Function call?
                if self.lexer.peek()? == &Token::LParen {
                    return self.parse_call(name);
                }
                // Vector selector with possible {matchers}
                let mut matchers: Vec<LabelMatcher> = vec![
                    LabelMatcher::Equal { name: "__name__".to_string(), value: name.clone() }
                ];
                let mut extra_name = Some(name);
                if self.lexer.peek()? == &Token::LBrace {
                    self.lexer.next()?;
                    let extra = self.parse_label_matchers()?;
                    matchers.extend(extra);
                    self.expect(Token::RBrace)?;
                }
                // offset / @
                let (offset, at) = self.parse_offset_at()?;
                // matrix selector?
                if self.lexer.peek()? == &Token::LBracket {
                    self.lexer.next()?;
                    let range_ms = self.parse_duration()?;
                    self.expect(Token::RBracket)?;
                    let sel = Expr::VectorSelector {
                        name: extra_name.take(),
                        matchers,
                        offset,
                        at,
                    };
                    return Ok(Expr::MatrixSelector {
                        selector: Box::new(sel),
                        range_ms,
                    });
                }
                Ok(Expr::VectorSelector {
                    name: extra_name,
                    matchers,
                    offset,
                    at,
                })
            }
            Token::LBrace => {
                self.lexer.next()?;
                let matchers = self.parse_label_matchers()?;
                self.expect(Token::RBrace)?;
                let (offset, at) = self.parse_offset_at()?;
                if self.lexer.peek()? == &Token::LBracket {
                    self.lexer.next()?;
                    let range_ms = self.parse_duration()?;
                    self.expect(Token::RBracket)?;
                    return Ok(Expr::MatrixSelector {
                        selector: Box::new(Expr::VectorSelector {
                            name: None,
                            matchers,
                            offset,
                            at,
                        }),
                        range_ms,
                    });
                }
                Ok(Expr::VectorSelector { name: None, matchers, offset, at })
            }
            other => Err(MetricsError::Parse(format!("unexpected token: {:?}", other))),
        }
    }

    fn parse_call(&mut self, func: String) -> MetricsResult<Expr> {
        self.expect(Token::LParen)?;
        let mut args = Vec::new();
        if self.lexer.peek()? != &Token::RParen {
            args.push(self.parse_expr(0)?);
            while self.lexer.peek()? == &Token::Comma {
                self.lexer.next()?;
                if self.lexer.peek()? == &Token::RParen {
                    break;
                }
                args.push(self.parse_expr(0)?);
            }
        }
        self.expect(Token::RParen)?;
        Ok(Expr::Call { func, args })
    }

    fn parse_aggregation(&mut self, op: AggregateOp) -> MetricsResult<Expr> {
        // Check for "by/without" BEFORE the paren (e.g., sum by (label) (...))
        let grouping = if matches!(self.lexer.peek()?, Token::By | Token::Without) {
            self.parse_grouping()?
        } else {
            Grouping::default()
        };
        self.expect(Token::LParen)?;
        // For topk/bottomk/quantile, first arg is the param
        let (param, expr) = match op {
            AggregateOp::Topk | AggregateOp::Bottomk | AggregateOp::Quantile => {
                let p = self.parse_expr(0)?;
                self.expect(Token::Comma)?;
                let e = self.parse_expr(0)?;
                (Some(Box::new(p)), e)
            }
            AggregateOp::CountValues => {
                let p = self.parse_expr(0)?;
                self.expect(Token::Comma)?;
                let e = self.parse_expr(0)?;
                (Some(Box::new(p)), e)
            }
            _ => {
                let e = self.parse_expr(0)?;
                (None, e)
            }
        };
        self.expect(Token::RParen)?;
        // Check for "by/without" AFTER the paren (e.g., sum(...) by (label))
        let grouping = if grouping.labels.is_empty() && matches!(self.lexer.peek()?, Token::By | Token::Without) {
            self.parse_grouping()?
        } else {
            grouping
        };
        Ok(Expr::Aggregate {
            op,
            expr: Box::new(expr),
            param,
            grouping,
        })
    }

    fn parse_grouping(&mut self) -> MetricsResult<Grouping> {
        let by = match self.lexer.next()? {
            Token::By => true,
            Token::Without => false,
            t => return Err(MetricsError::Parse(format!("expected by/without, got {:?}", t))),
        };
        let labels = self.parse_label_list()?;
        Ok(Grouping { by, labels, specified: true })
    }

    fn parse_label_list(&mut self) -> MetricsResult<Vec<String>> {
        self.expect(Token::LParen)?;
        let mut labels = Vec::new();
        if self.lexer.peek()? != &Token::RParen {
            labels.push(self.expect_ident()?);
            while self.lexer.peek()? == &Token::Comma {
                self.lexer.next()?;
                if self.lexer.peek()? == &Token::RParen {
                    break;
                }
                labels.push(self.expect_ident()?);
            }
        }
        self.expect(Token::RParen)?;
        Ok(labels)
    }

    fn parse_label_matchers(&mut self) -> MetricsResult<Vec<LabelMatcher>> {
        let mut matchers = Vec::new();
        while self.lexer.peek()? != &Token::RBrace {
            let name = self.expect_ident()?;
            let tok = self.lexer.next()?;
            let (is_regex, is_neg) = match &tok {
                Token::Assign => (false, false),
                Token::Eq => (false, false),
                Token::NotEq => (false, true),
                Token::EqTilde => (true, false),
                Token::NotEqTilde => (true, true),
                t => return Err(MetricsError::Parse(format!("expected matcher op, got {:?}", t))),
            };
            let value = match self.lexer.next()? {
                Token::Str(s) => s,
                Token::Ident(s) => s,
                t => return Err(MetricsError::Parse(format!("expected string value, got {:?}", t))),
            };
            let m = match (is_regex, is_neg) {
                (false, false) => LabelMatcher::Equal { name, value },
                (false, true) => LabelMatcher::NotEqual { name, value },
                (true, false) => LabelMatcher::RegexMatch { name, pattern: value },
                (true, true) => LabelMatcher::RegexNotMatch { name, pattern: value },
            };
            matchers.push(m);
            if self.lexer.peek()? == &Token::Comma {
                self.lexer.next()?;
            }
        }
        Ok(matchers)
    }

    fn parse_offset_at(&mut self) -> MetricsResult<(Option<i64>, Option<i64>)> {
        let mut offset = None;
        let mut at = None;
        loop {
            match self.lexer.peek()? {
                Token::Offset => {
                    self.lexer.next()?;
                    offset = Some(self.parse_duration()?);
                }
                Token::At => {
                    self.lexer.next()?;
                    let ts = match self.lexer.next()? {
                        Token::Number(n) => n as i64,
                        t => return Err(MetricsError::Parse(format!("expected timestamp after @, got {:?}", t))),
                    };
                    at = Some(ts * 1000); // convert to ms
                }
                _ => break,
            }
        }
        Ok((offset, at))
    }

    fn parse_duration(&mut self) -> MetricsResult<i64> {
        match self.lexer.next()? {
            Token::Duration(ms) => Ok(ms),
            Token::Number(n) => Ok(n as i64), // bare number treated as seconds? Actually ms
            t => Err(MetricsError::Parse(format!("expected duration, got {:?}", t))),
        }
    }

    fn expect(&mut self, expected: Token) -> MetricsResult<()> {
        let got = self.lexer.next()?;
        if got != expected {
            Err(MetricsError::Parse(format!("expected {:?}, got {:?}", expected, got)))
        } else {
            Ok(())
        }
    }

    fn expect_ident(&mut self) -> MetricsResult<String> {
        match self.lexer.next()? {
            Token::Ident(s) => Ok(s),
            // Keywords that can also be label names
            Token::By => Ok("by".to_string()),
            Token::On => Ok("on".to_string()),
            t => Err(MetricsError::Parse(format!("expected identifier, got {:?}", t))),
        }
    }
}

fn parse_aggregate_op(s: &str) -> Option<AggregateOp> {
    match s {
        "sum" => Some(AggregateOp::Sum),
        "avg" => Some(AggregateOp::Avg),
        "min" => Some(AggregateOp::Min),
        "max" => Some(AggregateOp::Max),
        "count" => Some(AggregateOp::Count),
        "stddev" => Some(AggregateOp::Stddev),
        "stdvar" => Some(AggregateOp::Stdvar),
        "quantile" => Some(AggregateOp::Quantile),
        "topk" => Some(AggregateOp::Topk),
        "bottomk" => Some(AggregateOp::Bottomk),
        "count_values" => Some(AggregateOp::CountValues),
        _ => None,
    }
}
