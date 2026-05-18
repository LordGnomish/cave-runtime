// SPDX-License-Identifier: AGPL-3.0-or-later
//! cave-datafusion — DataFusion-style logical + execution plans.
//!
//! Mirrors apache/datafusion v40 logical-plan / physical-plan layering:
//!
//!   * `LogicalPlan`  — declarative operator tree (Scan, Projection, Filter,
//!                      Limit, Aggregate). Maps to datafusion-expr/src/logical_plan/.
//!   * `Expr`         — column/literal/binary/aggregate expressions.
//!   * `ExecutionPlan` — physical operator with `execute(batch)` semantics
//!                       producing rows. Maps to datafusion-physical-plan.
//!   * `RecordBatch`  — minimal column-major batch (Vec<Column>).
//!
//! Primary citation: apache/datafusion v40 datafusion-expr/src/logical_plan/plan.rs
//! and datafusion-physical-plan/src/{projection,filter,limit,aggregates}.rs.

pub mod error;
pub mod tenant;
pub mod expr;
pub mod batch;
pub mod logical_plan;
pub mod execution_plan;
