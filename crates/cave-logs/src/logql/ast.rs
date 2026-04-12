//! LogQL abstract syntax tree.

use std::time::Duration;

// ─── Top-level expression ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    /// Log stream selector + optional pipeline stages.
    Log(LogStreamExpr),
    /// Metric expression (range agg, vector agg).
    Metric(MetricExpr),
}

// ─── Log stream ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LogStreamExpr {
    pub matchers: Vec<crate::models::LabelMatcher>,
    pub pipeline: Vec<PipelineStage>,
}

// ─── Pipeline stages ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum PipelineStage {
    Filter(FilterStage),
    Parser(ParserStage),
    LabelFilter(LabelFilterStage),
    LineFormat(String),
    LabelFmt(Vec<(String, String)>), // rename: (from, to)
    Decolorize,
}

// ── Filter ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct FilterStage {
    pub op: FilterOp,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FilterOp {
    Contains,    // |=
    NotContains, // !=
    Re,          // |~
    NotRe,       // !~
}

// ── Parser ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum ParserStage {
    Json,
    Logfmt,
    Regexp(String),
    Pattern(String),
    Unpack,
}

// ── Label filter ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LabelFilterStage {
    pub label: String,
    pub op: LabelFilterOp,
    pub value: LabelFilterValue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LabelFilterOp {
    Eq,
    Ne,
    Gt,
    Gte,
    Lt,
    Lte,
    Re,
    NRe,
}

#[derive(Debug, Clone)]
pub enum LabelFilterValue {
    String(String),
    Float(f64),
    Duration(Duration),
}

// ─── Metric expressions ───────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MetricExpr {
    RangeAgg(RangeAggExpr),
    VectorAgg(VectorAggExpr),
}

#[derive(Debug, Clone)]
pub struct RangeAggExpr {
    pub op: RangeAggOp,
    pub stream: LogStreamExpr,
    pub range: Duration,
    pub grouping: Option<Grouping>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RangeAggOp {
    Rate,
    CountOverTime,
    BytesOverTime,
    BytesRate,
    SumOverTime(String),          // sum_over_time(... | unwrap <label>)
    AvgOverTime(String),
    MaxOverTime(String),
    MinOverTime(String),
    FirstOverTime(String),
    LastOverTime(String),
    QuantileOverTime(String, u32), // quantile_over_time(q, ... | unwrap <label>)
}

#[derive(Debug, Clone)]
pub struct VectorAggExpr {
    pub op: VectorAggOp,
    pub expr: Box<MetricExpr>,
    pub grouping: Option<Grouping>,
    pub param: Option<u64>, // for topk / bottomk / quantile_over_time
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VectorAggOp {
    Sum,
    Avg,
    Max,
    Min,
    Count,
    Topk,
    Bottomk,
}

#[derive(Debug, Clone)]
pub struct Grouping {
    pub by: bool, // true = "by (…)", false = "without (…)"
    pub labels: Vec<String>,
}
