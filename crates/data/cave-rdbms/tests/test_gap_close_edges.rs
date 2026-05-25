// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Edge / failure / boundary coverage for cave-rdbms — types, schema, transaction,
//! catalog, optimizer, error responses, lexer & parser.
//!
//! Strict no-stub mandate: every test asserts a real behavior, not just "doesn't panic".

use cave_rdbms::engine::Engine;
use cave_rdbms::protocol::error::ErrorResponse;
use cave_rdbms::sql::ast::*;
use cave_rdbms::sql::lexer::{Lexer, Token};
use cave_rdbms::sql::optimizer::Optimizer;
use cave_rdbms::sql::parser::Parser;
use cave_rdbms::storage::catalog::SystemCatalog;
use cave_rdbms::storage::schema::{ColumnDef, Database, Schema, Table};
use cave_rdbms::storage::transaction::{Transaction, TransactionState};
use cave_rdbms::types::{SqlType, SqlValue, oid};
use std::cmp::Ordering;

// ---------------------------------------------------------------------------
// SqlType OID round-trip + coverage of every variant
// ---------------------------------------------------------------------------

#[test]
fn sqltype_oid_roundtrip_every_variant() {
    let all = [
        SqlType::Int4,
        SqlType::Int8,
        SqlType::Numeric,
        SqlType::Text,
        SqlType::Varchar,
        SqlType::Bool,
        SqlType::Date,
        SqlType::Timestamp,
        SqlType::Null,
    ];
    for ty in &all {
        let oid = ty.oid();
        let back = SqlType::from_oid(oid).expect("known oid must map back");
        assert_eq!(&back, ty, "oid roundtrip failed for {:?}", ty);
    }
}

#[test]
fn sqltype_from_unknown_oid_is_none() {
    assert!(SqlType::from_oid(999_999).is_none());
    assert!(SqlType::from_oid(42).is_none());
}

#[test]
fn sqltype_names_are_pg_compatible() {
    assert_eq!(SqlType::Int4.name(), "int4");
    assert_eq!(SqlType::Int8.name(), "int8");
    assert_eq!(SqlType::Varchar.name(), "character varying");
    assert_eq!(SqlType::Bool.name(), "boolean");
    assert_eq!(SqlType::Timestamp.name(), "timestamp without time zone");
    assert_eq!(SqlType::Null.name(), "void");
}

#[test]
fn sqltype_oid_constants_match_postgres() {
    assert_eq!(oid::INT4, 23);
    assert_eq!(oid::INT8, 20);
    assert_eq!(oid::TEXT, 25);
    assert_eq!(oid::BOOL, 16);
    assert_eq!(oid::NUMERIC, 1700);
    assert_eq!(oid::VARCHAR, 1043);
    assert_eq!(oid::DATE, 1082);
    assert_eq!(oid::TIMESTAMP, 1114);
}

// ---------------------------------------------------------------------------
// SqlValue accessor / coercion edges
// ---------------------------------------------------------------------------

#[test]
fn sqlvalue_accessors_return_none_for_wrong_type() {
    assert!(SqlValue::Int4(1).as_str().is_none());
    assert!(SqlValue::Text("x".into()).as_i32().is_none());
    assert!(SqlValue::Bool(true).as_f64().is_none());
    assert!(SqlValue::Null.as_bool().is_none());
}

#[test]
fn sqlvalue_as_i64_widens_int4() {
    assert_eq!(SqlValue::Int4(42).as_i64(), Some(42i64));
    assert_eq!(SqlValue::Int4(i32::MIN).as_i64(), Some(i32::MIN as i64));
    assert_eq!(SqlValue::Int8(1_000_000_000_000).as_i64(), Some(1_000_000_000_000));
}

#[test]
fn sqlvalue_as_f64_widens_integers() {
    assert_eq!(SqlValue::Int4(7).as_f64(), Some(7.0));
    assert_eq!(SqlValue::Int8(7).as_f64(), Some(7.0));
    assert_eq!(SqlValue::Numeric(2.5).as_f64(), Some(2.5));
    assert!(SqlValue::Bool(false).as_f64().is_none());
}

#[test]
fn coerce_int8_to_int4_out_of_range_errors() {
    let big = SqlValue::Int8(i64::MAX);
    let res = big.coerce_to(&SqlType::Int4);
    assert!(res.is_err(), "i64::MAX must fail to coerce into i32");
    let err = res.unwrap_err();
    assert!(err.contains("out of"), "error must mention range: {}", err);
}

#[test]
fn coerce_int8_to_int4_in_range_ok() {
    let v = SqlValue::Int8(123);
    assert_eq!(v.coerce_to(&SqlType::Int4).unwrap(), SqlValue::Int4(123));
}

#[test]
fn coerce_int8_min_max_boundary() {
    assert_eq!(
        SqlValue::Int8(i32::MIN as i64).coerce_to(&SqlType::Int4).unwrap(),
        SqlValue::Int4(i32::MIN)
    );
    assert_eq!(
        SqlValue::Int8(i32::MAX as i64).coerce_to(&SqlType::Int4).unwrap(),
        SqlValue::Int4(i32::MAX)
    );
    assert!(SqlValue::Int8(i32::MAX as i64 + 1).coerce_to(&SqlType::Int4).is_err());
    assert!(SqlValue::Int8(i32::MIN as i64 - 1).coerce_to(&SqlType::Int4).is_err());
}

#[test]
fn coerce_text_to_int_parses_and_rejects() {
    assert_eq!(
        SqlValue::Text("42".into()).coerce_to(&SqlType::Int4).unwrap(),
        SqlValue::Int4(42)
    );
    let err = SqlValue::Text("abc".into()).coerce_to(&SqlType::Int4).unwrap_err();
    assert!(err.contains("cannot cast"));
}

#[test]
fn coerce_text_to_bool_all_truthy_falsy_forms() {
    for truthy in &["t", "T", "true", "TRUE", "y", "YES", "1"] {
        let v = SqlValue::Text((*truthy).into());
        assert_eq!(v.coerce_to(&SqlType::Bool).unwrap(), SqlValue::Bool(true), "{}", truthy);
    }
    for falsy in &["f", "F", "false", "FALSE", "n", "NO", "0"] {
        let v = SqlValue::Text((*falsy).into());
        assert_eq!(v.coerce_to(&SqlType::Bool).unwrap(), SqlValue::Bool(false), "{}", falsy);
    }
    assert!(SqlValue::Text("maybe".into()).coerce_to(&SqlType::Bool).is_err());
}

#[test]
fn coerce_null_to_any_type_stays_null() {
    for ty in &[SqlType::Int4, SqlType::Text, SqlType::Bool, SqlType::Numeric] {
        assert_eq!(SqlValue::Null.coerce_to(ty).unwrap(), SqlValue::Null);
    }
}

#[test]
fn coerce_identity_returns_clone() {
    assert_eq!(
        SqlValue::Text("hello".into()).coerce_to(&SqlType::Text).unwrap(),
        SqlValue::Text("hello".into())
    );
    assert_eq!(SqlValue::Bool(true).coerce_to(&SqlType::Bool).unwrap(), SqlValue::Bool(true));
}

#[test]
fn coerce_incompatible_pair_errors() {
    // Bool → Numeric is not in the coercion table.
    assert!(SqlValue::Bool(true).coerce_to(&SqlType::Numeric).is_err());
    // Date → Int4
    assert!(SqlValue::Date("2026-01-01".into()).coerce_to(&SqlType::Int4).is_err());
}

#[test]
fn coerce_bool_to_int_uses_one_zero() {
    assert_eq!(SqlValue::Bool(true).coerce_to(&SqlType::Int4).unwrap(), SqlValue::Int4(1));
    assert_eq!(SqlValue::Bool(false).coerce_to(&SqlType::Int4).unwrap(), SqlValue::Int4(0));
    assert_eq!(SqlValue::Bool(true).coerce_to(&SqlType::Int8).unwrap(), SqlValue::Int8(1));
    assert_eq!(SqlValue::Bool(true).coerce_to(&SqlType::Text).unwrap(), SqlValue::Text("true".into()));
}

#[test]
fn sqlvalue_to_string_special_forms() {
    assert_eq!(SqlValue::Null.to_string(), "NULL");
    assert_eq!(SqlValue::Bool(true).to_string(), "true");
    assert_eq!(SqlValue::Bool(false).to_string(), "false");
    // Numeric with no fractional part renders as "N.0"
    assert_eq!(SqlValue::Numeric(3.0).to_string(), "3.0");
    // Numeric with fractional part renders directly
    assert!(SqlValue::Numeric(3.5).to_string().starts_with("3.5"));
}

#[test]
fn sqlvalue_to_json_handles_non_finite_as_null() {
    use serde_json::Value as J;
    assert_eq!(SqlValue::Numeric(f64::NAN).to_json(), J::Null);
    assert_eq!(SqlValue::Numeric(f64::INFINITY).to_json(), J::Null);
    assert_eq!(SqlValue::Null.to_json(), J::Null);
    assert_eq!(SqlValue::Bool(true).to_json(), J::Bool(true));
}

// ---------------------------------------------------------------------------
// SqlValue comparison (Null ordering, type-mismatch, equality)
// ---------------------------------------------------------------------------

#[test]
fn compare_null_orders_below_every_value() {
    assert_eq!(SqlValue::Null.compare(&SqlValue::Null), Some(Ordering::Equal));
    assert_eq!(
        SqlValue::Null.compare(&SqlValue::Int4(0)),
        Some(Ordering::Less)
    );
    assert_eq!(
        SqlValue::Int4(0).compare(&SqlValue::Null),
        Some(Ordering::Greater)
    );
}

#[test]
fn compare_same_type_orders_naturally() {
    assert_eq!(
        SqlValue::Int4(1).compare(&SqlValue::Int4(2)),
        Some(Ordering::Less)
    );
    assert_eq!(
        SqlValue::Text("apple".into()).compare(&SqlValue::Text("banana".into())),
        Some(Ordering::Less)
    );
    assert_eq!(
        SqlValue::Bool(false).compare(&SqlValue::Bool(true)),
        Some(Ordering::Less)
    );
}

#[test]
fn compare_cross_type_int4_int8_returns_none() {
    // The current implementation rejects cross-type comparison.
    assert!(SqlValue::Int4(1).compare(&SqlValue::Int8(2)).is_none());
    assert!(SqlValue::Int4(1).compare(&SqlValue::Text("x".into())).is_none());
}

#[test]
fn compare_numeric_nan_is_unordered() {
    assert!(SqlValue::Numeric(f64::NAN).compare(&SqlValue::Numeric(1.0)).is_none());
}

#[test]
fn compare_date_and_timestamp_lexicographic() {
    assert_eq!(
        SqlValue::Date("2026-01-01".into()).compare(&SqlValue::Date("2026-01-02".into())),
        Some(Ordering::Less)
    );
}

// ---------------------------------------------------------------------------
// Schema / Table / Database
// ---------------------------------------------------------------------------

#[test]
fn database_has_three_default_schemas() {
    let db = Database::new("mydb");
    assert!(db.schemas.contains_key("public"));
    assert!(db.schemas.contains_key("pg_catalog"));
    assert!(db.schemas.contains_key("information_schema"));
    assert_eq!(db.schemas.len(), 3);
}

#[test]
fn schema_starts_empty() {
    let s = Schema::new("custom");
    assert!(s.tables.is_empty());
    assert_eq!(s.name, "custom");
}

#[test]
fn table_column_index_resolves_or_none() {
    let cols = vec![
        ColumnDef { name: "id".into(), type_name: "int".into(), not_null: true, primary_key: true },
        ColumnDef { name: "name".into(), type_name: "text".into(), not_null: false, primary_key: false },
    ];
    let t = Table::new("things", cols);
    assert_eq!(t.column_index("id"), Some(0));
    assert_eq!(t.column_index("name"), Some(1));
    assert!(t.column_index("missing").is_none());
}

#[test]
fn table_row_count_reflects_inserted_rows() {
    let t_empty = Table::new("e", vec![]);
    assert_eq!(t_empty.row_count(), 0);

    let mut t = Table::new("t", vec![ColumnDef {
        name: "v".into(), type_name: "int".into(), not_null: false, primary_key: false,
    }]);
    t.rows.push(vec![SqlValue::Int4(1)]);
    t.rows.push(vec![SqlValue::Int4(2)]);
    assert_eq!(t.row_count(), 2);
}

// ---------------------------------------------------------------------------
// Transaction state machine
// ---------------------------------------------------------------------------

#[test]
fn transaction_default_equals_new() {
    let a = Transaction::new();
    let b = Transaction::default();
    assert_eq!(a.state, b.state);
    assert_eq!(a.savepoints.len(), b.savepoints.len());
}

#[test]
fn transaction_rollback_clears_all_savepoints() {
    let mut tx = Transaction::new();
    tx.begin();
    tx.create_savepoint("a");
    tx.create_savepoint("b");
    tx.create_savepoint("c");
    assert_eq!(tx.savepoints.len(), 3);
    tx.rollback();
    assert!(tx.savepoints.is_empty());
    assert_eq!(tx.state, TransactionState::Idle);
}

#[test]
fn transaction_commit_clears_savepoints() {
    let mut tx = Transaction::new();
    tx.begin();
    tx.create_savepoint("only");
    tx.commit();
    assert!(tx.savepoints.is_empty());
    assert_eq!(tx.state, TransactionState::Idle);
}

#[test]
fn rollback_to_unknown_savepoint_returns_false() {
    let mut tx = Transaction::new();
    tx.begin();
    tx.create_savepoint("sp1");
    assert!(!tx.rollback_to_savepoint("nonexistent"));
    // Existing savepoints untouched
    assert_eq!(tx.savepoints, vec!["sp1".to_string()]);
}

#[test]
fn rollback_to_savepoint_truncates_after_it() {
    let mut tx = Transaction::new();
    tx.begin();
    for name in &["a", "b", "c", "d"] {
        tx.create_savepoint(name);
    }
    assert!(tx.rollback_to_savepoint("b"));
    // Implementation keeps savepoints up-to-and-including "b".
    assert_eq!(tx.savepoints, vec!["a".to_string(), "b".to_string()]);
}

// ---------------------------------------------------------------------------
// System catalog
// ---------------------------------------------------------------------------

#[test]
fn catalog_empty_db_still_has_three_schemas_but_no_tables() {
    let db = Database::new("empty");
    assert!(SystemCatalog::pg_tables(&db).is_empty());
    assert!(SystemCatalog::information_schema_tables(&db).is_empty());
    assert!(SystemCatalog::information_schema_columns(&db).is_empty());
}

#[test]
fn catalog_columns_uses_one_based_ordinal_position() {
    let mut db = Database::new("d");
    let cols = vec![
        ColumnDef { name: "a".into(), type_name: "int".into(), not_null: true, primary_key: false },
        ColumnDef { name: "b".into(), type_name: "text".into(), not_null: false, primary_key: false },
    ];
    db.schemas
        .get_mut("public")
        .unwrap()
        .tables
        .insert("t".into(), Table::new("t", cols));
    let rows = SystemCatalog::information_schema_columns(&db);
    assert_eq!(rows.len(), 2);
    // ordinal_position is column[4]
    let ordinals: Vec<&str> = rows.iter().map(|r| r[4].as_str()).collect();
    assert!(ordinals.contains(&"1"));
    assert!(ordinals.contains(&"2"));
}

#[test]
fn catalog_columns_is_nullable_yes_no() {
    let mut db = Database::new("d");
    let cols = vec![
        ColumnDef { name: "id".into(), type_name: "int".into(), not_null: true, primary_key: true },
        ColumnDef { name: "opt".into(), type_name: "text".into(), not_null: false, primary_key: false },
    ];
    db.schemas
        .get_mut("public")
        .unwrap()
        .tables
        .insert("t".into(), Table::new("t", cols));
    let rows = SystemCatalog::information_schema_columns(&db);
    let id_row = rows.iter().find(|r| r[3] == "id").expect("id col");
    let opt_row = rows.iter().find(|r| r[3] == "opt").expect("opt col");
    assert_eq!(id_row[6], "NO");
    assert_eq!(opt_row[6], "YES");
}

#[test]
fn catalog_pg_tables_includes_user_owner_postgres() {
    let mut db = Database::new("d");
    db.schemas
        .get_mut("public")
        .unwrap()
        .tables
        .insert("x".into(), Table::new("x", vec![]));
    let rows = SystemCatalog::pg_tables(&db);
    assert!(!rows.is_empty());
    assert_eq!(rows[0][2], "postgres");
}

// ---------------------------------------------------------------------------
// Optimizer constant folding edges
// ---------------------------------------------------------------------------

#[test]
fn optimizer_does_not_fold_division_by_zero() {
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Literal::Integer(10))),
        op: BinaryOp::Div,
        right: Box::new(Expr::Literal(Literal::Integer(0))),
    };
    let folded = Optimizer::fold_constants(&expr);
    // Should NOT collapse to a literal — leaves the BinaryOp intact so executor reports div_by_zero.
    assert!(matches!(folded, Expr::BinaryOp { .. }));
}

#[test]
fn optimizer_folds_float_division() {
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Literal::Float(10.0))),
        op: BinaryOp::Div,
        right: Box::new(Expr::Literal(Literal::Float(4.0))),
    };
    let folded = Optimizer::fold_constants(&expr);
    if let Expr::Literal(Literal::Float(v)) = folded {
        assert!((v - 2.5).abs() < 1e-9);
    } else {
        panic!("expected float literal, got {:?}", folded);
    }
}

#[test]
fn optimizer_folds_unary_not_boolean() {
    let expr = Expr::UnaryOp {
        op: UnaryOp::Not,
        operand: Box::new(Expr::Literal(Literal::Boolean(true))),
    };
    let folded = Optimizer::fold_constants(&expr);
    assert!(matches!(folded, Expr::Literal(Literal::Boolean(false))));
}

#[test]
fn optimizer_keeps_unfoldable_unary_minus_string() {
    let expr = Expr::UnaryOp {
        op: UnaryOp::Minus,
        operand: Box::new(Expr::Literal(Literal::String("not-a-number".into()))),
    };
    let folded = Optimizer::fold_constants(&expr);
    // Cannot negate a string literal, so the UnaryOp must remain.
    assert!(matches!(folded, Expr::UnaryOp { .. }));
}

#[test]
fn optimizer_recursively_folds_nested_arithmetic() {
    // (2 + 3) * 4  →  20
    let inner = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Literal::Integer(2))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Literal::Integer(3))),
    };
    let outer = Expr::BinaryOp {
        left: Box::new(inner),
        op: BinaryOp::Mul,
        right: Box::new(Expr::Literal(Literal::Integer(4))),
    };
    let folded = Optimizer::fold_constants(&outer);
    assert!(matches!(folded, Expr::Literal(Literal::Integer(20))));
}

#[test]
fn optimizer_does_not_fold_mixed_int_float() {
    let expr = Expr::BinaryOp {
        left: Box::new(Expr::Literal(Literal::Integer(2))),
        op: BinaryOp::Add,
        right: Box::new(Expr::Literal(Literal::Float(3.0))),
    };
    let folded = Optimizer::fold_constants(&expr);
    // No mixed-type rule in eval_binop → stays as BinaryOp.
    assert!(matches!(folded, Expr::BinaryOp { .. }));
}

// ---------------------------------------------------------------------------
// ErrorResponse — SQLSTATE codes
// ---------------------------------------------------------------------------

#[test]
fn error_sqlstate_codes_match_postgres() {
    assert_eq!(ErrorResponse::syntax_error("x").code, "42601");
    assert_eq!(ErrorResponse::table_not_found("t").code, "42P01");
    assert_eq!(ErrorResponse::column_not_found("c").code, "42703");
    assert_eq!(ErrorResponse::duplicate_table("t").code, "42P07");
    assert_eq!(ErrorResponse::unique_violation().code, "23505");
    assert_eq!(ErrorResponse::not_null_violation("c").code, "23502");
    assert_eq!(ErrorResponse::div_by_zero().code, "22012");
    assert_eq!(ErrorResponse::connection_error("x").code, "08000");
    assert_eq!(ErrorResponse::failed_transaction().code, "25P02");
}

#[test]
fn error_to_backend_fields_includes_detail_iff_present() {
    let bare = ErrorResponse::new("XX000", "msg");
    let f = bare.to_backend_fields();
    assert!(!f.contains_key(&'D'));

    let with_d = bare.with_detail("more");
    let f2 = with_d.to_backend_fields();
    assert_eq!(f2.get(&'D'), Some(&"more".to_string()));
}

#[test]
fn error_message_interpolates_identifier() {
    let e = ErrorResponse::table_not_found("missing_tbl");
    assert!(e.message.contains("missing_tbl"));
    let e = ErrorResponse::column_not_found("missing_col");
    assert!(e.message.contains("missing_col"));
}

#[test]
fn error_severity_default_is_error() {
    assert_eq!(ErrorResponse::new("XX", "y").severity, "ERROR");
}

// ---------------------------------------------------------------------------
// Lexer edge cases
// ---------------------------------------------------------------------------

#[test]
fn lexer_handles_dash_dash_line_comments() {
    let mut lx = Lexer::new("SELECT -- comment\n 1");
    assert_eq!(lx.next_token(), Token::Select);
    assert_eq!(lx.next_token(), Token::Integer(1));
    assert_eq!(lx.next_token(), Token::Eof);
}

#[test]
fn lexer_handles_quoted_identifier_with_doubled_quote_escape() {
    let mut lx = Lexer::new("\"weird\"\"name\"");
    match lx.next_token() {
        Token::QuotedIdentifier(s) => assert_eq!(s, "weird\"name"),
        other => panic!("expected QuotedIdentifier, got {:?}", other),
    }
}

#[test]
fn lexer_handles_string_with_doubled_apostrophe_escape() {
    let mut lx = Lexer::new("'it''s fine'");
    match lx.next_token() {
        Token::String(s) => assert_eq!(s, "it's fine"),
        other => panic!("expected String, got {:?}", other),
    }
}

#[test]
fn lexer_handles_backslash_n_escape_in_string() {
    let mut lx = Lexer::new("'line1\\nline2'");
    match lx.next_token() {
        Token::String(s) => assert_eq!(s, "line1\nline2"),
        other => panic!("got {:?}", other),
    }
}

#[test]
fn lexer_distinguishes_lt_le_ne_gt_ge() {
    let mut lx = Lexer::new("< <= > >= <> !=");
    assert_eq!(lx.next_token(), Token::Less);
    assert_eq!(lx.next_token(), Token::LessEqual);
    assert_eq!(lx.next_token(), Token::Greater);
    assert_eq!(lx.next_token(), Token::GreaterEqual);
    assert_eq!(lx.next_token(), Token::NotEqual);
    assert_eq!(lx.next_token(), Token::NotEqual);
}

#[test]
fn lexer_handles_empty_input() {
    let mut lx = Lexer::new("");
    assert_eq!(lx.next_token(), Token::Eof);
}

#[test]
fn lexer_keyword_case_insensitive() {
    let mut lx_lower = Lexer::new("select");
    let mut lx_mixed = Lexer::new("SeLeCt");
    assert_eq!(lx_lower.next_token(), Token::Select);
    assert_eq!(lx_mixed.next_token(), Token::Select);
}

#[test]
fn lexer_tokenize_produces_trailing_eof() {
    let mut lx = Lexer::new("SELECT 1");
    let tokens = lx.tokenize();
    assert_eq!(tokens.last(), Some(&Token::Eof));
}

#[test]
fn lexer_float_vs_integer_dispatch() {
    let mut lx = Lexer::new("100 100.5");
    assert_eq!(lx.next_token(), Token::Integer(100));
    match lx.next_token() {
        Token::Float(f) => assert!((f - 100.5).abs() < 1e-9),
        other => panic!("expected Float, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Parser — transaction control
// ---------------------------------------------------------------------------

#[test]
fn parser_begin_commit_rollback_keywords() {
    for (sql, expected) in &[
        ("BEGIN", Statement::Begin),
        ("COMMIT", Statement::Commit),
        ("ROLLBACK", Statement::Rollback),
    ] {
        let ast = Parser::new(sql).parse().expect(sql);
        assert_eq!(&ast.statement, expected);
    }
}

#[test]
fn parser_savepoint_creates_named_statement() {
    let ast = Parser::new("SAVEPOINT sp1").parse().expect("savepoint");
    match ast.statement {
        Statement::Savepoint(name) => assert_eq!(name, "sp1"),
        other => panic!("expected Savepoint, got {:?}", other),
    }
}

#[test]
fn parser_rollback_to_savepoint_recognized() {
    let ast = Parser::new("ROLLBACK TO SAVEPOINT sp1").parse().expect("rollback to");
    match ast.statement {
        Statement::RollbackTo(name) => assert_eq!(name, "sp1"),
        other => panic!("expected RollbackTo, got {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Engine DDL recognizer
// ---------------------------------------------------------------------------

#[tokio::test]
async fn engine_execute_ddl_create_table_acks() {
    let engine = Engine::new();
    let out = engine.execute_ddl("CREATE TABLE foo(id int)").await.unwrap();
    assert_eq!(out, "TABLE CREATED");
}

#[tokio::test]
async fn engine_execute_ddl_drop_table_acks() {
    let engine = Engine::new();
    let out = engine.execute_ddl("DROP TABLE foo").await.unwrap();
    assert_eq!(out, "TABLE DROPPED");
}

#[tokio::test]
async fn engine_execute_ddl_unknown_returns_ok() {
    let engine = Engine::new();
    let out = engine.execute_ddl("VACUUM").await.unwrap();
    assert_eq!(out, "OK");
}

#[tokio::test]
async fn engine_default_users_table_has_two_seed_rows() {
    let engine = Engine::default();
    let db = engine.get_database().await;
    let users = db.schemas.get("public").unwrap().tables.get("users").unwrap();
    assert_eq!(users.row_count(), 2);
    assert_eq!(users.columns.len(), 3);
    assert_eq!(users.columns[0].name, "id");
    assert!(users.columns[0].primary_key);
}

#[tokio::test]
async fn engine_concurrent_reads_do_not_deadlock() {
    let engine = Engine::new();
    let a = engine.get_database();
    let b = engine.get_database();
    let c = engine.get_database();
    let (ra, rb, rc) = tokio::join!(a, b, c);
    assert_eq!(ra.name, "postgres");
    assert_eq!(rb.name, "postgres");
    assert_eq!(rc.name, "postgres");
}
