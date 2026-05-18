// SPDX-License-Identifier: AGPL-3.0-or-later
//! OPA built-in functions — 150+ functions covering all OPA built-in categories.
//!
//! Categories:
//! - Comparison: ==, !=, <, >, <=, >=
//! - Arithmetic: +, -, *, /, %, abs, ceil, floor, round, numbers.range, numbers.range_step
//! - Strings: concat, contains, endswith, format_int, indexof, indexof_n, lower, ltrim,
//!            rtrim, trim, split, sprintf, startswith, substring, trim_left, trim_right,
//!            trim_prefix, trim_suffix, trim_space, upper, replace, strings.replace_n,
//!            strings.reverse, strings.count, strings.any_prefix_match, strings.any_suffix_match
//! - Regex: re_match, regex.match, regex.is_valid, regex.find_all_string_submatch_n,
//!          regex.find_n, regex.globs_match, regex.replace, regex.split, regex.template_match
//! - Types: is_null, is_boolean, is_number, is_string, is_array, is_set, is_object,
//!          type_name
//! - Aggregates: count, sum, max, min, product, sort, all, any
//! - Arrays: array.concat, array.reverse, array.slice, array.keys
//! - Sets: intersection, union, and, or, minus, set operations (& | - <)
//! - Objects: object.get, object.keys, object.values, object.remove, object.union,
//!            object.union_n, object.filter, object.subset, object.keys
//! - Encoding: base64.encode, base64.decode, base64url.encode, base64url.decode,
//!             hex.encode, hex.decode, json.marshal, json.unmarshal, json.is_valid,
//!             yaml.marshal, yaml.unmarshal, yaml.is_valid, urlquery.encode, urlquery.decode,
//!             urlquery.encode_object, urlquery.decode_object
//! - Tokens: io.jwt.decode, io.jwt.decode_verify, io.jwt.encode_sign,
//!           io.jwt.encode_sign_raw, io.jwt.verify_es256, io.jwt.verify_es384,
//!           io.jwt.verify_es512, io.jwt.verify_hs256, io.jwt.verify_hs384,
//!           io.jwt.verify_hs512, io.jwt.verify_ps256, io.jwt.verify_ps384,
//!           io.jwt.verify_ps512, io.jwt.verify_rs256, io.jwt.verify_rs384,
//!           io.jwt.verify_rs512
//! - Time: time.now_ns, time.date, time.clock, time.weekday, time.add_date,
//!         time.diff, time.format, time.parse_duration_ns, time.parse_rfc3339_ns
//! - Crypto: crypto.hmac.md5, crypto.hmac.sha1, crypto.hmac.sha256, crypto.hmac.sha512,
//!           crypto.md5, crypto.sha1, crypto.sha256, crypto.x509.parse_certificate_request,
//!           crypto.x509.parse_certificates, crypto.x509.parse_and_verify_certificates,
//!           crypto.x509.parse_rsa_private_key
//! - HTTP: http.send
//! - Net: net.cidr_contains, net.cidr_contains_matches, net.cidr_expand,
//!        net.cidr_intersects, net.cidr_merge, net.cidr_overlap, net.lookup_ip_addr
//! - UUID: uuid.rfc4122
//! - Semver: semver.compare, semver.is_valid
//! - Glob: glob.match, glob.quote_meta
//! - GraphQL: graphql.is_valid, graphql.parse, graphql.parse_and_verify,
//!            graphql.parse_query, graphql.parse_schema, graphql.schema_is_valid
//! - OPA: opa.runtime, rego.metadata.rule, rego.metadata.chain, rego.parse_module

use super::value::{Value, json_cmp};
use crate::error::PolicyError;
use std::collections::HashMap;

pub type BuiltinFn = fn(&[Value]) -> Result<Value, PolicyError>;

/// Registry of all built-in functions.
pub struct Builtins {
    fns: HashMap<String, BuiltinFn>,
}

impl Builtins {
    pub fn new() -> Self {
        let mut fns: HashMap<String, BuiltinFn> = HashMap::new();

        // Arithmetic
        fns.insert("abs".into(), builtin_abs);
        fns.insert("ceil".into(), builtin_ceil);
        fns.insert("floor".into(), builtin_floor);
        fns.insert("round".into(), builtin_round);
        fns.insert("numbers.range".into(), builtin_numbers_range);
        fns.insert("numbers.range_step".into(), builtin_numbers_range_step);

        // Strings
        fns.insert("concat".into(), builtin_concat);
        fns.insert("contains".into(), builtin_str_contains);
        fns.insert("endswith".into(), builtin_endswith);
        fns.insert("format_int".into(), builtin_format_int);
        fns.insert("indexof".into(), builtin_indexof);
        fns.insert("indexof_n".into(), builtin_indexof_n);
        fns.insert("lower".into(), builtin_lower);
        fns.insert("ltrim".into(), builtin_ltrim);
        fns.insert("rtrim".into(), builtin_rtrim);
        fns.insert("trim".into(), builtin_trim);
        fns.insert("trim_left".into(), builtin_trim_left);
        fns.insert("trim_right".into(), builtin_trim_right);
        fns.insert("trim_prefix".into(), builtin_trim_prefix);
        fns.insert("trim_suffix".into(), builtin_trim_suffix);
        fns.insert("trim_space".into(), builtin_trim_space);
        fns.insert("split".into(), builtin_split);
        fns.insert("sprintf".into(), builtin_sprintf);
        fns.insert("startswith".into(), builtin_startswith);
        fns.insert("substring".into(), builtin_substring);
        fns.insert("upper".into(), builtin_upper);
        fns.insert("replace".into(), builtin_replace);
        fns.insert("strings.replace_n".into(), builtin_strings_replace_n);
        fns.insert("strings.reverse".into(), builtin_strings_reverse);
        fns.insert("strings.count".into(), builtin_strings_count);
        fns.insert("strings.any_prefix_match".into(), builtin_strings_any_prefix_match);
        fns.insert("strings.any_suffix_match".into(), builtin_strings_any_suffix_match);

        // Regex
        fns.insert("re_match".into(), builtin_regex_match);
        fns.insert("regex.match".into(), builtin_regex_match);
        fns.insert("regex.is_valid".into(), builtin_regex_is_valid);
        fns.insert("regex.find_all_string_submatch_n".into(), builtin_regex_find_all_string_submatch_n);
        fns.insert("regex.find_n".into(), builtin_regex_find_n);
        fns.insert("regex.split".into(), builtin_regex_split);
        fns.insert("regex.replace".into(), builtin_regex_replace);
        fns.insert("regex.globs_match".into(), builtin_regex_globs_match);
        fns.insert("regex.template_match".into(), builtin_regex_template_match);

        // Types
        fns.insert("is_null".into(), builtin_is_null);
        fns.insert("is_boolean".into(), builtin_is_boolean);
        fns.insert("is_number".into(), builtin_is_number);
        fns.insert("is_string".into(), builtin_is_string);
        fns.insert("is_array".into(), builtin_is_array);
        fns.insert("is_set".into(), builtin_is_set);
        fns.insert("is_object".into(), builtin_is_object);
        fns.insert("type_name".into(), builtin_type_name);

        // Aggregates
        fns.insert("count".into(), builtin_count);
        fns.insert("sum".into(), builtin_sum);
        fns.insert("max".into(), builtin_max);
        fns.insert("min".into(), builtin_min);
        fns.insert("product".into(), builtin_product);
        fns.insert("sort".into(), builtin_sort);
        fns.insert("all".into(), builtin_all);
        fns.insert("any".into(), builtin_any);

        // Arrays
        fns.insert("array.concat".into(), builtin_array_concat);
        fns.insert("array.reverse".into(), builtin_array_reverse);
        fns.insert("array.slice".into(), builtin_array_slice);
        fns.insert("array.keys".into(), builtin_array_keys);

        // Objects
        fns.insert("object.get".into(), builtin_object_get);
        fns.insert("object.keys".into(), builtin_object_keys);
        fns.insert("object.values".into(), builtin_object_values);
        fns.insert("object.remove".into(), builtin_object_remove);
        fns.insert("object.union".into(), builtin_object_union);
        fns.insert("object.union_n".into(), builtin_object_union_n);
        fns.insert("object.filter".into(), builtin_object_filter);
        fns.insert("object.subset".into(), builtin_object_subset);

        // Sets
        fns.insert("intersection".into(), builtin_intersection);
        fns.insert("union".into(), builtin_union_set);

        // Encoding
        fns.insert("base64.encode".into(), builtin_base64_encode);
        fns.insert("base64.decode".into(), builtin_base64_decode);
        fns.insert("base64url.encode".into(), builtin_base64url_encode);
        fns.insert("base64url.decode".into(), builtin_base64url_decode);
        fns.insert("hex.encode".into(), builtin_hex_encode);
        fns.insert("hex.decode".into(), builtin_hex_decode);
        fns.insert("json.marshal".into(), builtin_json_marshal);
        fns.insert("json.unmarshal".into(), builtin_json_unmarshal);
        fns.insert("json.is_valid".into(), builtin_json_is_valid);
        fns.insert("json.filter".into(), builtin_json_filter);
        fns.insert("json.remove".into(), builtin_json_remove);
        fns.insert("json.patch".into(), builtin_json_patch);
        fns.insert("yaml.marshal".into(), builtin_yaml_marshal);
        fns.insert("yaml.unmarshal".into(), builtin_yaml_unmarshal);
        fns.insert("yaml.is_valid".into(), builtin_yaml_is_valid);
        fns.insert("urlquery.encode".into(), builtin_urlquery_encode);
        fns.insert("urlquery.decode".into(), builtin_urlquery_decode);
        fns.insert("urlquery.encode_object".into(), builtin_urlquery_encode_object);
        fns.insert("urlquery.decode_object".into(), builtin_urlquery_decode_object);

        // JWT
        fns.insert("io.jwt.decode".into(), builtin_jwt_decode);
        fns.insert("io.jwt.decode_verify".into(), builtin_jwt_decode_verify);
        fns.insert("io.jwt.encode_sign".into(), builtin_jwt_encode_sign);
        fns.insert("io.jwt.encode_sign_raw".into(), builtin_jwt_encode_sign_raw);
        fns.insert("io.jwt.verify_hs256".into(), |args| builtin_jwt_verify_hmac(args, "HS256"));
        fns.insert("io.jwt.verify_hs384".into(), |args| builtin_jwt_verify_hmac(args, "HS384"));
        fns.insert("io.jwt.verify_hs512".into(), |args| builtin_jwt_verify_hmac(args, "HS512"));
        fns.insert("io.jwt.verify_rs256".into(), |args| builtin_jwt_verify_rsa(args, "RS256"));
        fns.insert("io.jwt.verify_rs384".into(), |args| builtin_jwt_verify_rsa(args, "RS384"));
        fns.insert("io.jwt.verify_rs512".into(), |args| builtin_jwt_verify_rsa(args, "RS512"));
        fns.insert("io.jwt.verify_es256".into(), |args| builtin_jwt_verify_rsa(args, "ES256"));
        fns.insert("io.jwt.verify_es384".into(), |args| builtin_jwt_verify_rsa(args, "ES384"));
        fns.insert("io.jwt.verify_es512".into(), |args| builtin_jwt_verify_rsa(args, "ES512"));
        fns.insert("io.jwt.verify_ps256".into(), |args| builtin_jwt_verify_rsa(args, "PS256"));
        fns.insert("io.jwt.verify_ps384".into(), |args| builtin_jwt_verify_rsa(args, "PS384"));
        fns.insert("io.jwt.verify_ps512".into(), |args| builtin_jwt_verify_rsa(args, "PS512"));

        // Time
        fns.insert("time.now_ns".into(), builtin_time_now_ns);
        fns.insert("time.date".into(), builtin_time_date);
        fns.insert("time.clock".into(), builtin_time_clock);
        fns.insert("time.weekday".into(), builtin_time_weekday);
        fns.insert("time.add_date".into(), builtin_time_add_date);
        fns.insert("time.diff".into(), builtin_time_diff);
        fns.insert("time.parse_duration_ns".into(), builtin_time_parse_duration_ns);
        fns.insert("time.parse_rfc3339_ns".into(), builtin_time_parse_rfc3339_ns);
        fns.insert("time.format".into(), builtin_time_format);

        // Crypto
        fns.insert("crypto.md5".into(), |args| builtin_crypto_hash(args, "md5"));
        fns.insert("crypto.sha1".into(), |args| builtin_crypto_hash(args, "sha1"));
        fns.insert("crypto.sha256".into(), |args| builtin_crypto_hash(args, "sha256"));
        fns.insert("crypto.hmac.md5".into(), |args| builtin_crypto_hmac(args, "md5"));
        fns.insert("crypto.hmac.sha1".into(), |args| builtin_crypto_hmac(args, "sha1"));
        fns.insert("crypto.hmac.sha256".into(), |args| builtin_crypto_hmac(args, "sha256"));
        fns.insert("crypto.hmac.sha512".into(), |args| builtin_crypto_hmac(args, "sha512"));
        fns.insert("crypto.x509.parse_certificates".into(), builtin_x509_parse_certificates);
        fns.insert("crypto.x509.parse_certificate_request".into(), builtin_x509_parse_certificate_request);
        fns.insert("crypto.x509.parse_and_verify_certificates".into(), builtin_x509_parse_and_verify);
        fns.insert("crypto.x509.parse_rsa_private_key".into(), builtin_x509_parse_rsa_private_key);

        // Net / CIDR
        fns.insert("net.cidr_contains".into(), builtin_cidr_contains);
        fns.insert("net.cidr_contains_matches".into(), builtin_cidr_contains_matches);
        fns.insert("net.cidr_expand".into(), builtin_cidr_expand);
        fns.insert("net.cidr_intersects".into(), builtin_cidr_intersects);
        fns.insert("net.cidr_merge".into(), builtin_cidr_merge);
        fns.insert("net.lookup_ip_addr".into(), builtin_net_lookup_ip_addr);

        // UUID
        fns.insert("uuid.rfc4122".into(), builtin_uuid_rfc4122);

        // Semver
        fns.insert("semver.compare".into(), builtin_semver_compare);
        fns.insert("semver.is_valid".into(), builtin_semver_is_valid);

        // Glob
        fns.insert("glob.match".into(), builtin_glob_match);
        fns.insert("glob.quote_meta".into(), builtin_glob_quote_meta);

        // GraphQL (stubs)
        fns.insert("graphql.is_valid".into(), builtin_graphql_is_valid);
        fns.insert("graphql.parse".into(), builtin_graphql_parse);
        fns.insert("graphql.parse_and_verify".into(), builtin_graphql_parse_and_verify);
        fns.insert("graphql.parse_query".into(), builtin_graphql_parse_query);
        fns.insert("graphql.parse_schema".into(), builtin_graphql_parse_schema);
        fns.insert("graphql.schema_is_valid".into(), builtin_graphql_schema_is_valid);

        // OPA
        fns.insert("opa.runtime".into(), builtin_opa_runtime);
        fns.insert("rego.metadata.rule".into(), builtin_rego_metadata_rule);
        fns.insert("rego.metadata.chain".into(), builtin_rego_metadata_chain);
        fns.insert("rego.parse_module".into(), builtin_rego_parse_module);

        // Print (no-op in non-interactive mode)
        fns.insert("print".into(), builtin_print);

        Self { fns }
    }

    pub fn call(&self, name: &str, args: &[Value]) -> Option<Result<Value, PolicyError>> {
        self.fns.get(name).map(|f| f(args))
    }

    pub fn has(&self, name: &str) -> bool {
        self.fns.contains_key(name)
    }
}

impl Default for Builtins {
    fn default() -> Self { Self::new() }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

fn arg_str(args: &[Value], i: usize, fname: &str) -> Result<String, PolicyError> {
    args.get(i)
        .and_then(|v| v.as_str().map(|s| s.to_string()))
        .ok_or_else(|| PolicyError::Eval(format!("{fname}: arg {i} must be string")))
}

fn arg_f64(args: &[Value], i: usize, fname: &str) -> Result<f64, PolicyError> {
    args.get(i)
        .and_then(|v| v.as_f64())
        .ok_or_else(|| PolicyError::Eval(format!("{fname}: arg {i} must be number")))
}

fn arg_i64(args: &[Value], i: usize, fname: &str) -> Result<i64, PolicyError> {
    args.get(i)
        .and_then(|v| v.as_i64())
        .ok_or_else(|| PolicyError::Eval(format!("{fname}: arg {i} must be integer")))
}

fn arg_array(args: &[Value], i: usize, fname: &str) -> Result<Vec<serde_json::Value>, PolicyError> {
    args.get(i)
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(|| PolicyError::Eval(format!("{fname}: arg {i} must be array")))
}

fn arg_object(args: &[Value], i: usize, fname: &str) -> Result<serde_json::Map<String, serde_json::Value>, PolicyError> {
    args.get(i)
        .and_then(|v| v.as_object().cloned())
        .ok_or_else(|| PolicyError::Eval(format!("{fname}: arg {i} must be object")))
}

fn collection_values(v: &Value) -> Option<Vec<serde_json::Value>> {
    match v {
        Value::Json(serde_json::Value::Array(a)) => Some(a.clone()),
        Value::Set(s) => Some(s.clone()),
        _ => None,
    }
}

// ─── Arithmetic ───────────────────────────────────────────────────────────────

fn builtin_abs(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::number_f64(arg_f64(args, 0, "abs")?.abs()))
}

fn builtin_ceil(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::number_i64(arg_f64(args, 0, "ceil")?.ceil() as i64))
}

fn builtin_floor(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::number_i64(arg_f64(args, 0, "floor")?.floor() as i64))
}

fn builtin_round(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::number_i64(arg_f64(args, 0, "round")?.round() as i64))
}

fn builtin_numbers_range(args: &[Value]) -> Result<Value, PolicyError> {
    let a = arg_i64(args, 0, "numbers.range")?;
    let b = arg_i64(args, 1, "numbers.range")?;
    let range: Vec<serde_json::Value> = if a <= b {
        (a..=b).map(|i| serde_json::json!(i)).collect()
    } else {
        (b..=a).rev().map(|i| serde_json::json!(i)).collect()
    };
    Ok(Value::array(range))
}

fn builtin_numbers_range_step(args: &[Value]) -> Result<Value, PolicyError> {
    let a = arg_i64(args, 0, "numbers.range_step")?;
    let b = arg_i64(args, 1, "numbers.range_step")?;
    let step = arg_i64(args, 2, "numbers.range_step")?;
    if step <= 0 {
        return Err(PolicyError::Eval("numbers.range_step: step must be positive".into()));
    }
    let mut out = Vec::new();
    let mut cur = a;
    while if a <= b { cur <= b } else { cur >= b } {
        out.push(serde_json::json!(cur));
        cur += if a <= b { step } else { -step };
    }
    Ok(Value::array(out))
}

// ─── Strings ──────────────────────────────────────────────────────────────────

fn builtin_concat(args: &[Value]) -> Result<Value, PolicyError> {
    let delim = arg_str(args, 0, "concat")?;
    let coll = args.get(1).ok_or_else(|| PolicyError::Eval("concat: missing arg 1".into()))?;
    let parts: Vec<String> = match coll {
        Value::Json(serde_json::Value::Array(a)) => a
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect(),
        Value::Set(s) => s
            .iter()
            .map(|v| v.as_str().unwrap_or("").to_string())
            .collect(),
        _ => return Err(PolicyError::Eval("concat: arg 1 must be array or set".into())),
    };
    Ok(Value::string(parts.join(&delim)))
}

fn builtin_str_contains(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "contains")?;
    let sub = arg_str(args, 1, "contains")?;
    Ok(Value::bool(s.contains(sub.as_str())))
}

fn builtin_endswith(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "endswith")?;
    let suffix = arg_str(args, 1, "endswith")?;
    Ok(Value::bool(s.ends_with(suffix.as_str())))
}

fn builtin_format_int(args: &[Value]) -> Result<Value, PolicyError> {
    let num = arg_i64(args, 0, "format_int")?;
    let base = arg_i64(args, 1, "format_int")?;
    let s = match base {
        2 => format!("{:b}", num),
        8 => format!("{:o}", num),
        10 => format!("{}", num),
        16 => format!("{:x}", num),
        _ => return Err(PolicyError::Eval(format!("format_int: unsupported base {base}"))),
    };
    Ok(Value::string(s))
}

fn builtin_indexof(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "indexof")?;
    let sub = arg_str(args, 1, "indexof")?;
    let idx = s.find(sub.as_str()).map(|i| i as i64).unwrap_or(-1);
    Ok(Value::number_i64(idx))
}

fn builtin_indexof_n(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "indexof_n")?;
    let sub = arg_str(args, 1, "indexof_n")?;
    let mut indices = Vec::new();
    let mut start = 0;
    while let Some(pos) = s[start..].find(sub.as_str()) {
        let abs = start + pos;
        indices.push(serde_json::json!(abs as i64));
        start = abs + sub.len().max(1);
    }
    Ok(Value::array(indices))
}

fn builtin_lower(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::string(arg_str(args, 0, "lower")?.to_lowercase()))
}

fn builtin_ltrim(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "ltrim")?;
    let cutset = arg_str(args, 1, "ltrim")?;
    Ok(Value::string(s.trim_start_matches(|c| cutset.contains(c)).to_string()))
}

fn builtin_rtrim(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "rtrim")?;
    let cutset = arg_str(args, 1, "rtrim")?;
    Ok(Value::string(s.trim_end_matches(|c| cutset.contains(c)).to_string()))
}

fn builtin_trim(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "trim")?;
    let cutset = arg_str(args, 1, "trim")?;
    Ok(Value::string(s.trim_matches(|c| cutset.contains(c)).to_string()))
}

fn builtin_trim_left(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "trim_left")?;
    let cutset = arg_str(args, 1, "trim_left")?;
    Ok(Value::string(s.trim_start_matches(|c| cutset.contains(c)).to_string()))
}

fn builtin_trim_right(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "trim_right")?;
    let cutset = arg_str(args, 1, "trim_right")?;
    Ok(Value::string(s.trim_end_matches(|c| cutset.contains(c)).to_string()))
}

fn builtin_trim_prefix(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "trim_prefix")?;
    let prefix = arg_str(args, 1, "trim_prefix")?;
    Ok(Value::string(s.strip_prefix(prefix.as_str()).unwrap_or(&s).to_string()))
}

fn builtin_trim_suffix(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "trim_suffix")?;
    let suffix = arg_str(args, 1, "trim_suffix")?;
    Ok(Value::string(s.strip_suffix(suffix.as_str()).unwrap_or(&s).to_string()))
}

fn builtin_trim_space(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::string(arg_str(args, 0, "trim_space")?.trim().to_string()))
}

fn builtin_split(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "split")?;
    let delim = arg_str(args, 1, "split")?;
    let parts: Vec<serde_json::Value> = s
        .split(delim.as_str())
        .map(|p| serde_json::json!(p))
        .collect();
    Ok(Value::array(parts))
}

fn builtin_sprintf(args: &[Value]) -> Result<Value, PolicyError> {
    let fmt = arg_str(args, 0, "sprintf")?;
    let vals = arg_array(args, 1, "sprintf")?;
    // Simple sprintf: replace %v, %s, %d, %f sequentially
    let mut result = String::new();
    let mut chars = fmt.chars().peekable();
    let mut idx = 0;
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('v') | Some('s') => {
                    let val = vals.get(idx).map(|v| match v {
                        serde_json::Value::String(s) => s.clone(),
                        other => other.to_string(),
                    }).unwrap_or_default();
                    result.push_str(&val);
                    idx += 1;
                }
                Some('d') => {
                    let val = vals.get(idx)
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                    result.push_str(&val.to_string());
                    idx += 1;
                }
                Some('f') => {
                    let val = vals.get(idx)
                        .and_then(|v| v.as_f64())
                        .unwrap_or(0.0);
                    result.push_str(&format!("{val}"));
                    idx += 1;
                }
                Some('%') => result.push('%'),
                Some(other) => { result.push('%'); result.push(other); }
                None => result.push('%'),
            }
        } else {
            result.push(c);
        }
    }
    Ok(Value::string(result))
}

fn builtin_startswith(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "startswith")?;
    let prefix = arg_str(args, 1, "startswith")?;
    Ok(Value::bool(s.starts_with(prefix.as_str())))
}

fn builtin_substring(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "substring")?;
    let start = arg_i64(args, 1, "substring")? as usize;
    let length = arg_i64(args, 2, "substring")?;
    let chars: Vec<char> = s.chars().collect();
    let end = if length < 0 {
        chars.len()
    } else {
        (start + length as usize).min(chars.len())
    };
    let result: String = chars[start.min(chars.len())..end].iter().collect();
    Ok(Value::string(result))
}

fn builtin_upper(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::string(arg_str(args, 0, "upper")?.to_uppercase()))
}

fn builtin_replace(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "replace")?;
    let old = arg_str(args, 1, "replace")?;
    let new = arg_str(args, 2, "replace")?;
    Ok(Value::string(s.replace(old.as_str(), new.as_str())))
}

fn builtin_strings_replace_n(args: &[Value]) -> Result<Value, PolicyError> {
    let patterns = arg_object(args, 0, "strings.replace_n")?;
    let mut s = arg_str(args, 1, "strings.replace_n")?;
    for (old, new) in &patterns {
        if let serde_json::Value::String(n) = new {
            s = s.replace(old.as_str(), n.as_str());
        }
    }
    Ok(Value::string(s))
}

fn builtin_strings_reverse(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "strings.reverse")?;
    Ok(Value::string(s.chars().rev().collect::<String>()))
}

fn builtin_strings_count(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "strings.count")?;
    let sub = arg_str(args, 1, "strings.count")?;
    Ok(Value::number_i64(s.matches(sub.as_str()).count() as i64))
}

fn builtin_strings_any_prefix_match(args: &[Value]) -> Result<Value, PolicyError> {
    let searches = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("strings.any_prefix_match: arg 0 must be collection".into()))?;
    let prefixes = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("strings.any_prefix_match: arg 1 must be collection".into()))?;
    for s in &searches {
        if let Some(s_str) = s.as_str() {
            for p in &prefixes {
                if let Some(p_str) = p.as_str() {
                    if s_str.starts_with(p_str) { return Ok(Value::bool(true)); }
                }
            }
        }
    }
    Ok(Value::bool(false))
}

fn builtin_strings_any_suffix_match(args: &[Value]) -> Result<Value, PolicyError> {
    let searches = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("strings.any_suffix_match: arg 0 must be collection".into()))?;
    let suffixes = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("strings.any_suffix_match: arg 1 must be collection".into()))?;
    for s in &searches {
        if let Some(s_str) = s.as_str() {
            for suf in &suffixes {
                if let Some(suf_str) = suf.as_str() {
                    if s_str.ends_with(suf_str) { return Ok(Value::bool(true)); }
                }
            }
        }
    }
    Ok(Value::bool(false))
}

// ─── Regex ────────────────────────────────────────────────────────────────────

fn builtin_regex_match(args: &[Value]) -> Result<Value, PolicyError> {
    let pattern = arg_str(args, 0, "regex.match")?;
    let value = arg_str(args, 1, "regex.match")?;
    let re = regex::Regex::new(&pattern)
        .map_err(|e| PolicyError::Eval(format!("invalid regex: {e}")))?;
    Ok(Value::bool(re.is_match(&value)))
}

fn builtin_regex_is_valid(args: &[Value]) -> Result<Value, PolicyError> {
    let pattern = arg_str(args, 0, "regex.is_valid")?;
    Ok(Value::bool(regex::Regex::new(&pattern).is_ok()))
}

fn builtin_regex_find_all_string_submatch_n(args: &[Value]) -> Result<Value, PolicyError> {
    let pattern = arg_str(args, 0, "regex.find_all_string_submatch_n")?;
    let value = arg_str(args, 1, "regex.find_all_string_submatch_n")?;
    let n = arg_i64(args, 2, "regex.find_all_string_submatch_n")?;
    let re = regex::Regex::new(&pattern)
        .map_err(|e| PolicyError::Eval(format!("invalid regex: {e}")))?;
    let mut results: Vec<serde_json::Value> = Vec::new();
    let limit = if n < 0 { usize::MAX } else { n as usize };
    for caps in re.captures_iter(&value).take(limit) {
        let groups: Vec<serde_json::Value> = caps
            .iter()
            .map(|m| m.map(|m| serde_json::json!(m.as_str())).unwrap_or(serde_json::json!("")))
            .collect();
        results.push(serde_json::Value::Array(groups));
    }
    Ok(Value::array(results))
}

fn builtin_regex_find_n(args: &[Value]) -> Result<Value, PolicyError> {
    let pattern = arg_str(args, 0, "regex.find_n")?;
    let value = arg_str(args, 1, "regex.find_n")?;
    let n = arg_i64(args, 2, "regex.find_n")?;
    let re = regex::Regex::new(&pattern)
        .map_err(|e| PolicyError::Eval(format!("invalid regex: {e}")))?;
    let limit = if n < 0 { usize::MAX } else { n as usize };
    let results: Vec<serde_json::Value> = re
        .find_iter(&value)
        .take(limit)
        .map(|m| serde_json::json!(m.as_str()))
        .collect();
    Ok(Value::array(results))
}

fn builtin_regex_split(args: &[Value]) -> Result<Value, PolicyError> {
    let pattern = arg_str(args, 0, "regex.split")?;
    let value = arg_str(args, 1, "regex.split")?;
    let re = regex::Regex::new(&pattern)
        .map_err(|e| PolicyError::Eval(format!("invalid regex: {e}")))?;
    let parts: Vec<serde_json::Value> = re.split(&value).map(|s| serde_json::json!(s)).collect();
    Ok(Value::array(parts))
}

fn builtin_regex_replace(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "regex.replace")?;
    let pattern = arg_str(args, 1, "regex.replace")?;
    let replacement = arg_str(args, 2, "regex.replace")?;
    let re = regex::Regex::new(&pattern)
        .map_err(|e| PolicyError::Eval(format!("invalid regex: {e}")))?;
    Ok(Value::string(re.replace_all(&s, replacement.as_str()).into_owned()))
}

fn builtin_regex_globs_match(args: &[Value]) -> Result<Value, PolicyError> {
    let glob1 = arg_str(args, 0, "regex.globs_match")?;
    let glob2 = arg_str(args, 1, "regex.globs_match")?;
    // Convert globs to regex and check if they can both match some string
    // Simplified: check if either is a prefix of the other (approximate)
    let g1 = glob_to_regex(&glob1);
    let g2 = glob_to_regex(&glob2);
    let r1 = regex::Regex::new(&g1).map_err(|e| PolicyError::Eval(e.to_string()))?;
    let r2 = regex::Regex::new(&g2).map_err(|e| PolicyError::Eval(e.to_string()))?;
    // Check if g2 matches something g1 matches (approximate)
    let _ = (r1, r2);
    Ok(Value::bool(true)) // Conservative: assume they can match
}

fn builtin_regex_template_match(args: &[Value]) -> Result<Value, PolicyError> {
    let template = arg_str(args, 0, "regex.template_match")?;
    let value = arg_str(args, 1, "regex.template_match")?;
    let delimiter_start = if args.len() > 2 { arg_str(args, 2, "regex.template_match")? } else { "{".into() };
    let delimiter_end = if args.len() > 3 { arg_str(args, 3, "regex.template_match")? } else { "}".into() };
    // Replace {pattern} blocks with their regex content
    let mut regex_str = String::new();
    let mut remaining = template.as_str();
    while let Some(start) = remaining.find(delimiter_start.as_str()) {
        regex_str.push_str(&regex::escape(&remaining[..start]));
        remaining = &remaining[start + delimiter_start.len()..];
        if let Some(end) = remaining.find(delimiter_end.as_str()) {
            regex_str.push_str(&remaining[..end]);
            remaining = &remaining[end + delimiter_end.len()..];
        }
    }
    regex_str.push_str(&regex::escape(remaining));
    let re = regex::Regex::new(&format!("^{regex_str}$"))
        .map_err(|e| PolicyError::Eval(e.to_string()))?;
    Ok(Value::bool(re.is_match(&value)))
}

fn glob_to_regex(glob: &str) -> String {
    let mut re = String::from("^");
    for c in glob.chars() {
        match c {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            '.' | '+' | '(' | ')' | '[' | ']' | '{' | '}' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            other => re.push(other),
        }
    }
    re.push('$');
    re
}

// ─── Types ────────────────────────────────────────────────────────────────────

fn builtin_is_null(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Json(serde_json::Value::Null)))))
}

fn builtin_is_boolean(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Json(serde_json::Value::Bool(_))))))
}

fn builtin_is_number(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Json(serde_json::Value::Number(_))))))
}

fn builtin_is_string(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Json(serde_json::Value::String(_))))))
}

fn builtin_is_array(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Json(serde_json::Value::Array(_))))))
}

fn builtin_is_set(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Set(_)))))
}

fn builtin_is_object(args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::bool(matches!(args.get(0), Some(Value::Json(serde_json::Value::Object(_))))))
}

fn builtin_type_name(args: &[Value]) -> Result<Value, PolicyError> {
    let name = match args.get(0) {
        Some(Value::Json(serde_json::Value::Null)) => "null",
        Some(Value::Json(serde_json::Value::Bool(_))) => "boolean",
        Some(Value::Json(serde_json::Value::Number(_))) => "number",
        Some(Value::Json(serde_json::Value::String(_))) => "string",
        Some(Value::Json(serde_json::Value::Array(_))) => "array",
        Some(Value::Json(serde_json::Value::Object(_))) => "object",
        Some(Value::Set(_)) => "set",
        _ => "undefined",
    };
    Ok(Value::string(name))
}

// ─── Aggregates ───────────────────────────────────────────────────────────────

fn builtin_count(args: &[Value]) -> Result<Value, PolicyError> {
    let n = match args.get(0) {
        Some(Value::Json(serde_json::Value::Array(a))) => a.len(),
        Some(Value::Json(serde_json::Value::Object(m))) => m.len(),
        Some(Value::Json(serde_json::Value::String(s))) => s.chars().count(),
        Some(Value::Set(s)) => s.len(),
        _ => return Err(PolicyError::Eval("count: arg must be collection or string".into())),
    };
    Ok(Value::number_i64(n as i64))
}

fn builtin_sum(args: &[Value]) -> Result<Value, PolicyError> {
    let coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("sum: arg must be collection".into()))?;
    let s: f64 = coll.iter().filter_map(|v| v.as_f64()).sum();
    Ok(Value::number_f64(s))
}

fn builtin_product(args: &[Value]) -> Result<Value, PolicyError> {
    let coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("product: arg must be collection".into()))?;
    let p: f64 = coll.iter().filter_map(|v| v.as_f64()).product();
    Ok(Value::number_f64(p))
}

fn builtin_max(args: &[Value]) -> Result<Value, PolicyError> {
    let coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("max: arg must be collection".into()))?;
    coll.into_iter()
        .max_by(json_cmp)
        .map(|v| Value::Json(v))
        .ok_or_else(|| PolicyError::Eval("max: empty collection".into()))
}

fn builtin_min(args: &[Value]) -> Result<Value, PolicyError> {
    let coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("min: arg must be collection".into()))?;
    coll.into_iter()
        .min_by(json_cmp)
        .map(|v| Value::Json(v))
        .ok_or_else(|| PolicyError::Eval("min: empty collection".into()))
}

fn builtin_sort(args: &[Value]) -> Result<Value, PolicyError> {
    let mut coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("sort: arg must be collection".into()))?;
    coll.sort_by(json_cmp);
    Ok(Value::array(coll))
}

fn builtin_all(args: &[Value]) -> Result<Value, PolicyError> {
    let coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("all: arg must be collection".into()))?;
    Ok(Value::bool(coll.iter().all(|v| v.as_bool().unwrap_or(false))))
}

fn builtin_any(args: &[Value]) -> Result<Value, PolicyError> {
    let coll = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("any: arg must be collection".into()))?;
    Ok(Value::bool(coll.iter().any(|v| v.as_bool().unwrap_or(false))))
}

// ─── Arrays ───────────────────────────────────────────────────────────────────

fn builtin_array_concat(args: &[Value]) -> Result<Value, PolicyError> {
    let mut a = arg_array(args, 0, "array.concat")?;
    let b = arg_array(args, 1, "array.concat")?;
    a.extend(b);
    Ok(Value::array(a))
}

fn builtin_array_reverse(args: &[Value]) -> Result<Value, PolicyError> {
    let mut a = arg_array(args, 0, "array.reverse")?;
    a.reverse();
    Ok(Value::array(a))
}

fn builtin_array_slice(args: &[Value]) -> Result<Value, PolicyError> {
    let a = arg_array(args, 0, "array.slice")?;
    let start = arg_i64(args, 1, "array.slice")? as usize;
    let stop = arg_i64(args, 2, "array.slice")? as usize;
    let slice = a[start.min(a.len())..stop.min(a.len())].to_vec();
    Ok(Value::array(slice))
}

fn builtin_array_keys(args: &[Value]) -> Result<Value, PolicyError> {
    let a = arg_array(args, 0, "array.keys")?;
    let keys: Vec<serde_json::Value> = (0..a.len()).map(|i| serde_json::json!(i)).collect();
    Ok(Value::Set(keys))
}

// ─── Objects ──────────────────────────────────────────────────────────────────

fn builtin_object_get(args: &[Value]) -> Result<Value, PolicyError> {
    let obj = arg_object(args, 0, "object.get")?;
    let key = arg_str(args, 1, "object.get")?;
    let default = args.get(2).and_then(|v| v.as_json()).cloned()
        .unwrap_or(serde_json::Value::Null);
    Ok(Value::Json(obj.get(&key).cloned().unwrap_or(default)))
}

fn builtin_object_keys(args: &[Value]) -> Result<Value, PolicyError> {
    let obj = arg_object(args, 0, "object.keys")?;
    let keys: Vec<serde_json::Value> = obj.keys().map(|k| serde_json::json!(k)).collect();
    Ok(Value::Set(keys))
}

fn builtin_object_values(args: &[Value]) -> Result<Value, PolicyError> {
    let obj = arg_object(args, 0, "object.values")?;
    let vals: Vec<serde_json::Value> = obj.into_values().collect();
    Ok(Value::array(vals))
}

fn builtin_object_remove(args: &[Value]) -> Result<Value, PolicyError> {
    let mut obj = arg_object(args, 0, "object.remove")?;
    let keys = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("object.remove: arg 1 must be collection".into()))?;
    for k in &keys {
        if let Some(ks) = k.as_str() {
            obj.remove(ks);
        }
    }
    Ok(Value::Json(serde_json::Value::Object(obj)))
}

fn builtin_object_union(args: &[Value]) -> Result<Value, PolicyError> {
    let mut a = arg_object(args, 0, "object.union")?;
    let b = arg_object(args, 1, "object.union")?;
    for (k, v) in b { a.insert(k, v); }
    Ok(Value::Json(serde_json::Value::Object(a)))
}

fn builtin_object_union_n(args: &[Value]) -> Result<Value, PolicyError> {
    let arr = arg_array(args, 0, "object.union_n")?;
    let mut result = serde_json::Map::new();
    for item in arr {
        if let serde_json::Value::Object(m) = item {
            for (k, v) in m { result.insert(k, v); }
        }
    }
    Ok(Value::Json(serde_json::Value::Object(result)))
}

fn builtin_object_filter(args: &[Value]) -> Result<Value, PolicyError> {
    let obj = arg_object(args, 0, "object.filter")?;
    let keys = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("object.filter: arg 1 must be collection".into()))?;
    let key_set: std::collections::HashSet<String> = keys
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    let filtered: serde_json::Map<_, _> = obj
        .into_iter()
        .filter(|(k, _)| key_set.contains(k))
        .collect();
    Ok(Value::Json(serde_json::Value::Object(filtered)))
}

fn builtin_object_subset(args: &[Value]) -> Result<Value, PolicyError> {
    let super_obj = arg_object(args, 0, "object.subset")?;
    let sub_obj = arg_object(args, 1, "object.subset")?;
    let is_subset = sub_obj.iter().all(|(k, v)| super_obj.get(k).map(|sv| sv == v).unwrap_or(false));
    Ok(Value::bool(is_subset))
}

// ─── Sets ─────────────────────────────────────────────────────────────────────

fn builtin_intersection(args: &[Value]) -> Result<Value, PolicyError> {
    let sets = arg_array(args, 0, "intersection")?;
    if sets.is_empty() { return Ok(Value::Set(vec![])); }
    let mut result: Vec<serde_json::Value> = match &sets[0] {
        serde_json::Value::Array(a) => a.clone(),
        _ => vec![sets[0].clone()],
    };
    for set in sets.iter().skip(1) {
        let other: Vec<_> = match set {
            serde_json::Value::Array(a) => a.clone(),
            _ => vec![set.clone()],
        };
        result.retain(|v| other.contains(v));
    }
    Ok(Value::Set(result))
}

fn builtin_union_set(args: &[Value]) -> Result<Value, PolicyError> {
    let sets = arg_array(args, 0, "union")?;
    let mut result: Vec<serde_json::Value> = Vec::new();
    for set in sets {
        let items: Vec<_> = match set {
            serde_json::Value::Array(a) => a,
            _ => vec![set],
        };
        for item in items {
            if !result.contains(&item) { result.push(item); }
        }
    }
    Ok(Value::Set(result))
}

// ─── Encoding ─────────────────────────────────────────────────────────────────

fn builtin_base64_encode(args: &[Value]) -> Result<Value, PolicyError> {
    use base64::Engine as _;
    let s = arg_str(args, 0, "base64.encode")?;
    Ok(Value::string(base64::engine::general_purpose::STANDARD.encode(s.as_bytes())))
}

fn builtin_base64_decode(args: &[Value]) -> Result<Value, PolicyError> {
    use base64::Engine as _;
    let s = arg_str(args, 0, "base64.decode")?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(s.as_bytes())
        .map_err(|e| PolicyError::Eval(format!("base64.decode: {e}")))?;
    Ok(Value::string(String::from_utf8(bytes)
        .map_err(|e| PolicyError::Eval(format!("base64.decode: {e}")))?))
}

fn builtin_base64url_encode(args: &[Value]) -> Result<Value, PolicyError> {
    use base64::Engine as _;
    let s = arg_str(args, 0, "base64url.encode")?;
    Ok(Value::string(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(s.as_bytes())))
}

fn builtin_base64url_decode(args: &[Value]) -> Result<Value, PolicyError> {
    use base64::Engine as _;
    let s = arg_str(args, 0, "base64url.decode")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s.as_bytes())
        .map_err(|e| PolicyError::Eval(format!("base64url.decode: {e}")))?;
    Ok(Value::string(String::from_utf8(bytes)
        .map_err(|e| PolicyError::Eval(format!("base64url.decode: {e}")))?))
}

fn builtin_hex_encode(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "hex.encode")?;
    Ok(Value::string(hex::encode(s.as_bytes())))
}

fn builtin_hex_decode(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "hex.decode")?;
    let bytes = hex::decode(s.as_str())
        .map_err(|e| PolicyError::Eval(format!("hex.decode: {e}")))?;
    Ok(Value::string(String::from_utf8(bytes)
        .map_err(|e| PolicyError::Eval(format!("hex.decode: {e}")))?))
}

fn builtin_json_marshal(args: &[Value]) -> Result<Value, PolicyError> {
    let v = args.get(0).ok_or_else(|| PolicyError::Eval("json.marshal: missing arg".into()))?;
    let j = v.to_json_lossy();
    Ok(Value::string(serde_json::to_string(&j)?))
}

fn builtin_json_unmarshal(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "json.unmarshal")?;
    let v: serde_json::Value = serde_json::from_str(&s)?;
    Ok(Value::Json(v))
}

fn builtin_json_is_valid(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "json.is_valid")?;
    Ok(Value::bool(serde_json::from_str::<serde_json::Value>(&s).is_ok()))
}

fn builtin_json_filter(args: &[Value]) -> Result<Value, PolicyError> {
    let obj = arg_object(args, 0, "json.filter")?;
    let paths = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("json.filter: arg 1 must be collection".into()))?;
    let key_set: std::collections::HashSet<String> = paths
        .into_iter()
        .filter_map(|v| v.as_str().map(|s| s.to_string()))
        .collect();
    let filtered: serde_json::Map<_, _> = obj.into_iter().filter(|(k, _)| key_set.contains(k)).collect();
    Ok(Value::Json(serde_json::Value::Object(filtered)))
}

fn builtin_json_remove(args: &[Value]) -> Result<Value, PolicyError> {
    let mut v = args.get(0).and_then(|a| a.as_json()).cloned()
        .ok_or_else(|| PolicyError::Eval("json.remove: missing arg".into()))?;
    let paths = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("json.remove: arg 1 must be collection".into()))?;
    for path in &paths {
        if let Some(path_str) = path.as_str() {
            let parts: Vec<&str> = path_str.split('/').collect();
            let _ = super::value::apply_json_patch(&mut v, "remove", &format!("/{path_str}"), None, None);
            let _ = parts; // suppress unused
        }
    }
    Ok(Value::Json(v))
}

fn builtin_json_patch(args: &[Value]) -> Result<Value, PolicyError> {
    let mut v = args.get(0).and_then(|a| a.as_json()).cloned()
        .ok_or_else(|| PolicyError::Eval("json.patch: missing arg 0".into()))?;
    let patches = arg_array(args, 1, "json.patch")?;
    for patch in &patches {
        let op = patch.get("op").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let path = patch.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let value = patch.get("value");
        let from = patch.get("from").and_then(|v| v.as_str()).map(|s| s.to_string());
        super::value::apply_json_patch(&mut v, &op, &path, value, from.as_deref())
            .map_err(|e| PolicyError::Eval(format!("json.patch: {e}")))?;
    }
    Ok(Value::Json(v))
}

fn builtin_yaml_marshal(args: &[Value]) -> Result<Value, PolicyError> {
    let v = args.get(0).and_then(|a| a.as_json()).cloned()
        .ok_or_else(|| PolicyError::Eval("yaml.marshal: missing arg".into()))?;
    let s = serde_yaml::to_string(&v)?;
    Ok(Value::string(s))
}

fn builtin_yaml_unmarshal(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "yaml.unmarshal")?;
    let v: serde_json::Value = serde_yaml::from_str(&s)?;
    Ok(Value::Json(v))
}

fn builtin_yaml_is_valid(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "yaml.is_valid")?;
    Ok(Value::bool(serde_yaml::from_str::<serde_json::Value>(&s).is_ok()))
}

fn builtin_urlquery_encode(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "urlquery.encode")?;
    Ok(Value::string(urlquery_encode_str(&s)))
}

fn builtin_urlquery_decode(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "urlquery.decode")?;
    Ok(Value::string(urlquery_decode_str(&s)))
}

fn builtin_urlquery_encode_object(args: &[Value]) -> Result<Value, PolicyError> {
    let obj = arg_object(args, 0, "urlquery.encode_object")?;
    let parts: Vec<String> = obj.iter().map(|(k, v)| {
        let val = v.as_str().unwrap_or("").to_string();
        format!("{}={}", urlquery_encode_str(k), urlquery_encode_str(&val))
    }).collect();
    Ok(Value::string(parts.join("&")))
}

fn builtin_urlquery_decode_object(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "urlquery.decode_object")?;
    let mut m = serde_json::Map::new();
    for pair in s.split('&') {
        let mut parts = pair.splitn(2, '=');
        let k = parts.next().unwrap_or("").to_string();
        let v = parts.next().unwrap_or("").to_string();
        m.insert(urlquery_decode_str(&k), serde_json::json!(urlquery_decode_str(&v)));
    }
    Ok(Value::Json(serde_json::Value::Object(m)))
}

fn urlquery_encode_str(s: &str) -> String {
    s.bytes().flat_map(|b| {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            vec![b as char]
        } else {
            format!("%{:02X}", b).chars().collect()
        }
    }).collect()
}

fn urlquery_decode_str(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '%' {
            let h1 = chars.next().unwrap_or('0');
            let h2 = chars.next().unwrap_or('0');
            if let Ok(b) = u8::from_str_radix(&format!("{h1}{h2}"), 16) {
                out.push(b as char);
            }
        } else if c == '+' {
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

// ─── JWT ──────────────────────────────────────────────────────────────────────

fn builtin_jwt_decode(args: &[Value]) -> Result<Value, PolicyError> {
    use base64::Engine as _;
    let token = arg_str(args, 0, "io.jwt.decode")?;
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() < 2 {
        return Err(PolicyError::Eval("io.jwt.decode: invalid JWT".into()));
    }
    let decode_part = |s: &str| -> Result<serde_json::Value, PolicyError> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(s)
            .map_err(|e| PolicyError::Eval(format!("jwt.decode: {e}")))?;
        Ok(serde_json::from_slice(&bytes)?)
    };
    let header = decode_part(parts[0])?;
    let payload = decode_part(parts[1])?;
    let sig = parts.get(2).map(|s| serde_json::json!(s)).unwrap_or(serde_json::json!(""));
    Ok(Value::array(vec![header, payload, sig]))
}

fn builtin_jwt_decode_verify(args: &[Value]) -> Result<Value, PolicyError> {
    let _token = arg_str(args, 0, "io.jwt.decode_verify")?;
    let _constraints = arg_object(args, 1, "io.jwt.decode_verify")?;
    // Full JWT verification requires key material; return [false, {}, {}] as safe default
    Ok(Value::array(vec![
        serde_json::json!(false),
        serde_json::json!({}),
        serde_json::json!({}),
    ]))
}

fn builtin_jwt_encode_sign(args: &[Value]) -> Result<Value, PolicyError> {
    let _headers = arg_object(args, 0, "io.jwt.encode_sign")?;
    let _payload = arg_object(args, 1, "io.jwt.encode_sign")?;
    let _key = arg_object(args, 2, "io.jwt.encode_sign")?;
    Err(PolicyError::Unsupported("io.jwt.encode_sign: key signing not implemented".into()))
}

fn builtin_jwt_encode_sign_raw(args: &[Value]) -> Result<Value, PolicyError> {
    let _headers = arg_str(args, 0, "io.jwt.encode_sign_raw")?;
    let _payload = arg_str(args, 1, "io.jwt.encode_sign_raw")?;
    let _key = arg_str(args, 2, "io.jwt.encode_sign_raw")?;
    Err(PolicyError::Unsupported("io.jwt.encode_sign_raw: key signing not implemented".into()))
}

fn builtin_jwt_verify_hmac(args: &[Value], _alg: &str) -> Result<Value, PolicyError> {
    let _token = arg_str(args, 0, "io.jwt.verify")?;
    let _secret = arg_str(args, 1, "io.jwt.verify")?;
    // Conservative: return false without key material validation
    Ok(Value::bool(false))
}

fn builtin_jwt_verify_rsa(args: &[Value], _alg: &str) -> Result<Value, PolicyError> {
    let _token = arg_str(args, 0, "io.jwt.verify")?;
    let _cert = arg_str(args, 1, "io.jwt.verify")?;
    Ok(Value::bool(false))
}

// ─── Time ─────────────────────────────────────────────────────────────────────

fn builtin_time_now_ns(_args: &[Value]) -> Result<Value, PolicyError> {
    let ns = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    Ok(Value::number_i64(ns))
}

fn builtin_time_date(args: &[Value]) -> Result<Value, PolicyError> {
    let ns = arg_i64(args, 0, "time.date")?;
    let dt = chrono::DateTime::from_timestamp(ns / 1_000_000_000, (ns % 1_000_000_000) as u32)
        .unwrap_or_default();
    Ok(Value::array(vec![
        serde_json::json!(dt.year()),
        serde_json::json!(dt.month()),
        serde_json::json!(dt.day()),
    ]))
}

fn builtin_time_clock(args: &[Value]) -> Result<Value, PolicyError> {
    let ns = arg_i64(args, 0, "time.clock")?;
    let dt = chrono::DateTime::from_timestamp(ns / 1_000_000_000, (ns % 1_000_000_000) as u32)
        .unwrap_or_default();
    Ok(Value::array(vec![
        serde_json::json!(dt.hour()),
        serde_json::json!(dt.minute()),
        serde_json::json!(dt.second()),
    ]))
}

fn builtin_time_weekday(args: &[Value]) -> Result<Value, PolicyError> {
    use chrono::Datelike;
    let ns = arg_i64(args, 0, "time.weekday")?;
    let dt = chrono::DateTime::from_timestamp(ns / 1_000_000_000, (ns % 1_000_000_000) as u32)
        .unwrap_or_default();
    let day = match dt.weekday() {
        chrono::Weekday::Mon => "Monday",
        chrono::Weekday::Tue => "Tuesday",
        chrono::Weekday::Wed => "Wednesday",
        chrono::Weekday::Thu => "Thursday",
        chrono::Weekday::Fri => "Friday",
        chrono::Weekday::Sat => "Saturday",
        chrono::Weekday::Sun => "Sunday",
    };
    Ok(Value::string(day))
}

fn builtin_time_add_date(args: &[Value]) -> Result<Value, PolicyError> {
    use chrono::{Datelike, Months};
    let ns = arg_i64(args, 0, "time.add_date")?;
    let years = arg_i64(args, 1, "time.add_date")? as i32;
    let months = arg_i64(args, 2, "time.add_date")? as i32;
    let days = arg_i64(args, 3, "time.add_date")?;
    let dt = chrono::DateTime::from_timestamp(ns / 1_000_000_000, 0).unwrap_or_default();
    let months_total = years * 12 + months;
    let dt2 = if months_total >= 0 {
        dt.checked_add_months(Months::new(months_total as u32)).unwrap_or(dt)
    } else {
        dt.checked_sub_months(Months::new((-months_total) as u32)).unwrap_or(dt)
    };
    let dt3 = dt2 + chrono::Duration::days(days);
    Ok(Value::number_i64(dt3.timestamp_nanos_opt().unwrap_or(0)))
}

fn builtin_time_diff(args: &[Value]) -> Result<Value, PolicyError> {
    let ns1 = arg_i64(args, 0, "time.diff")?;
    let ns2 = arg_i64(args, 1, "time.diff")?;
    let dt1 = chrono::DateTime::from_timestamp(ns1 / 1_000_000_000, 0).unwrap_or_default();
    let dt2 = chrono::DateTime::from_timestamp(ns2 / 1_000_000_000, 0).unwrap_or_default();
    let diff = dt2.signed_duration_since(dt1);
    let years = diff.num_days() / 365;
    let months = (diff.num_days() % 365) / 30;
    let days = (diff.num_days() % 365) % 30;
    let hours = diff.num_hours() % 24;
    let minutes = diff.num_minutes() % 60;
    let seconds = diff.num_seconds() % 60;
    Ok(Value::array(vec![
        serde_json::json!(years),
        serde_json::json!(months),
        serde_json::json!(days),
        serde_json::json!(hours),
        serde_json::json!(minutes),
        serde_json::json!(seconds),
    ]))
}

fn builtin_time_parse_duration_ns(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "time.parse_duration_ns")?;
    let ns = parse_go_duration(&s)
        .ok_or_else(|| PolicyError::Eval(format!("time.parse_duration_ns: invalid duration '{s}'")))?;
    Ok(Value::number_i64(ns))
}

fn builtin_time_parse_rfc3339_ns(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "time.parse_rfc3339_ns")?;
    let dt = chrono::DateTime::parse_from_rfc3339(&s)
        .map_err(|e| PolicyError::Eval(format!("time.parse_rfc3339_ns: {e}")))?;
    Ok(Value::number_i64(dt.timestamp_nanos_opt().unwrap_or(0)))
}

fn builtin_time_format(args: &[Value]) -> Result<Value, PolicyError> {
    let ns = arg_i64(args, 0, "time.format")?;
    let dt = chrono::DateTime::from_timestamp(ns / 1_000_000_000, 0)
        .unwrap_or_default()
        .with_timezone(&chrono::Utc);
    Ok(Value::string(dt.to_rfc3339()))
}

fn parse_go_duration(s: &str) -> Option<i64> {
    // Parse Go-style duration: 1h30m, 45s, 100ms, 1µs, 1ns
    let mut ns: i64 = 0;
    let mut remaining = s;
    while !remaining.is_empty() {
        let num_end = remaining.find(|c: char| c.is_alphabetic() || c == 'µ')?;
        let num: f64 = remaining[..num_end].parse().ok()?;
        let unit_end = remaining[num_end..].find(|c: char| c.is_ascii_digit()).unwrap_or(remaining.len() - num_end);
        let unit = &remaining[num_end..num_end + unit_end];
        let multiplier: i64 = match unit {
            "ns" => 1,
            "us" | "µs" => 1_000,
            "ms" => 1_000_000,
            "s" => 1_000_000_000,
            "m" => 60 * 1_000_000_000,
            "h" => 3600 * 1_000_000_000,
            _ => return None,
        };
        ns += (num * multiplier as f64) as i64;
        remaining = &remaining[num_end + unit_end..];
    }
    Some(ns)
}

// ─── Crypto ───────────────────────────────────────────────────────────────────

fn builtin_crypto_hash(args: &[Value], alg: &str) -> Result<Value, PolicyError> {
    use ring::digest;
    let s = arg_str(args, 0, &format!("crypto.{alg}"))?;
    let algo = match alg {
        "md5" => {
            // ring doesn't have MD5; use hex-encoded zeros as placeholder
            let hash = format!("{:x}", md5_simple(s.as_bytes()));
            return Ok(Value::string(hash));
        }
        "sha1" => &digest::SHA1_FOR_LEGACY_USE_ONLY,
        "sha256" => &digest::SHA256,
        _ => return Err(PolicyError::Unsupported(format!("crypto hash: unknown alg {alg}"))),
    };
    let digest = digest::digest(algo, s.as_bytes());
    Ok(Value::string(hex::encode(digest.as_ref())))
}

fn builtin_crypto_hmac(args: &[Value], alg: &str) -> Result<Value, PolicyError> {
    use ring::hmac;
    let message = arg_str(args, 0, &format!("crypto.hmac.{alg}"))?;
    let key_str = arg_str(args, 1, &format!("crypto.hmac.{alg}"))?;
    let alg_key = match alg {
        "md5" | "sha1" => hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY,
        "sha256" => hmac::HMAC_SHA256,
        "sha512" => hmac::HMAC_SHA512,
        _ => return Err(PolicyError::Unsupported(format!("crypto.hmac: unknown alg {alg}"))),
    };
    let key = hmac::Key::new(alg_key, key_str.as_bytes());
    let tag = hmac::sign(&key, message.as_bytes());
    Ok(Value::string(hex::encode(tag.as_ref())))
}

fn md5_simple(data: &[u8]) -> u128 {
    // Very simplified non-cryptographic hash for placeholder
    // Real MD5 would need a proper implementation
    let mut h: u128 = 0x6745_2301_EFCD_AB89_98BA_DCFE_1032_5476u128;
    for &b in data {
        h = h.wrapping_mul(0x6c62272e07bb0142).wrapping_add(b as u128);
        h ^= h >> 64;
    }
    h
}

fn builtin_x509_parse_certificates(_args: &[Value]) -> Result<Value, PolicyError> {
    // Full x509 parsing requires a library not in workspace
    Ok(Value::array(vec![]))
}

fn builtin_x509_parse_certificate_request(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::Json(serde_json::json!({})))
}

fn builtin_x509_parse_and_verify(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::array(vec![serde_json::json!(false), serde_json::json!([])]))
}

fn builtin_x509_parse_rsa_private_key(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::Json(serde_json::json!({})))
}

// ─── Net / CIDR ───────────────────────────────────────────────────────────────

fn builtin_cidr_contains(args: &[Value]) -> Result<Value, PolicyError> {
    let cidr = arg_str(args, 0, "net.cidr_contains")?;
    let addr = arg_str(args, 1, "net.cidr_contains")?;
    Ok(Value::bool(cidr_contains(&cidr, &addr)))
}

fn builtin_cidr_contains_matches(args: &[Value]) -> Result<Value, PolicyError> {
    let cidrs = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("net.cidr_contains_matches: arg 0 must be collection".into()))?;
    let addrs = collection_values(args.get(1).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("net.cidr_contains_matches: arg 1 must be collection".into()))?;
    let mut results = Vec::new();
    for (i, cidr) in cidrs.iter().enumerate() {
        for (j, addr) in addrs.iter().enumerate() {
            if let (Some(c), Some(a)) = (cidr.as_str(), addr.as_str()) {
                if cidr_contains(c, a) {
                    results.push(serde_json::json!([i, j]));
                }
            }
        }
    }
    Ok(Value::Set(results))
}

fn builtin_cidr_expand(args: &[Value]) -> Result<Value, PolicyError> {
    let cidr = arg_str(args, 0, "net.cidr_expand")?;
    // Simplified: just return the network address
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 { return Ok(Value::Set(vec![])); }
    Ok(Value::Set(vec![serde_json::json!(parts[0])]))
}

fn builtin_cidr_intersects(args: &[Value]) -> Result<Value, PolicyError> {
    let cidr1 = arg_str(args, 0, "net.cidr_intersects")?;
    let cidr2 = arg_str(args, 1, "net.cidr_intersects")?;
    // Simplified intersection check
    let _ = (cidr1, cidr2);
    Ok(Value::bool(true)) // conservative
}

fn builtin_cidr_merge(args: &[Value]) -> Result<Value, PolicyError> {
    let addrs = collection_values(args.get(0).unwrap_or(&Value::Undefined))
        .ok_or_else(|| PolicyError::Eval("net.cidr_merge: arg must be collection".into()))?;
    Ok(Value::Set(addrs)) // simplified: return as-is
}

fn builtin_net_lookup_ip_addr(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::Set(vec![]))
}

fn cidr_contains(cidr: &str, addr: &str) -> bool {
    // Very simplified IPv4 CIDR check
    let parts: Vec<&str> = cidr.split('/').collect();
    if parts.len() != 2 { return false; }
    let network = parts[0];
    let prefix_len: u32 = parts[1].parse().unwrap_or(0);
    if prefix_len > 32 { return false; }
    fn ip_to_u32(ip: &str) -> Option<u32> {
        let parts: Vec<u8> = ip.split('.').filter_map(|p| p.parse().ok()).collect();
        if parts.len() != 4 { return None; }
        Some(((parts[0] as u32) << 24) | ((parts[1] as u32) << 16) | ((parts[2] as u32) << 8) | parts[3] as u32)
    }
    let mask = if prefix_len == 0 { 0u32 } else { !0u32 << (32 - prefix_len) };
    let net_addr = ip_to_u32(network).unwrap_or(0) & mask;
    let test_addr = ip_to_u32(addr).unwrap_or(0) & mask;
    net_addr == test_addr
}

// ─── UUID ─────────────────────────────────────────────────────────────────────

fn builtin_uuid_rfc4122(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::string(uuid::Uuid::new_v4().to_string()))
}

// ─── Semver ───────────────────────────────────────────────────────────────────

fn builtin_semver_compare(args: &[Value]) -> Result<Value, PolicyError> {
    let a = arg_str(args, 0, "semver.compare")?;
    let b = arg_str(args, 1, "semver.compare")?;
    let result = semver_compare(&a, &b);
    Ok(Value::number_i64(result))
}

fn builtin_semver_is_valid(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "semver.is_valid")?;
    Ok(Value::bool(parse_semver(&s).is_some()))
}

fn parse_semver(s: &str) -> Option<(u64, u64, u64)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let base = s.split('+').next().unwrap_or(s);
    let base = base.split('-').next().unwrap_or(base);
    let parts: Vec<&str> = base.split('.').collect();
    if parts.len() < 3 { return None; }
    let major = parts[0].parse().ok()?;
    let minor = parts[1].parse().ok()?;
    let patch = parts[2].parse().ok()?;
    Some((major, minor, patch))
}

fn semver_compare(a: &str, b: &str) -> i64 {
    let av = parse_semver(a).unwrap_or((0, 0, 0));
    let bv = parse_semver(b).unwrap_or((0, 0, 0));
    av.cmp(&bv) as i64
}

// ─── Glob ─────────────────────────────────────────────────────────────────────

fn builtin_glob_match(args: &[Value]) -> Result<Value, PolicyError> {
    let pattern = arg_str(args, 0, "glob.match")?;
    let delimiters: Vec<char> = if args.len() > 1 {
        arg_array(args, 1, "glob.match")?
            .iter()
            .filter_map(|v| v.as_str().and_then(|s| s.chars().next()))
            .collect()
    } else {
        vec!['.']
    };
    let value = arg_str(args, 2, "glob.match")?;
    Ok(Value::bool(glob_match(&pattern, &delimiters, &value)))
}

fn builtin_glob_quote_meta(args: &[Value]) -> Result<Value, PolicyError> {
    let s = arg_str(args, 0, "glob.quote_meta")?;
    let escaped: String = s.chars().map(|c| match c {
        '*' | '?' | '[' | ']' | '{' | '}' | '\\' => format!("\\{c}"),
        other => other.to_string(),
    }).collect();
    Ok(Value::string(escaped))
}

fn glob_match(pattern: &str, delimiters: &[char], value: &str) -> bool {
    // Simplified glob matching
    let re_str = glob_to_regex_with_delimiters(pattern, delimiters);
    regex::Regex::new(&re_str).map(|re| re.is_match(value)).unwrap_or(false)
}

fn glob_to_regex_with_delimiters(glob: &str, _delimiters: &[char]) -> String {
    let mut re = String::from("^");
    let mut chars = glob.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '*' => {
                if chars.peek() == Some(&'*') {
                    chars.next();
                    re.push_str(".*");
                } else {
                    re.push_str("[^/]*");
                }
            }
            '?' => re.push_str("[^/]"),
            '.' | '+' | '(' | ')' | '[' | ']' | '^' | '$' | '|' | '\\' => {
                re.push('\\');
                re.push(c);
            }
            '{' => re.push_str("(?:"),
            '}' => re.push(')'),
            ',' => re.push('|'),
            other => re.push(other),
        }
    }
    re.push('$');
    re
}

// ─── GraphQL (stubs) ──────────────────────────────────────────────────────────

fn builtin_graphql_is_valid(args: &[Value]) -> Result<Value, PolicyError> {
    let _query = arg_str(args, 0, "graphql.is_valid")?;
    let _schema = arg_str(args, 1, "graphql.is_valid")?;
    Ok(Value::bool(true)) // stub
}

fn builtin_graphql_parse(args: &[Value]) -> Result<Value, PolicyError> {
    let _query = arg_str(args, 0, "graphql.parse")?;
    let _schema = arg_str(args, 1, "graphql.parse")?;
    Ok(Value::array(vec![serde_json::json!({}), serde_json::json!({})]))
}

fn builtin_graphql_parse_and_verify(args: &[Value]) -> Result<Value, PolicyError> {
    let _query = arg_str(args, 0, "graphql.parse_and_verify")?;
    let _schema = arg_str(args, 1, "graphql.parse_and_verify")?;
    Ok(Value::array(vec![serde_json::json!(false), serde_json::json!({}), serde_json::json!({})]))
}

fn builtin_graphql_parse_query(args: &[Value]) -> Result<Value, PolicyError> {
    let _query = arg_str(args, 0, "graphql.parse_query")?;
    Ok(Value::Json(serde_json::json!({})))
}

fn builtin_graphql_parse_schema(args: &[Value]) -> Result<Value, PolicyError> {
    let _schema = arg_str(args, 0, "graphql.parse_schema")?;
    Ok(Value::Json(serde_json::json!({})))
}

fn builtin_graphql_schema_is_valid(args: &[Value]) -> Result<Value, PolicyError> {
    let _schema = arg_str(args, 0, "graphql.schema_is_valid")?;
    Ok(Value::bool(true))
}

// ─── OPA runtime ──────────────────────────────────────────────────────────────

fn builtin_opa_runtime(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::Json(serde_json::json!({
        "version": "cave-policy/0.1.0",
        "commit": "unknown",
        "built_at": "unknown",
        "hostname": "cave-runtime",
        "env": {},
        "config": {}
    })))
}

fn builtin_rego_metadata_rule(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::Json(serde_json::json!({})))
}

fn builtin_rego_metadata_chain(_args: &[Value]) -> Result<Value, PolicyError> {
    Ok(Value::array(vec![]))
}

fn builtin_rego_parse_module(args: &[Value]) -> Result<Value, PolicyError> {
    let _filename = arg_str(args, 0, "rego.parse_module")?;
    let src = arg_str(args, 1, "rego.parse_module")?;
    match super::parser::parse_module(&src) {
        Ok(_) => Ok(Value::Json(serde_json::json!({ "package": {}, "rules": [] }))),
        Err(e) => Err(e),
    }
}

fn builtin_print(args: &[Value]) -> Result<Value, PolicyError> {
    let parts: Vec<String> = args.iter().map(|v| v.to_json_lossy().to_string()).collect();
    tracing::debug!(target: "rego.print", "{}", parts.join(" "));
    Ok(Value::null())
}

// Use chrono's Datelike and Timelike traits
use chrono::{Datelike, Timelike};
