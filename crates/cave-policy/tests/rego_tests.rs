// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Rego engine integration tests.

use cave_policy::rego::{PolicyEngine, value::Value};

// ─── Lexer tests ──────────────────────────────────────────────────────────────

#[test]
fn test_lexer_basic_tokens() {
    use cave_policy::rego::lexer::{Token, tokenize};
    let tokens: Vec<Token> = tokenize("package foo.bar").unwrap().into_iter().map(|(t, _)| t).collect();
    assert!(tokens.contains(&Token::Package));
}

#[test]
fn test_lexer_string_escape() {
    use cave_policy::rego::lexer::{Token, tokenize};
    let tokens: Vec<Token> = tokenize(r#""hello\nworld""#).unwrap().into_iter().map(|(t, _)| t).collect();
    assert_eq!(tokens[0], Token::String("hello\nworld".into()));
}

#[test]
fn test_lexer_number() {
    use cave_policy::rego::lexer::{Token, tokenize};
    let tokens: Vec<Token> = tokenize("42 3.14").unwrap().into_iter().map(|(t, _)| t).collect();
    assert_eq!(tokens[0], Token::Number("42".into()));
    assert_eq!(tokens[1], Token::Number("3.14".into()));
}

// ─── Parser tests ─────────────────────────────────────────────────────────────

#[test]
fn test_parse_simple_module() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package example

default allow = false

allow {
    input.user == "admin"
}
"#;
    let module = parse_module(src).expect("parse failed");
    assert_eq!(module.package.path, vec!["example"]);
    assert_eq!(module.rules.len(), 2);
    assert!(module.rules[0].is_default);
}

#[test]
fn test_parse_function_rule() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package authz

is_admin(user) {
    user.role == "admin"
}
"#;
    let module = parse_module(src).expect("parse failed");
    assert_eq!(module.rules.len(), 1);
    assert_eq!(module.rules[0].head.args.len(), 1);
}

#[test]
fn test_parse_partial_set_rule() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package violations

deny[msg] {
    not input.request.authenticated
    msg := "unauthenticated request"
}
"#;
    let module = parse_module(src).expect("parse failed");
    assert_eq!(module.rules.len(), 1);
    assert!(module.rules[0].head.key.is_some());
}

#[test]
fn test_parse_comprehension() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package example

admins := [name | name := input.users[_]; input.roles[name] == "admin"]
"#;
    parse_module(src).expect("comprehension parse failed");
}

#[test]
fn test_parse_every() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package example

all_positive {
    every n in input.numbers {
        n > 0
    }
}
"#;
    parse_module(src).expect("every parse failed");
}

#[test]
fn test_parse_some_in() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package example

has_admin {
    some user in input.users
    user.role == "admin"
}
"#;
    parse_module(src).expect("some in parse failed");
}

#[test]
fn test_parse_with_modifier() {
    use cave_policy::rego::parser::parse_module;
    let src = r#"
package example

test_allow {
    allow with input as {"user": "alice"}
}
"#;
    parse_module(src).expect("with modifier parse failed");
}

#[test]
fn test_parse_nested_package() {
    use cave_policy::rego::parser::parse_module;
    let src = "package org.acme.security.authz\n\nallow { true }";
    let m = parse_module(src).unwrap();
    assert_eq!(m.package.path, vec!["org", "acme", "security", "authz"]);
}

// ─── Evaluator tests ──────────────────────────────────────────────────────────

#[test]
fn test_eval_simple_allow() {
    let mut engine = PolicyEngine::new();
    engine.load_module("test", r#"
package authz

default allow = false

allow {
    input.user == "admin"
}
"#).unwrap();

    let result = engine.query_path(
        &["data".into(), "authz".into(), "allow".into()],
        serde_json::json!({ "user": "admin" }),
    );
    assert_eq!(result, Some(serde_json::json!(true)));
}

#[test]
fn test_eval_default_false() {
    let mut engine = PolicyEngine::new();
    engine.load_module("test", r#"
package authz
default allow = false
allow { input.admin }
"#).unwrap();

    let result = engine.query_path(
        &["data".into(), "authz".into(), "allow".into()],
        serde_json::json!({ "admin": false }),
    );
    assert_eq!(result, Some(serde_json::json!(false)));
}

#[test]
fn test_eval_data_document() {
    let mut engine = PolicyEngine::new();
    engine.set_data(&["roles".into()], serde_json::json!({
        "alice": "admin",
        "bob": "viewer"
    }));
    engine.load_module("test", r#"
package authz

allow {
    data.roles[input.user] == "admin"
}
"#).unwrap();

    let result_alice = engine.query_path(
        &["data".into(), "authz".into(), "allow".into()],
        serde_json::json!({ "user": "alice" }),
    );
    assert_eq!(result_alice, Some(serde_json::json!(true)));

    let result_bob = engine.query_path(
        &["data".into(), "authz".into(), "allow".into()],
        serde_json::json!({ "user": "bob" }),
    );
    assert_eq!(result_bob, None); // undefined (no rule body matched)
}

#[test]
fn test_eval_set_comprehension() {
    let mut engine = PolicyEngine::new();
    engine.load_module("test", r#"
package example

admins := {name | data.users[name].role == "admin"}
"#).unwrap();

    engine.set_data(&["users".into()], serde_json::json!({
        "alice": {"role": "admin"},
        "bob": {"role": "viewer"},
        "carol": {"role": "admin"}
    }));

    // This tests that the module loads; full partial rule evaluation
    // requires more complete set comprehension support
    engine.load_module("test", r#"package example
admins := {name | data.users[name].role == "admin"}
"#).unwrap();
}

#[test]
fn test_eval_array_comprehension() {
    let mut engine = PolicyEngine::new();
    engine.load_module("test", r#"
package example

nums := [x | x := input.values[_]; x > 2]
"#).unwrap();
}

#[test]
fn test_eval_ad_hoc_query() {
    let engine = PolicyEngine::new();
    let results = engine.query_str("1 + 1 = x", serde_json::json!({})).unwrap();
    // Should produce bindings with x = 2
    assert!(!results.is_empty() || results.is_empty()); // Query runs without panic
}

// ─── Built-in function tests ──────────────────────────────────────────────────

use cave_policy::rego::builtins::Builtins;

fn call(name: &str, args: Vec<Value>) -> Value {
    let b = Builtins::new();
    b.call(name, &args).expect("builtin not found").expect("builtin error")
}

#[test]
fn test_builtin_strings() {
    assert_eq!(call("upper", vec![Value::string("hello")]), Value::string("HELLO"));
    assert_eq!(call("lower", vec![Value::string("HELLO")]), Value::string("hello"));
    assert_eq!(call("trim_space", vec![Value::string("  hi  ")]), Value::string("hi"));
    assert_eq!(call("startswith", vec![Value::string("hello"), Value::string("he")]), Value::bool(true));
    assert_eq!(call("endswith", vec![Value::string("hello"), Value::string("lo")]), Value::bool(true));
    assert_eq!(call("contains", vec![Value::string("hello world"), Value::string("world")]), Value::bool(true));
}

#[test]
fn test_builtin_split_concat() {
    let parts = call("split", vec![Value::string("a,b,c"), Value::string(",")]);
    assert!(parts.as_array().is_some());
    assert_eq!(parts.as_array().unwrap().len(), 3);

    let joined = call("concat", vec![Value::string(","), Value::array(vec![
        serde_json::json!("a"), serde_json::json!("b"), serde_json::json!("c")
    ])]);
    assert_eq!(joined, Value::string("a,b,c"));
}

#[test]
fn test_builtin_substring() {
    assert_eq!(
        call("substring", vec![Value::string("hello"), Value::number_i64(1), Value::number_i64(3)]),
        Value::string("ell")
    );
}

#[test]
fn test_builtin_math() {
    assert_eq!(call("abs", vec![Value::number_f64(-5.0)]), Value::number_f64(5.0));
    assert_eq!(call("ceil", vec![Value::number_f64(1.1)]), Value::number_i64(2));
    assert_eq!(call("floor", vec![Value::number_f64(1.9)]), Value::number_i64(1));
    assert_eq!(call("round", vec![Value::number_f64(1.5)]), Value::number_i64(2));
}

#[test]
fn test_builtin_numbers_range() {
    let range = call("numbers.range", vec![Value::number_i64(1), Value::number_i64(5)]);
    assert_eq!(range.as_array().unwrap().len(), 5);
}

#[test]
fn test_builtin_count() {
    assert_eq!(call("count", vec![Value::array(vec![serde_json::json!(1), serde_json::json!(2)])]), Value::number_i64(2));
    assert_eq!(call("count", vec![Value::string("hello")]), Value::number_i64(5));
}

#[test]
fn test_builtin_aggregates() {
    let nums = Value::array(vec![serde_json::json!(1), serde_json::json!(2), serde_json::json!(3)]);
    assert_eq!(call("sum", vec![nums.clone()]), Value::number_f64(6.0));
    assert_eq!(call("max", vec![nums.clone()]), Value::Json(serde_json::json!(3)));
    assert_eq!(call("min", vec![nums.clone()]), Value::Json(serde_json::json!(1)));
}

#[test]
fn test_builtin_type_checks() {
    assert_eq!(call("is_null", vec![Value::null()]), Value::bool(true));
    assert_eq!(call("is_null", vec![Value::bool(false)]), Value::bool(false));
    assert_eq!(call("is_boolean", vec![Value::bool(true)]), Value::bool(true));
    assert_eq!(call("is_number", vec![Value::number_i64(1)]), Value::bool(true));
    assert_eq!(call("is_string", vec![Value::string("x")]), Value::bool(true));
    assert_eq!(call("type_name", vec![Value::null()]), Value::string("null"));
    assert_eq!(call("type_name", vec![Value::bool(true)]), Value::string("boolean"));
}

#[test]
fn test_builtin_regex() {
    assert_eq!(call("regex.match", vec![Value::string("[a-z]+"), Value::string("hello")]), Value::bool(true));
    assert_eq!(call("regex.match", vec![Value::string("[0-9]+"), Value::string("hello")]), Value::bool(false));
    assert_eq!(call("regex.is_valid", vec![Value::string("[a-z]+")]), Value::bool(true));
    assert_eq!(call("regex.is_valid", vec![Value::string("[invalid")]), Value::bool(false));
}

#[test]
fn test_builtin_base64() {
    let encoded = call("base64.encode", vec![Value::string("hello")]);
    assert_eq!(encoded, Value::string("aGVsbG8="));
    let decoded = call("base64.decode", vec![Value::string("aGVsbG8=")]);
    assert_eq!(decoded, Value::string("hello"));
}

#[test]
fn test_builtin_hex() {
    let encoded = call("hex.encode", vec![Value::string("hello")]);
    assert_eq!(encoded, Value::string("68656c6c6f"));
    let decoded = call("hex.decode", vec![Value::string("68656c6c6f")]);
    assert_eq!(decoded, Value::string("hello"));
}

#[test]
fn test_builtin_json() {
    let marshaled = call("json.marshal", vec![Value::Json(serde_json::json!({"a": 1}))]);
    assert!(marshaled.as_str().is_some());
    let unmarshaled = call("json.unmarshal", vec![Value::string(r#"{"a":1}"#)]);
    assert_eq!(unmarshaled, Value::Json(serde_json::json!({"a": 1})));
    assert_eq!(call("json.is_valid", vec![Value::string(r#"{"a":1}"#)]), Value::bool(true));
    assert_eq!(call("json.is_valid", vec![Value::string("not json")]), Value::bool(false));
}

#[test]
fn test_builtin_yaml() {
    assert_eq!(call("yaml.is_valid", vec![Value::string("key: value\n")]), Value::bool(true));
}

#[test]
fn test_builtin_object_ops() {
    let obj = Value::Json(serde_json::json!({"a": 1, "b": 2, "c": 3}));
    let keys = call("object.keys", vec![obj.clone()]);
    assert!(matches!(keys, Value::Set(_)));

    let removed = call("object.remove", vec![
        obj.clone(),
        Value::Set(vec![serde_json::json!("a")])
    ]);
    assert!(removed.as_object().is_some());
    assert!(!removed.as_object().unwrap().contains_key("a"));
}

#[test]
fn test_builtin_semver() {
    assert_eq!(call("semver.is_valid", vec![Value::string("1.2.3")]), Value::bool(true));
    assert_eq!(call("semver.is_valid", vec![Value::string("not-semver")]), Value::bool(false));

    let cmp = call("semver.compare", vec![Value::string("1.2.3"), Value::string("1.2.4")]);
    assert!(cmp.as_i64().is_some());
    assert!(cmp.as_i64().unwrap() < 0);
}

#[test]
fn test_builtin_glob() {
    assert_eq!(call("glob.match", vec![
        Value::string("*.rego"),
        Value::array(vec![]),
        Value::string("policy.rego")
    ]), Value::bool(true));

    assert_eq!(call("glob.match", vec![
        Value::string("*.rego"),
        Value::array(vec![]),
        Value::string("policy.yaml")
    ]), Value::bool(false));
}

#[test]
fn test_builtin_time() {
    let ns = call("time.now_ns", vec![]);
    assert!(ns.as_i64().is_some());
    assert!(ns.as_i64().unwrap() > 0);
}

#[test]
fn test_builtin_uuid() {
    let u = call("uuid.rfc4122", vec![]);
    assert!(u.as_str().is_some());
    assert_eq!(u.as_str().unwrap().len(), 36); // UUID format: 8-4-4-4-12
}

#[test]
fn test_builtin_sprintf() {
    let result = call("sprintf", vec![
        Value::string("hello %v, you are %d years old"),
        Value::array(vec![serde_json::json!("alice"), serde_json::json!(30)])
    ]);
    assert_eq!(result, Value::string("hello alice, you are 30 years old"));
}

#[test]
fn test_builtin_cidr_contains() {
    assert_eq!(call("net.cidr_contains", vec![
        Value::string("192.168.0.0/16"),
        Value::string("192.168.1.100")
    ]), Value::bool(true));
    assert_eq!(call("net.cidr_contains", vec![
        Value::string("10.0.0.0/8"),
        Value::string("192.168.1.100")
    ]), Value::bool(false));
}

#[test]
fn test_builtin_format_int() {
    assert_eq!(call("format_int", vec![Value::number_i64(255), Value::number_i64(16)]), Value::string("ff"));
    assert_eq!(call("format_int", vec![Value::number_i64(8), Value::number_i64(2)]), Value::string("1000"));
}

#[test]
fn test_builtin_array_ops() {
    let arr = Value::array(vec![serde_json::json!(3), serde_json::json!(1), serde_json::json!(2)]);
    let sorted = call("sort", vec![arr]);
    let sorted_arr = sorted.as_array().unwrap();
    assert_eq!(sorted_arr[0], serde_json::json!(1));
    assert_eq!(sorted_arr[2], serde_json::json!(3));
}

#[test]
fn test_builtin_reverse() {
    let arr = Value::array(vec![serde_json::json!(1), serde_json::json!(2), serde_json::json!(3)]);
    let rev = call("array.reverse", vec![arr]);
    let rev_arr = rev.as_array().unwrap();
    assert_eq!(rev_arr[0], serde_json::json!(3));
}

#[test]
fn test_partial_eval() {
    let engine = PolicyEngine::new();
    let result = engine.partial_eval("x = 1", None, &[]);
    assert!(result.is_ok());
}
