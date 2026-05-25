// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Query planner — AST to logical plan to physical plan.

use crate::sql::ast::*;

#[derive(Debug, Clone)]
pub enum LogicalPlan {
    TableScan {
        table: String,
    },
    Filter {
        input: Box<LogicalPlan>,
        predicate: Expr,
    },
    Project {
        input: Box<LogicalPlan>,
        exprs: Vec<Expr>,
    },
    Limit {
        input: Box<LogicalPlan>,
        count: i64,
    },
    Sort {
        input: Box<LogicalPlan>,
        order_by: Vec<OrderBy>,
    },
    GroupBy {
        input: Box<LogicalPlan>,
        group_exprs: Vec<Expr>,
        agg_exprs: Vec<(String, Vec<Expr>)>,
    },
    Join {
        left: Box<LogicalPlan>,
        right: Box<LogicalPlan>,
        kind: JoinKind,
        condition: Option<Expr>,
    },
    Values {
        rows: Vec<Vec<Expr>>,
    },
}

#[derive(Debug, Clone)]
pub enum PhysicalPlan {
    TableScan {
        table: String,
    },
    Filter {
        input: Box<PhysicalPlan>,
        predicate: Expr,
    },
    Project {
        input: Box<PhysicalPlan>,
        exprs: Vec<Expr>,
    },
    Limit {
        input: Box<PhysicalPlan>,
        count: i64,
        offset: Option<i64>,
    },
    Sort {
        input: Box<PhysicalPlan>,
        order_by: Vec<OrderBy>,
    },
    GroupBy {
        input: Box<PhysicalPlan>,
        group_exprs: Vec<Expr>,
        agg_exprs: Vec<(String, Vec<Expr>)>,
    },
    NestedLoopJoin {
        left: Box<PhysicalPlan>,
        right: Box<PhysicalPlan>,
        kind: JoinKind,
        condition: Option<Expr>,
    },
    Values {
        rows: Vec<Vec<Expr>>,
    },
}

pub struct Planner;

impl Planner {
    pub fn plan(stmt: &Statement) -> Result<PhysicalPlan, String> {
        match stmt {
            Statement::Select(select) => Self::plan_select(select),
            Statement::Insert(insert) => Self::plan_insert(insert),
            Statement::Update(update) => Self::plan_update(update),
            Statement::Delete(delete) => Self::plan_delete(delete),
            Statement::CreateTable(_) => Ok(PhysicalPlan::Values { rows: vec![] }),
            Statement::DropTable(_) => Ok(PhysicalPlan::Values { rows: vec![] }),
            Statement::CreateIndex(_) => Ok(PhysicalPlan::Values { rows: vec![] }),
            Statement::DropIndex(_) => Ok(PhysicalPlan::Values { rows: vec![] }),
            Statement::CreateSchema(_) => Ok(PhysicalPlan::Values { rows: vec![] }),
            Statement::AlterTable(_) => Ok(PhysicalPlan::Values { rows: vec![] }),
            _ => Ok(PhysicalPlan::Values { rows: vec![] }),
        }
    }

    fn plan_select(select: &SelectStmt) -> Result<PhysicalPlan, String> {
        let mut plan = if let Some(from) = &select.from {
            Self::plan_from(from)?
        } else {
            PhysicalPlan::Values { rows: vec![vec![]] }
        };

        if let Some(where_clause) = &select.where_clause {
            plan = PhysicalPlan::Filter {
                input: Box::new(plan),
                predicate: where_clause.as_ref().clone(),
            };
        }

        if let Some(group_by) = &select.group_by {
            let agg_exprs = vec![];
            plan = PhysicalPlan::GroupBy {
                input: Box::new(plan),
                group_exprs: group_by.clone(),
                agg_exprs,
            };
        }

        if let Some(order_by) = &select.order_by {
            plan = PhysicalPlan::Sort {
                input: Box::new(plan),
                order_by: order_by.clone(),
            };
        }

        if let Some(limit) = select.limit {
            plan = PhysicalPlan::Limit {
                input: Box::new(plan),
                count: limit,
                offset: select.offset,
            };
        }

        let cols = Self::select_columns_to_exprs(&select.columns);
        plan = PhysicalPlan::Project {
            input: Box::new(plan),
            exprs: cols,
        };

        Ok(plan)
    }

    fn plan_from(from: &FromClause) -> Result<PhysicalPlan, String> {
        match from {
            FromClause::Table(table, _alias) => Ok(PhysicalPlan::TableScan {
                table: table.clone(),
            }),
            FromClause::Join {
                left,
                kind,
                right,
                on,
            } => {
                let left_plan = Self::plan_from(left)?;
                let right_plan = Self::plan_from(right)?;
                Ok(PhysicalPlan::NestedLoopJoin {
                    left: Box::new(left_plan),
                    right: Box::new(right_plan),
                    kind: *kind,
                    condition: on.as_ref().map(|b| b.as_ref().clone()),
                })
            }
        }
    }

    fn select_columns_to_exprs(columns: &[SelectColumn]) -> Vec<Expr> {
        columns
            .iter()
            .map(|col| match col {
                SelectColumn::Star => Expr::Identifier("*".to_string()),
                SelectColumn::TableStar(t) => Expr::QualifiedIdentifier(t.clone(), "*".to_string()),
                SelectColumn::Expr(expr, _alias) => expr.clone(),
            })
            .collect()
    }

    fn plan_insert(_insert: &InsertStmt) -> Result<PhysicalPlan, String> {
        Ok(PhysicalPlan::Values { rows: vec![] })
    }

    fn plan_update(_update: &UpdateStmt) -> Result<PhysicalPlan, String> {
        Ok(PhysicalPlan::Values { rows: vec![] })
    }

    fn plan_delete(_delete: &DeleteStmt) -> Result<PhysicalPlan, String> {
        Ok(PhysicalPlan::Values { rows: vec![] })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_planner_simple_select() {
        let select = SelectStmt {
            distinct: false,
            columns: vec![SelectColumn::Star],
            from: Some(Box::new(FromClause::Table("users".to_string(), None))),
            where_clause: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: None,
            offset: None,
        };
        let plan = Planner::plan(&Statement::Select(select));
        assert!(plan.is_ok());
    }

    #[test]
    fn test_planner_select_with_limit() {
        let select = SelectStmt {
            distinct: false,
            columns: vec![SelectColumn::Star],
            from: Some(Box::new(FromClause::Table("users".to_string(), None))),
            where_clause: None,
            group_by: None,
            having: None,
            order_by: None,
            limit: Some(10),
            offset: None,
        };
        let plan = Planner::plan(&Statement::Select(select));
        assert!(plan.is_ok());
    }
}
