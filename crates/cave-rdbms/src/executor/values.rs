// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Value coercion and comparison helpers.

use crate::types::SqlValue;

pub fn coerce_to_common_type(values: &[SqlValue]) -> Option<SqlValue> {
    if values.is_empty() {
        return None;
    }
    if values.len() == 1 {
        return Some(values[0].clone());
    }
    let mut result = values[0].clone();
    for val in &values[1..] {
        result = coerce_pair(&result, val)?;
    }
    Some(result)
}

fn coerce_pair(a: &SqlValue, b: &SqlValue) -> Option<SqlValue> {
    match (a, b) {
        (SqlValue::Null, b) => Some(b.clone()),
        (a, SqlValue::Null) => Some(a.clone()),
        (SqlValue::Int4(x), SqlValue::Int4(y)) => Some(SqlValue::Int4(*x.max(y))),
        (SqlValue::Int8(x), SqlValue::Int8(y)) => Some(SqlValue::Int8(*x.max(y))),
        (SqlValue::Numeric(x), SqlValue::Numeric(y)) => {
            Some(SqlValue::Numeric(if x >= y { *x } else { *y }))
        }
        (SqlValue::Text(x), SqlValue::Text(_y)) => Some(SqlValue::Text(x.clone())),
        (SqlValue::Bool(x), SqlValue::Bool(y)) => Some(SqlValue::Bool(*x || *y)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coerce_to_common_type() {
        let vals = vec![SqlValue::Null, SqlValue::Int4(5)];
        let result = coerce_to_common_type(&vals).unwrap();
        assert_eq!(result, SqlValue::Int4(5));
    }

    #[test]
    fn test_coerce_pair() {
        let a = SqlValue::Int4(10);
        let b = SqlValue::Int4(20);
        let result = coerce_pair(&a, &b).unwrap();
        assert_eq!(result, SqlValue::Int4(20));
    }
}
