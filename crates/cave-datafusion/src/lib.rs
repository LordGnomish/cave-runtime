// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CAVE DATAFUSION — Sovereign Apache DataFusion query engine.
//!
//! Upstream: `apache/datafusion` 53.1.0 (`source_sha = eae7bf4f`).
//! DataFusion is the Rust query-engine framework — logical plan AST,
//! SQL parser, physical plan, vectorized executor, function registry,
//! and a TableProvider trait for plugging in data sources.
//!
//! This MVP carries the *engine shape*: LogicalPlan + LogicalExpr +
//! DataFrame builder, a minimal SQL parser/planner subset, a physical
//! plan with scan/filter/project/aggregate/sort/limit/join nodes, an
//! in-memory row-batch executor, CSV + JSON in-memory table providers,
//! a SQL function registry, and an execution context. Full SQL grammar
//! coverage, distributed execution (Ballista), and most physical
//! optimizers are deferred to `[[scope_cuts]]`.

pub mod catalog;
pub mod context;
pub mod data_source;
pub mod dataframe;
pub mod error;
pub mod functions;
pub mod logical_expr;
pub mod logical_plan;
pub mod physical_expr;
pub mod physical_plan;
pub mod row;
pub mod schema;
pub mod sql_parser;

pub use catalog::SessionCatalog;
pub use context::SessionContext;
pub use data_source::{CsvSource, MemTable, TableProvider};
pub use dataframe::DataFrame;
pub use error::{Error, Result};
pub use functions::FunctionRegistry;
pub use logical_expr::{BinaryOp, LogicalExpr};
pub use logical_plan::{JoinKind, LogicalPlan};
pub use physical_expr::{BinaryPhysicalOp, PhysicalExpr};
pub use physical_plan::{ExecutionPlan, PhysicalPlan};
pub use row::{Row, Value};
pub use schema::{DataType, Field, SchemaRef, TableSchema};
pub use sql_parser::parse_sql;

pub const UPSTREAM: &str = "apache/datafusion";
pub const UPSTREAM_VERSION: &str = "53.1.0";
