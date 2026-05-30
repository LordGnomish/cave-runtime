// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! LogQL Abstract Syntax Tree.
//!
//! Covers the full LogQL grammar:
//!   - Stream selectors with all four matcher operators
//!   - Pipeline stages: filters, parsers, label filters, formatters
//!   - Log range aggregations and vector aggregations
//!   - Binary operations between metric queries

use super::ip::IpPattern;
use std::time::Duration;

/// A label matcher operator.
#[derive(Debug, Clone, PartialEq)]
pub enum MatchOp {
    /// `=`
    Eq,
    /// `!=`
    Neq,
    /// `=~`
    Re,
    /// `!~`
    NotRe,
}

/// A single label matcher, e.g. `app="nginx"` or `env=~"prod.*"`.
#[derive(Debug, Clone)]
pub struct LabelMatcher {
    pub name: String,
    pub op: MatchOp,
    pub value: String,
}

/// A stream selector: `{matchers...}`
#[derive(Debug, Clone)]
pub struct StreamSelector {
    pub matchers: Vec<LabelMatcher>,
}

// ── Pipeline stages ──────────────────────────────────────────────────────────

/// Line filter expression stage.
#[derive(Debug, Clone)]
pub enum LineFilter {
    /// `|= "text"`
    Contains(String),
    /// `!= "text"`
    NotContains(String),
    /// `|~ "regex"`
    Matches(String),
    /// `!~ "regex"`
    NotMatches(String),
    /// `|= ip("…")` — line contains an address matching the pattern.
    IpMatch(IpPattern),
    /// `!= ip("…")` — line contains no address matching the pattern.
    IpNotMatch(IpPattern),
}

/// Log parser stage.
#[derive(Debug, Clone)]
pub enum Parser {
    /// `| json`
    Json,
    /// `| logfmt`
    Logfmt,
    /// `| regexp "pattern"`
    Regexp(String),
    /// `| pattern "pattern"`
    Pattern(String),
    /// `| unpack`
    Unpack,
}

/// Comparison operator used in label filters.
#[derive(Debug, Clone, PartialEq)]
pub enum CompareOp {
    Eq,
    Neq,
    Gt,
    Gte,
    Lt,
    Lte,
    Re,
    NotRe,
}

/// Label filter value.
#[derive(Debug, Clone)]
pub enum LabelFilterValue {
    String(String),
    Float(f64),
    Duration(Duration),
    Bytes(u64),
}

/// Label filter stage: `| status_code >= 400`
#[derive(Debug, Clone)]
pub struct LabelFilter {
    pub label: String,
    pub op: CompareOp,
    pub value: LabelFilterValue,
}

/// Line format stage: `| line_format "{{.method}} {{.status}}"`
#[derive(Debug, Clone)]
pub struct LineFormat {
    pub template: String,
}

/// Label format stage: `| label_format new_label=old_label`
#[derive(Debug, Clone)]
pub struct LabelFormat {
    /// (new_name, old_name_or_template)
    pub mappings: Vec<(String, String)>,
}

/// Decolorize: strip ANSI escape codes from log lines.
#[derive(Debug, Clone)]
pub struct Decolorize;

/// Unwrap expression: `| unwrap <label>` — extracts a numeric value from a label.
#[derive(Debug, Clone)]
pub struct UnwrapExpr {
    pub label: String,
    /// Optional conversion: `| unwrap duration(label)` or `| unwrap bytes(label)`
    pub converter: Option<String>,
}

/// One entry in a `| drop` or `| keep` list: either a bare label name or a
/// label name guarded by a value matcher (`level="debug"`, `path=~"/api/.*"`).
#[derive(Debug, Clone)]
pub enum DropKeepLabel {
    /// `drop foo` / `keep foo` — acts on the label unconditionally.
    Name(String),
    /// `drop foo="bar"` / `keep foo=~"re"` — acts only when the matcher passes.
    Matcher(LabelMatcher),
}

/// Drop labels stage: `| drop level, status="500"` — removes matching labels.
#[derive(Debug, Clone)]
pub struct DropLabels {
    pub labels: Vec<DropKeepLabel>,
}

/// Keep labels stage: `| keep level, status="500"` — retains only matching
/// labels and removes everything else.
#[derive(Debug, Clone)]
pub struct KeepLabels {
    pub labels: Vec<DropKeepLabel>,
}

/// A single stage in the log pipeline.
#[derive(Debug, Clone)]
pub enum PipelineStage {
    LineFilter(LineFilter),
    Parser(Parser),
    LabelFilter(LabelFilter),
    LineFormat(LineFormat),
    LabelFormat(LabelFormat),
    Decolorize(Decolorize),
    Unwrap(UnwrapExpr),
    Drop(DropLabels),
    Keep(KeepLabels),
}

// ── Log query ────────────────────────────────────────────────────────────────

/// A log stream query: selector + pipeline.
#[derive(Debug, Clone)]
pub struct LogQuery {
    pub selector: StreamSelector,
    pub pipeline: Vec<PipelineStage>,
}

// ── Range aggregations ───────────────────────────────────────────────────────

/// Range aggregation function.
#[derive(Debug, Clone, PartialEq)]
pub enum RangeAgg {
    Rate,
    CountOverTime,
    BytesOverTime,
    BytesRate,
    AbsentOverTime,
    SumOverTime,
    AvgOverTime,
    MaxOverTime,
    MinOverTime,
    FirstOverTime,
    LastOverTime,
    StddevOverTime,
    StdvarOverTime,
    QuantileOverTime(f64),
}

/// A log range aggregation: `rate({job="x"}[5m])`
#[derive(Debug, Clone)]
pub struct LogRangeAggregation {
    pub agg: RangeAgg,
    pub query: LogQuery,
    pub range: Duration,
    /// Optional `by(...)` grouping for unwrap expressions.
    pub grouping: Option<Grouping>,
    /// Optional `offset <duration>` modifier — shifts the lookup window back.
    pub offset: Option<Duration>,
}

// ── Vector aggregations ──────────────────────────────────────────────────────

/// Aggregation operator over vectors of series.
#[derive(Debug, Clone, PartialEq)]
pub enum VectorAgg {
    Sum,
    Avg,
    Max,
    Min,
    Count,
    Stddev,
    Stdvar,
    Topk(u64),
    Bottomk(u64),
    Quantile(f64),
}

/// `by(label1, label2, ...)` or `without(label1, ...)`
#[derive(Debug, Clone)]
pub struct Grouping {
    pub without: bool,
    pub labels: Vec<String>,
}

/// A vector aggregation over a metric query.
#[derive(Debug, Clone)]
pub struct VectorAggregation {
    pub agg: VectorAgg,
    pub grouping: Option<Grouping>,
    pub inner: Box<MetricQuery>,
}

// ── Binary operations ────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Pow,
    And,
    Or,
    Unless,
    /// Comparison operators (may be bool-returning)
    CmpEq(bool),
    CmpNeq(bool),
    CmpGt(bool),
    CmpGte(bool),
    CmpLt(bool),
    CmpLte(bool),
}

#[derive(Debug, Clone)]
pub struct BinaryExpr {
    pub op: BinOp,
    pub lhs: Box<MetricQuery>,
    pub rhs: Box<MetricQuery>,
    pub grouping: Option<VectorMatchGrouping>,
}

#[derive(Debug, Clone)]
pub struct VectorMatchGrouping {
    pub card: MatchCardinality,
    pub labels: Vec<String>,
    pub include: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MatchCardinality {
    OneToOne,
    ManyToOne,
    OneToMany,
}

// ── Top-level query node ─────────────────────────────────────────────────────

/// A metric query (produces a time-series / vector).
#[derive(Debug, Clone)]
pub enum MetricQuery {
    RangeAgg(LogRangeAggregation),
    VectorAgg(VectorAggregation),
    BinaryExpr(BinaryExpr),
    Literal(f64),
}

/// Top-level query — either a log query or a metric query.
#[derive(Debug, Clone)]
pub enum Query {
    Log(LogQuery),
    Metric(MetricQuery),
}
