//! Policy engine — public API.

pub mod ast;
pub mod eval;
pub mod lexer;
pub mod parser;

use std::collections::HashMap;
use serde_json::Value;

pub use ast::Policy;
pub use eval::EvalContext;
pub use parser::parse_policy;

/// Compile a Rego source string into a `Policy`.
pub fn compile(src: &str) -> Result<Policy, String> {
    parse_policy(src)
}

/// Evaluate all rules of `policy` given JSON `input` and external `data`.
/// Returns a map of rule-name → decided value.
pub fn evaluate(
    policy: &Policy,
    input: &Value,
    data: &Value,
) -> HashMap<String, Value> {
    EvalContext::new(input, data).evaluate(policy)
}

/// Run policy tests (rules whose names start with `test_`).
/// Returns (passed, failed) counts and any failure messages.
pub fn run_tests(policy: &Policy) -> TestReport {
    let input = Value::Object(Default::default());
    let data = Value::Object(Default::default());
    let ctx = EvalContext::new(&input, &data);
    let results = ctx.evaluate(policy);

    let mut report = TestReport::default();
    for rule in &policy.rules {
        if !rule.name.starts_with("test_") { continue; }
        let passed = results
            .get(&rule.name)
            .map(|v| matches!(v, Value::Bool(true)))
            .unwrap_or(false);
        if passed {
            report.passed += 1;
        } else {
            report.failed += 1;
            report.failures.push(format!("FAIL: {}", rule.name));
        }
    }
    report
}

#[derive(Debug, Default)]
pub struct TestReport {
    pub passed: usize,
    pub failed: usize,
    pub failures: Vec<String>,
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn eval_src(src: &str, input: Value, data: Value) -> HashMap<String, Value> {
        let policy = compile(src).expect("compile failed");
        evaluate(&policy, &input, &data)
    }

    // 1. Simple allow rule fires when condition is met
    #[test]
    fn test_simple_allow_rule() {
        let src = r#"
package test
default allow = false
allow {
    input.user == "admin"
}
"#;
        let res = eval_src(src, json!({"user": "admin"}), json!({}));
        assert_eq!(res.get("allow"), Some(&json!(true)));
    }

    // 2. Default deny — allow stays false when condition not met
    #[test]
    fn test_default_deny() {
        let src = r#"
package test
default allow = false
allow {
    input.user == "admin"
}
"#;
        let res = eval_src(src, json!({"user": "guest"}), json!({}));
        assert_eq!(res.get("allow"), Some(&json!(false)));
    }

    // 3. Data lookup — policy can reference external data
    #[test]
    fn test_data_lookup() {
        let src = r#"
package test
default allow = false
allow {
    data.roles[input.user] == "admin"
}
"#;
        let data = json!({"roles": {"alice": "admin", "bob": "viewer"}});
        let res = eval_src(src, json!({"user": "alice"}), data);
        assert_eq!(res.get("allow"), Some(&json!(true)));
    }

    // 4. String built-ins — startswith / contains
    #[test]
    fn test_string_builtins() {
        let src = r#"
package test
result = startswith(input.path, "/api")
"#;
        let res = eval_src(src, json!({"path": "/api/users"}), json!({}));
        assert_eq!(res.get("result"), Some(&json!(true)));
    }

    // 5. Regex built-in
    #[test]
    fn test_regex_builtin() {
        let src = r#"
package test
valid_email {
    re_match(`^[^@]+@[^@]+\.[^@]+$`, input.email)
}
"#;
        let res = eval_src(src, json!({"email": "user@example.com"}), json!({}));
        assert_eq!(res.get("valid_email"), Some(&json!(true)));
    }

    // 6. count aggregate
    #[test]
    fn test_count_aggregate() {
        let src = r#"
package test
num_items = count(input.items)
"#;
        let res = eval_src(src, json!({"items": [1, 2, 3]}), json!({}));
        assert_eq!(res.get("num_items"), Some(&json!(3)));
    }

    // 7. sum aggregate
    #[test]
    fn test_sum_aggregate() {
        let src = r#"
package test
total = sum(input.values)
"#;
        let res = eval_src(src, json!({"values": [10, 20, 30]}), json!({}));
        assert_eq!(res.get("total"), Some(&json!(60.0)));
    }

    // 8. Array comprehension
    #[test]
    fn test_comprehension() {
        let src = r#"
package test
result = concat(", ", input.names)
"#;
        let res = eval_src(
            src,
            json!({"names": ["alice", "bob", "carol"]}),
            json!({}),
        );
        assert_eq!(res.get("result"), Some(&json!("alice, bob, carol")));
    }

    // 9. Package name is parsed correctly
    #[test]
    fn test_package_parse() {
        let policy = compile("package my.custom.pkg\ndefault allow = false").unwrap();
        assert_eq!(policy.package, vec!["my", "custom", "pkg"]);
    }

    // 10. JWT decode built-in
    #[test]
    fn test_jwt_decode() {
        // A well-known example HS256 token (header.payload.sig)
        let src = r#"
package test
claims = io.jwt.decode(input.token)[1]
"#;
        // Base64URL-encoded {"alg":"HS256"}.{"sub":"1234","name":"Alice"}.sig
        let header  = base64_url_encode(b"{\"alg\":\"HS256\",\"typ\":\"JWT\"}");
        let payload = base64_url_encode(b"{\"sub\":\"1234\",\"name\":\"Alice\"}");
        let token   = format!("{header}.{payload}.fakesig");
        let res = eval_src(src, json!({"token": token}), json!({}));
        let claims = res.get("claims").expect("claims should be set");
        assert_eq!(claims.get("sub").and_then(|v| v.as_str()), Some("1234"));
    }

    fn base64_url_encode(data: &[u8]) -> String {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(data)
    }

    // 11. multiple rules produce independent decisions
    #[test]
    fn test_multiple_rules() {
        let src = r#"
package test
default allow = false
default deny = true
allow {
    input.role == "admin"
}
deny {
    input.role == "banned"
}
"#;
        let res = eval_src(src, json!({"role": "admin"}), json!({}));
        assert_eq!(res.get("allow"), Some(&json!(true)));
        // deny did not fire (role != banned), so default applies
        assert_eq!(res.get("deny"), Some(&json!(true)));
    }

    // 12. arithmetic operations
    #[test]
    fn test_arithmetic() {
        let src = r#"
package test
result = (input.a + input.b) * 2
"#;
        let res = eval_src(src, json!({"a": 3, "b": 4}), json!({}));
        assert_eq!(res.get("result"), Some(&json!(14.0)));
    }

    // 13. nested object lookup
    #[test]
    fn test_nested_object_lookup() {
        let src = r#"
package test
role = data.users[input.user].role
"#;
        let data = json!({"users": {"alice": {"role": "admin"}, "bob": {"role": "viewer"}}});
        let res = eval_src(src, json!({"user": "alice"}), data);
        assert_eq!(res.get("role"), Some(&json!("admin")));
    }

    // 14. built-in test framework (test_ prefixed rules)
    #[test]
    fn test_policy_test_framework() {
        let src = r#"
package cave.test

test_numbers_equal {
    1 + 1 == 2
}

test_string_concat {
    concat("", ["hello", "world"]) == "helloworld"
}

test_default_false {
    not false
}
"#;
        // Rules discovered and all tests pass.
        let policy = compile(src).unwrap();
        let test_rules: Vec<&str> = policy.rules.iter()
            .filter(|r| r.name.starts_with("test_"))
            .map(|r| r.name.as_str())
            .collect();
        assert_eq!(test_rules.len(), 3);
        assert!(test_rules.contains(&"test_numbers_equal"));

        let report = run_tests(&policy);
        assert_eq!(report.failed, 0, "test failures: {:?}", report.failures);
        assert_eq!(report.passed, 3);
    }

    // 15. min/max aggregates
    #[test]
    fn test_min_max() {
        let src = r#"
package test
lowest  = min(input.scores)
highest = max(input.scores)
"#;
        let res = eval_src(src, json!({"scores": [5, 1, 9, 3, 7]}), json!({}));
        assert_eq!(res.get("lowest"),  Some(&json!(1.0)));
        assert_eq!(res.get("highest"), Some(&json!(9.0)));
    }
}
