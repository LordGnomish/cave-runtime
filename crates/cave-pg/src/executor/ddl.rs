//! DDL executor — CREATE/DROP/ALTER TABLE, INDEX, VIEW, SCHEMA, SEQUENCE, TYPE, DATABASE.

use std::collections::HashMap;
use sqlparser::ast::{self as ast, Statement, DataType, ColumnOptionDef, ColumnOption, ObjectType,
    ReferentialAction, AlterTableOperation, CreateTableOptions};
use crate::error::{Error, PgError, Result, SqlState};
use crate::executor::Executor;
use crate::storage::{alloc_oid};
use crate::storage::heap::{ColumnDef, Constraint, FkAction, Index, IndexMethod, Sequence, Table, View};
use crate::types::{oid, CommandResult, PgValue};

// ─────────────────────────────────────────────────────────────────────────────
// CREATE TABLE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_table(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (name, columns, constraints, if_not_exists, or_replace, temporary, like_table) = match stmt {
        Statement::CreateTable(ct) => {
            let like = ct.like.clone();
            (ct.name, ct.columns, ct.constraints, ct.if_not_exists, ct.or_replace, ct.temporary, like)
        }
        _ => unreachable!(),
    };

    let full_name = name.to_string();
    let (schema_name, table_name) = split_schema_table(&full_name, exec);

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;

    // Check for IF NOT EXISTS
    if if_not_exists {
        let search_path = exec.config.search_path_refs().iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let search_refs: Vec<&str> = search_path.iter().map(String::as_str).collect();
        if db.resolve_table(&table_name, &[&schema_name]).is_some()
            || db.resolve_table(&table_name, &search_refs).is_some()
        {
            return Ok(CommandResult::Created("CREATE TABLE".to_string()));
        }
    }

    let table_oid = alloc_oid();

    // Convert column definitions
    let mut col_defs: Vec<ColumnDef> = Vec::new();
    let mut table_constraints: Vec<Constraint> = Vec::new();

    for col in &columns {
        let col_oid = data_type_to_oid(&col.data_type);
        let mut cd = ColumnDef::new(col.name.value.clone(), col_oid);

        // Type modifier for varchar(n), char(n), numeric(p,s)
        cd.type_modifier = data_type_modifier(&col.data_type);

        // Column options
        for opt in &col.options {
            match &opt.option {
                ColumnOption::NotNull => { cd.not_null = true; }
                ColumnOption::Null => { cd.not_null = false; }
                ColumnOption::Default(expr) => {
                    cd.default_expr = Some(expr.to_string());
                }
                ColumnOption::Unique { is_primary, .. } => {
                    if *is_primary {
                        cd.not_null = true;
                        table_constraints.push(Constraint::PrimaryKey {
                            name: format!("{}_pkey", table_name),
                            columns: vec![col.name.value.clone()],
                        });
                    } else {
                        table_constraints.push(Constraint::Unique {
                            name: opt.name.as_ref().map(|n| n.value.clone())
                                .unwrap_or_else(|| format!("{}_{}_key", table_name, col.name.value)),
                            columns: vec![col.name.value.clone()],
                        });
                    }
                }
                ColumnOption::ForeignKey {
                    foreign_table,
                    referred_columns,
                    on_delete,
                    on_update,
                    ..
                } => {
                    table_constraints.push(Constraint::ForeignKey {
                        name: opt.name.as_ref().map(|n| n.value.clone())
                            .unwrap_or_else(|| format!("{}_{}_fkey", table_name, col.name.value)),
                        columns: vec![col.name.value.clone()],
                        ref_table: foreign_table.to_string(),
                        ref_columns: referred_columns.iter().map(|c| c.value.clone()).collect(),
                        on_delete: referential_action_to_fk(on_delete),
                        on_update: referential_action_to_fk(on_update),
                    });
                }
                ColumnOption::Check(expr) => {
                    table_constraints.push(Constraint::Check {
                        name: opt.name.as_ref().map(|n| n.value.clone())
                            .unwrap_or_else(|| format!("{}_check", table_name)),
                        expr: expr.to_string(),
                    });
                }
                ColumnOption::Generated { sequence_options, generation_expr, .. } => {
                    // GENERATED ALWAYS AS IDENTITY → create implicit sequence
                    let seq_name = format!("{}_{}_seq", table_name, col.name.value);
                    cd.default_expr = Some(format!("nextval('{}.{}')", schema_name, seq_name));
                    // Will create sequence below after table is created
                }
                _ => {}
            }
        }
        col_defs.push(cd);
    }

    // Table-level constraints
    for tc in &constraints {
        match tc {
            ast::TableConstraint::PrimaryKey { name, columns, .. } => {
                table_constraints.push(Constraint::PrimaryKey {
                    name: name.as_ref().map(|n| n.value.clone())
                        .unwrap_or_else(|| format!("{}_pkey", table_name)),
                    columns: columns.iter().map(|c| c.value.clone()).collect(),
                });
            }
            ast::TableConstraint::Unique { name, columns, .. } => {
                table_constraints.push(Constraint::Unique {
                    name: name.as_ref().map(|n| n.value.clone())
                        .unwrap_or_else(|| format!("{}_unique", table_name)),
                    columns: columns.iter().map(|c| c.value.clone()).collect(),
                });
            }
            ast::TableConstraint::ForeignKey {
                name,
                columns,
                foreign_table,
                referred_columns,
                on_delete,
                on_update,
                ..
            } => {
                table_constraints.push(Constraint::ForeignKey {
                    name: name.as_ref().map(|n| n.value.clone())
                        .unwrap_or_else(|| format!("{}_fkey", table_name)),
                    columns: columns.iter().map(|c| c.value.clone()).collect(),
                    ref_table: foreign_table.to_string(),
                    ref_columns: referred_columns.iter().map(|c| c.value.clone()).collect(),
                    on_delete: referential_action_to_fk(on_delete),
                    on_update: referential_action_to_fk(on_update),
                });
            }
            ast::TableConstraint::Check { name, expr, .. } => {
                table_constraints.push(Constraint::Check {
                    name: name.as_ref().map(|n| n.value.clone())
                        .unwrap_or_else(|| format!("{}_check", table_name)),
                    expr: expr.to_string(),
                });
            }
            _ => {}
        }
    }

    let schema_ref = db.schema(&schema_name)
        .ok_or_else(|| Error::Pg(PgError::error(
            SqlState::INVALID_SCHEMA_NAME,
            format!("schema \"{}\" does not exist", schema_name),
        )))?;

    let mut table = Table::new(&table_name, &schema_name, table_oid, col_defs);
    for c in table_constraints {
        table.add_constraint(c.clone());
    }

    let mut schema = schema_ref.write();

    // Check already exists (non-IF-NOT-EXISTS)
    if schema.table(&table_name).is_some() && !if_not_exists {
        return Err(Error::Pg(PgError::error(
            SqlState::DUPLICATE_TABLE,
            format!("relation \"{}\" already exists", table_name),
        )));
    }

    // Create implicit indexes for PK/UNIQUE
    let mut indexes_to_add: Vec<Index> = Vec::new();
    for c in &table.constraints {
        match c {
            Constraint::PrimaryKey { name, columns } => {
                let idx_oid = alloc_oid();
                let idx = Index::new(name.clone(), &schema_name, &table_name, idx_oid,
                    IndexMethod::BTree, columns.clone(), true, true);
                indexes_to_add.push(idx);
            }
            Constraint::Unique { name, columns } => {
                let idx_oid = alloc_oid();
                let idx = Index::new(name.clone(), &schema_name, &table_name, idx_oid,
                    IndexMethod::BTree, columns.clone(), true, false);
                indexes_to_add.push(idx);
            }
            _ => {}
        }
    }

    schema.add_table(table);
    for idx in indexes_to_add {
        schema.add_index(idx);
    }

    Ok(CommandResult::Created("CREATE TABLE".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// CREATE INDEX
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_index(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (name, table_name, columns, unique, if_not_exists, using) = match stmt {
        Statement::CreateIndex(ci) => {
            let name = ci.name.map(|n| n.to_string());
            let unique = ci.unique;
            let if_not_exists = ci.if_not_exists;
            let using = ci.using.map(|u| u.to_string().to_lowercase());
            let table = ci.table_name.to_string();
            let cols: Vec<String> = ci.columns.iter()
                .map(|oe| match &oe.expr {
                    ast::Expr::Identifier(id) => id.value.clone(),
                    other => other.to_string(),
                })
                .collect();
            (name, table, cols, unique, if_not_exists, using)
        }
        _ => unreachable!(),
    };

    let (schema_name, actual_table) = split_schema_table(&table_name, exec);
    let idx_name = name.unwrap_or_else(|| {
        format!("{}_{}_idx", actual_table, columns.join("_"))
    });

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let schema_ref = db.schema(&schema_name).ok_or_else(|| {
        Error::Pg(PgError::error(SqlState::INVALID_SCHEMA_NAME, format!("schema \"{}\" does not exist", schema_name)))
    })?;

    let method = match using.as_deref() {
        Some("hash") => IndexMethod::Hash,
        Some("gin") => IndexMethod::Gin,
        Some("gist") => IndexMethod::Gist,
        Some("brin") => IndexMethod::Brin,
        Some("spgist") => IndexMethod::SpGist,
        _ => IndexMethod::BTree,
    };

    let idx_oid = alloc_oid();
    let idx = Index::new(idx_name.clone(), &schema_name, &actual_table, idx_oid, method, columns, unique, false);

    let mut schema = schema_ref.write();
    if schema.index(&idx_name).is_some() && !if_not_exists {
        return Err(Error::Pg(PgError::error(
            SqlState::DUPLICATE_OBJECT,
            format!("index \"{}\" already exists", idx_name),
        )));
    }
    schema.add_index(idx);

    Ok(CommandResult::Created("CREATE INDEX".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// CREATE VIEW / MATERIALIZED VIEW
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_view(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (name, query, materialized, or_replace) = match stmt {
        Statement::CreateView { name, query, materialized, or_replace, .. } => {
            (name, query, materialized, or_replace)
        }
        _ => unreachable!(),
    };

    let full_name = name.to_string();
    let (schema_name, view_name) = split_schema_table(&full_name, exec);
    let query_str = query.to_string();

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let schema_ref = db.schema(&schema_name).ok_or_else(|| {
        Error::Pg(PgError::error(SqlState::INVALID_SCHEMA_NAME, format!("schema \"{}\" does not exist", schema_name)))
    })?;

    let view_oid = alloc_oid();
    let mut view = View::new(&view_name, &schema_name, view_oid, query_str);
    if materialized {
        view = view.materialized();
    }

    let mut schema = schema_ref.write();
    if !or_replace && schema.view(&view_name).is_some() {
        return Err(Error::Pg(PgError::error(
            SqlState::DUPLICATE_TABLE,
            format!("view \"{}\" already exists", view_name),
        )));
    }
    schema.add_view(view);

    let tag = if materialized { "CREATE MATERIALIZED VIEW" } else { "CREATE VIEW" };
    Ok(CommandResult::Created(tag.to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// CREATE SCHEMA
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_schema(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (schema_name, if_not_exists) = match stmt {
        Statement::CreateSchema { schema_name, if_not_exists, .. } => {
            (schema_name.to_string(), if_not_exists)
        }
        _ => unreachable!(),
    };

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;

    if db.schema(&schema_name).is_some() {
        if if_not_exists {
            return Ok(CommandResult::Created("CREATE SCHEMA".to_string()));
        }
        return Err(Error::Pg(PgError::error(
            SqlState::DUPLICATE_SCHEMA,
            format!("schema \"{}\" already exists", schema_name),
        )));
    }

    db.create_schema(&schema_name, &exec.config.current_user);
    Ok(CommandResult::Created("CREATE SCHEMA".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// CREATE SEQUENCE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_sequence(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (name, if_not_exists, increment_by, min_value, max_value, start_with, cycle) = match stmt {
        Statement::CreateSequence {
            name,
            if_not_exists,
            sequence_options,
            ..
        } => {
            let mut increment: i64 = 1;
            let mut min_val: Option<i64> = None;
            let mut max_val: Option<i64> = None;
            let mut start: i64 = 1;
            let mut cycle = false;
            for opt in &sequence_options {
                match opt {
                    ast::SequenceOptions::IncrementBy(n, _) => {
                        increment = value_to_i64(n).unwrap_or(1);
                    }
                    ast::SequenceOptions::MinValue(Some(n)) => {
                        min_val = value_to_i64(n);
                    }
                    ast::SequenceOptions::MaxValue(Some(n)) => {
                        max_val = value_to_i64(n);
                    }
                    ast::SequenceOptions::StartWith(n, _) => {
                        start = value_to_i64(n).unwrap_or(1);
                    }
                    ast::SequenceOptions::Cycle(c) => {
                        cycle = *c;
                    }
                    _ => {}
                }
            }
            (name, if_not_exists, increment, min_val, max_val, start, cycle)
        }
        _ => unreachable!(),
    };

    let full_name = name.to_string();
    let (schema_name, seq_name) = split_schema_table(&full_name, exec);
    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let schema_ref = db.schema(&schema_name).ok_or_else(|| {
        Error::Pg(PgError::error(SqlState::INVALID_SCHEMA_NAME, format!("schema \"{}\" does not exist", schema_name)))
    })?;

    {
        let schema = schema_ref.read();
        if schema.sequence(&seq_name).is_some() {
            if if_not_exists {
                return Ok(CommandResult::Created("CREATE SEQUENCE".to_string()));
            }
            return Err(Error::Pg(PgError::error(
                SqlState::DUPLICATE_OBJECT,
                format!("sequence \"{}\" already exists", seq_name),
            )));
        }
    }

    let seq_oid = alloc_oid();
    let min = min_value.unwrap_or(1);
    let max = max_value.unwrap_or(i64::MAX);
    let seq = Sequence::new(&seq_name, &schema_name, seq_oid)
        .with_range(start_with, min, max)
        .with_increment(increment_by);

    schema_ref.write().add_sequence(seq);
    Ok(CommandResult::Created("CREATE SEQUENCE".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// CREATE TYPE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_type(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (name, representation) = match stmt {
        Statement::CreateType { name, representation } => (name, representation),
        _ => unreachable!(),
    };

    let full_name = name.to_string();
    let (schema_name, type_name) = split_schema_table(&full_name, exec);
    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let schema_ref = db.schema(&schema_name).ok_or_else(|| {
        Error::Pg(PgError::error(SqlState::INVALID_SCHEMA_NAME, format!("schema \"{}\" does not exist", schema_name)))
    })?;

    match representation {
        ast::UserDefinedTypeRepresentation::Enum { labels } => {
            let label_strings: Vec<String> = labels.iter().map(|l| l.value.clone()).collect();
            schema_ref.write().enum_types.insert(type_name, label_strings);
        }
        ast::UserDefinedTypeRepresentation::Composite { attributes } => {
            let col_defs: Vec<crate::storage::heap::ColumnDef> = attributes.iter().map(|attr| {
                let type_oid = data_type_to_oid(&attr.data_type);
                ColumnDef::new(attr.name.value.clone(), type_oid)
            }).collect();
            schema_ref.write().composite_types.insert(type_name, col_defs);
        }
        _ => {}
    }

    Ok(CommandResult::Created("CREATE TYPE".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// DROP
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_drop(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (object_type, names, if_exists, cascade) = match stmt {
        Statement::Drop { object_type, names, if_exists, cascade, .. } => {
            (object_type, names, if_exists, cascade)
        }
        _ => unreachable!(),
    };

    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let search_path = exec.config.search_path_refs().iter().map(|s| s.to_string()).collect::<Vec<_>>();
    let search_refs: Vec<&str> = search_path.iter().map(String::as_str).collect();

    for name in &names {
        let full_name = name.to_string();
        let (schema_name, obj_name) = split_schema_table_direct(&full_name, &search_refs);

        match object_type {
            ObjectType::Table => {
                if let Some(schema_ref) = db.schema(&schema_name) {
                    let dropped = schema_ref.write().drop_table(&obj_name);
                    if !dropped && !if_exists {
                        return Err(Error::Pg(PgError::error(
                            SqlState::UNDEFINED_TABLE,
                            format!("table \"{}\" does not exist", obj_name),
                        )));
                    }
                }
            }
            ObjectType::View => {
                if let Some(schema_ref) = db.schema(&schema_name) {
                    let dropped = schema_ref.write().drop_view(&obj_name);
                    if !dropped && !if_exists {
                        return Err(Error::Pg(PgError::error(
                            SqlState::UNDEFINED_TABLE,
                            format!("view \"{}\" does not exist", obj_name),
                        )));
                    }
                }
            }
            ObjectType::Index => {
                if let Some(schema_ref) = db.schema(&schema_name) {
                    let dropped = schema_ref.write().drop_index(&obj_name);
                    if !dropped && !if_exists {
                        return Err(Error::Pg(PgError::error(
                            SqlState::UNDEFINED_OBJECT,
                            format!("index \"{}\" does not exist", obj_name),
                        )));
                    }
                }
            }
            ObjectType::Sequence => {
                if let Some(schema_ref) = db.schema(&schema_name) {
                    let dropped = schema_ref.write().drop_sequence(&obj_name);
                    if !dropped && !if_exists {
                        return Err(Error::Pg(PgError::error(
                            SqlState::UNDEFINED_OBJECT,
                            format!("sequence \"{}\" does not exist", obj_name),
                        )));
                    }
                }
            }
            ObjectType::Schema => {
                if db.schema(&obj_name).is_none() {
                    if !if_exists {
                        return Err(Error::Pg(PgError::error(
                            SqlState::INVALID_SCHEMA_NAME,
                            format!("schema \"{}\" does not exist", obj_name),
                        )));
                    }
                } else {
                    db.schemas.remove(&obj_name.to_lowercase());
                }
            }
            ObjectType::Database => {
                match exec.engine.drop_database(&obj_name) {
                    Ok(()) => {}
                    Err(e) if if_exists => {}
                    Err(e) => return Err(e),
                }
            }
            ObjectType::Role => {} // Roles not fully implemented
            _ => {} // Other types not supported
        }
    }

    let tag = match object_type {
        ObjectType::Table => "DROP TABLE",
        ObjectType::View => "DROP VIEW",
        ObjectType::Index => "DROP INDEX",
        ObjectType::Sequence => "DROP SEQUENCE",
        ObjectType::Schema => "DROP SCHEMA",
        ObjectType::Database => "DROP DATABASE",
        ObjectType::Role => "DROP ROLE",
        _ => "DROP",
    };
    Ok(CommandResult::Dropped(tag.to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// ALTER TABLE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_alter_table(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (name, operations) = match stmt {
        Statement::AlterTable { name, operations, .. } => (name, operations),
        _ => unreachable!(),
    };

    let full_name = name.to_string();
    let (schema_name, table_name) = split_schema_table(&full_name, exec);
    let db = exec.db().ok_or_else(|| Error::Pg(PgError::error(SqlState::UNDEFINED_DATABASE, "no current database")))?;
    let schema_ref = db.schema(&schema_name).ok_or_else(|| {
        Error::Pg(PgError::error(SqlState::INVALID_SCHEMA_NAME, format!("schema \"{}\" does not exist", schema_name)))
    })?;

    for op in &operations {
        match op {
            AlterTableOperation::AddColumn { column_def, .. } => {
                let type_oid = data_type_to_oid(&column_def.data_type);
                let mut cd = ColumnDef::new(column_def.name.value.clone(), type_oid);
                cd.type_modifier = data_type_modifier(&column_def.data_type);
                for opt in &column_def.options {
                    match &opt.option {
                        ColumnOption::NotNull => { cd.not_null = true; }
                        ColumnOption::Default(e) => { cd.default_expr = Some(e.to_string()); }
                        _ => {}
                    }
                }
                let schema = schema_ref.read();
                let table_arc = schema.table(&table_name).ok_or_else(|| {
                    Error::Pg(PgError::undefined_table(&table_name))
                })?;
                let mut table = table_arc.write();
                let attr_num = (table.columns.len() + 1) as i16;
                cd.attr_num = attr_num;
                table.columns.push(cd);
                // Extend existing tuples with NULL for new column
                for tuple in &mut table.tuples {
                    tuple.data.push(None);
                }
            }
            AlterTableOperation::DropColumn { column_name, if_exists, .. } => {
                let schema = schema_ref.read();
                let table_arc = schema.table(&table_name).ok_or_else(|| {
                    Error::Pg(PgError::undefined_table(&table_name))
                })?;
                let mut table = table_arc.write();
                let col_lower = column_name.value.to_lowercase();
                if let Some(pos) = table.column_idx(&col_lower) {
                    table.columns.remove(pos);
                    for tuple in &mut table.tuples {
                        if pos < tuple.data.len() {
                            tuple.data.remove(pos);
                        }
                    }
                } else if !if_exists {
                    return Err(Error::Pg(PgError::error(
                        SqlState::UNDEFINED_COLUMN,
                        format!("column \"{}\" of relation \"{}\" does not exist", column_name.value, table_name),
                    )));
                }
            }
            AlterTableOperation::RenameColumn { old_column_name, new_column_name } => {
                let schema = schema_ref.read();
                let table_arc = schema.table(&table_name).ok_or_else(|| {
                    Error::Pg(PgError::undefined_table(&table_name))
                })?;
                let mut table = table_arc.write();
                let old_lower = old_column_name.value.to_lowercase();
                if let Some(col) = table.columns.iter_mut().find(|c| c.name.to_lowercase() == old_lower) {
                    col.name = new_column_name.value.clone();
                } else {
                    return Err(Error::Pg(PgError::error(
                        SqlState::UNDEFINED_COLUMN,
                        format!("column \"{}\" does not exist", old_column_name.value),
                    )));
                }
            }
            AlterTableOperation::RenameTable { table_name: new_name } => {
                let mut schema = schema_ref.write();
                let old_lower = table_name.to_lowercase();
                if let Some(table_arc) = schema.tables.remove(&old_lower) {
                    let new_str = new_name.to_string();
                    let new_lower = new_str.to_lowercase();
                    table_arc.write().name = new_str.clone();
                    schema.tables.insert(new_lower, table_arc);
                }
            }
            AlterTableOperation::AddConstraint(tc) => {
                let schema = schema_ref.read();
                let table_arc = schema.table(&table_name).ok_or_else(|| {
                    Error::Pg(PgError::undefined_table(&table_name))
                })?;
                let mut table = table_arc.write();
                match tc {
                    ast::TableConstraint::PrimaryKey { name, columns, .. } => {
                        table.add_constraint(Constraint::PrimaryKey {
                            name: name.as_ref().map(|n| n.value.clone()).unwrap_or_else(|| format!("{}_pkey", table_name)),
                            columns: columns.iter().map(|c| c.value.clone()).collect(),
                        });
                    }
                    ast::TableConstraint::Unique { name, columns, .. } => {
                        table.add_constraint(Constraint::Unique {
                            name: name.as_ref().map(|n| n.value.clone()).unwrap_or_else(|| format!("{}_unique", table_name)),
                            columns: columns.iter().map(|c| c.value.clone()).collect(),
                        });
                    }
                    _ => {}
                }
            }
            AlterTableOperation::DropConstraint { name, if_exists, .. } => {
                let schema = schema_ref.read();
                let table_arc = schema.table(&table_name).ok_or_else(|| {
                    Error::Pg(PgError::undefined_table(&table_name))
                })?;
                let mut table = table_arc.write();
                let n = name.value.to_lowercase();
                let before = table.constraints.len();
                table.constraints.retain(|c| c.name().to_lowercase() != n);
                if table.constraints.len() == before && !if_exists {
                    return Err(Error::Pg(PgError::error(
                        SqlState::UNDEFINED_OBJECT,
                        format!("constraint \"{}\" of relation \"{}\" does not exist", name.value, table_name),
                    )));
                }
            }
            AlterTableOperation::AlterColumn { column_name, op } => {
                let schema = schema_ref.read();
                let table_arc = schema.table(&table_name).ok_or_else(|| {
                    Error::Pg(PgError::undefined_table(&table_name))
                })?;
                let mut table = table_arc.write();
                let col_lower = column_name.value.to_lowercase();
                if let Some(col) = table.columns.iter_mut().find(|c| c.name.to_lowercase() == col_lower) {
                    match op {
                        ast::AlterColumnOperation::SetNotNull => { col.not_null = true; }
                        ast::AlterColumnOperation::DropNotNull => { col.not_null = false; }
                        ast::AlterColumnOperation::SetDefault { value } => {
                            col.default_expr = Some(value.to_string());
                        }
                        ast::AlterColumnOperation::DropDefault => {
                            col.default_expr = None;
                        }
                        ast::AlterColumnOperation::SetDataType { data_type, .. } => {
                            col.type_oid = data_type_to_oid(data_type);
                        }
                        _ => {}
                    }
                }
            }
            _ => {} // Unsupported ALTER TABLE operations
        }
    }

    Ok(CommandResult::Altered("ALTER TABLE".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// CREATE DATABASE / DROP DATABASE
// ─────────────────────────────────────────────────────────────────────────────

pub fn execute_create_database(exec: &mut Executor, stmt: Statement) -> Result<CommandResult> {
    let (db_name, if_not_exists) = match stmt {
        Statement::CreateDatabase { db_name, if_not_exists, .. } => {
            (db_name.to_string(), if_not_exists)
        }
        _ => unreachable!(),
    };

    match exec.engine.create_database(&db_name, &exec.config.current_user) {
        Ok(()) => Ok(CommandResult::Created("CREATE DATABASE".to_string())),
        Err(e) if if_not_exists => Ok(CommandResult::Created("CREATE DATABASE".to_string())),
        Err(e) => Err(e),
    }
}

pub fn execute_drop_database(exec: &mut Executor, names: &[ast::ObjectName], if_exists: bool) -> Result<CommandResult> {
    for name in names {
        let db_name = name.to_string();
        match exec.engine.drop_database(&db_name) {
            Ok(()) => {}
            Err(e) if if_exists => {}
            Err(e) => return Err(e),
        }
    }
    Ok(CommandResult::Dropped("DROP DATABASE".to_string()))
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Split "schema.table" or just "table" using session search path.
pub fn split_schema_table(full_name: &str, exec: &Executor) -> (String, String) {
    // Strip quotes
    let stripped = full_name.trim_matches('"');
    if let Some((s, t)) = stripped.split_once('.') {
        (s.trim_matches('"').to_lowercase(), t.trim_matches('"').to_lowercase())
    } else {
        let schema = exec.config.search_path.first().cloned().unwrap_or_else(|| "public".to_string());
        (schema.to_lowercase(), stripped.to_lowercase())
    }
}

fn split_schema_table_direct(full_name: &str, search_path: &[&str]) -> (String, String) {
    let stripped = full_name.trim_matches('"');
    if let Some((s, t)) = stripped.split_once('.') {
        (s.trim_matches('"').to_lowercase(), t.trim_matches('"').to_lowercase())
    } else {
        let schema = search_path.first().map(|s| s.to_string()).unwrap_or_else(|| "public".to_string());
        (schema, stripped.to_lowercase())
    }
}

/// Map a sqlparser DataType to a PostgreSQL OID.
pub fn data_type_to_oid(dt: &DataType) -> crate::types::Oid {
    match dt {
        DataType::Boolean => oid::BOOL,
        DataType::TinyInt(_) | DataType::SmallInt(_) | DataType::Int2(_) => oid::INT2,
        DataType::MediumInt(_) | DataType::Int(_) | DataType::Integer(_) | DataType::Int4(_) => oid::INT4,
        DataType::BigInt(_) | DataType::Int8(_) => oid::INT8,
        DataType::Float(_) | DataType::Float4 | DataType::Real => oid::FLOAT4,
        DataType::Float8 | DataType::Double | DataType::DoublePrecision => oid::FLOAT8,
        DataType::Decimal(info) | DataType::Numeric(info) | DataType::Dec(info) => oid::NUMERIC,
        DataType::Varchar(_) | DataType::CharacterVarying(_) => oid::VARCHAR,
        DataType::Char(_) | DataType::Character(_) => oid::BPCHAR,
        DataType::Text | DataType::String(_) => oid::TEXT,
        DataType::Bytea | DataType::Blob(_) | DataType::Bytes(_) => oid::BYTEA,
        DataType::Date => oid::DATE,
        DataType::Time(_, _) => oid::TIME,
        DataType::Timestamp(_, _) => oid::TIMESTAMPTZ,
        DataType::Interval => oid::INTERVAL,
        DataType::Uuid => oid::UUID,
        DataType::JSON => oid::JSON,
        DataType::JSONB => oid::JSONB,
        DataType::Array(inner) => {
            match inner {
                ast::ArrayElemTypeDef::AngleBracket(t) |
                ast::ArrayElemTypeDef::SquareBracket(t, _) |
                ast::ArrayElemTypeDef::Parenthesis(t) => crate::types::array_oid_for(data_type_to_oid(t)),
                ast::ArrayElemTypeDef::None => oid::ANYARRAY,
            }
        }
        DataType::Custom(name, _) => {
            // Look up by name
            crate::types::oid_for_type_name(&name.to_string().to_lowercase()).unwrap_or(oid::TEXT)
        }
        _ => oid::TEXT,
    }
}

/// Get type modifier (e.g. varchar length, numeric precision).
fn data_type_modifier(dt: &DataType) -> i32 {
    match dt {
        DataType::Varchar(Some(n)) | DataType::CharacterVarying(Some(n)) => {
            match n {
                ast::CharacterLength::IntegerLength { length, .. } => *length as i32 + 4,
                _ => -1,
            }
        }
        DataType::Char(Some(n)) | DataType::Character(Some(n)) => {
            match n {
                ast::CharacterLength::IntegerLength { length, .. } => *length as i32 + 4,
                _ => -1,
            }
        }
        DataType::Decimal(ast::ExactNumberInfo::Precision(p)) => *p as i32,
        DataType::Decimal(ast::ExactNumberInfo::PrecisionAndScale(p, s)) => {
            ((*p as i32) << 16) | (*s as i32)
        }
        _ => -1,
    }
}

fn referential_action_to_fk(action: &Option<ReferentialAction>) -> FkAction {
    match action {
        None => FkAction::NoAction,
        Some(ReferentialAction::Cascade) => FkAction::Cascade,
        Some(ReferentialAction::SetNull) => FkAction::SetNull,
        Some(ReferentialAction::SetDefault) => FkAction::SetDefault,
        Some(ReferentialAction::Restrict) => FkAction::Restrict,
        Some(ReferentialAction::NoAction) => FkAction::NoAction,
    }
}

fn value_to_i64(expr: &ast::Expr) -> Option<i64> {
    match expr {
        ast::Expr::Value(ast::Value::Number(n, _)) => n.parse().ok(),
        ast::Expr::UnaryOp { op: ast::UnaryOperator::Minus, expr } => {
            value_to_i64(expr).map(|v| -v)
        }
        _ => None,
    }
}
