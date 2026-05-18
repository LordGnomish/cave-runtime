// SPDX-License-Identifier: AGPL-3.0-or-later
//! Aggregate functions: COUNT, SUM, AVG, MIN, MAX.

use crate::types::SqlValue;

pub fn count(values: &[SqlValue]) -> i64 {
    values.iter().filter(|v| !matches!(v, SqlValue::Null)).count() as i64
}

pub fn count_all(len: usize) -> i64 {
    len as i64
}

pub fn sum(values: &[SqlValue]) -> Option<SqlValue> {
    let mut total = 0i64;
    for val in values {
        match val {
            SqlValue::Int4(n) => total += *n as i64,
            SqlValue::Int8(n) => total += n,
            SqlValue::Numeric(f) => total = (*f as i64) + total,
            SqlValue::Null => {}
            _ => return None,
        }
    }
    Some(SqlValue::Int8(total))
}

pub fn avg(values: &[SqlValue]) -> Option<SqlValue> {
    let mut total = 0.0f64;
    let mut count = 0usize;
    for val in values {
        match val {
            SqlValue::Int4(n) => {
                total += *n as f64;
                count += 1;
            }
            SqlValue::Int8(n) => {
                total += *n as f64;
                count += 1;
            }
            SqlValue::Numeric(f) => {
                total += f;
                count += 1;
            }
            SqlValue::Null => {}
            _ => return None,
        }
    }
    if count == 0 {
        Some(SqlValue::Null)
    } else {
        Some(SqlValue::Numeric(total / count as f64))
    }
}

pub fn min(values: &[SqlValue]) -> Option<SqlValue> {
    let mut result = None;
    for val in values {
        if !matches!(val, SqlValue::Null) {
            result = match (&result, val) {
                (None, v) => Some(v.clone()),
                (Some(r), v) => {
                    if r.compare(v).map(|cmp| cmp.is_ge()).unwrap_or(false) {
                        Some(v.clone())
                    } else {
                        result
                    }
                }
            };
        }
    }
    result
}

pub fn max(values: &[SqlValue]) -> Option<SqlValue> {
    let mut result = None;
    for val in values {
        if !matches!(val, SqlValue::Null) {
            result = match (&result, val) {
                (None, v) => Some(v.clone()),
                (Some(r), v) => {
                    if r.compare(v).map(|cmp| cmp.is_le()).unwrap_or(false) {
                        Some(v.clone())
                    } else {
                        result
                    }
                }
            };
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_count() {
        let values = vec![SqlValue::Int4(1), SqlValue::Null, SqlValue::Int4(3)];
        assert_eq!(count(&values), 2);
    }

    #[test]
    fn test_count_all() {
        assert_eq!(count_all(5), 5);
    }

    #[test]
    fn test_sum() {
        let values = vec![SqlValue::Int4(1), SqlValue::Int4(2), SqlValue::Int4(3)];
        let result = sum(&values).unwrap();
        assert_eq!(result, SqlValue::Int8(6));
    }

    #[test]
    fn test_avg() {
        let values = vec![SqlValue::Int4(10), SqlValue::Int4(20)];
        let result = avg(&values).unwrap();
        assert!(matches!(result, SqlValue::Numeric(f) if (f - 15.0).abs() < 0.01));
    }

    #[test]
    fn test_min() {
        let values = vec![SqlValue::Int4(3), SqlValue::Int4(1), SqlValue::Int4(2)];
        let result = min(&values).unwrap();
        assert_eq!(result, SqlValue::Int4(1));
    }

    #[test]
    fn test_max() {
        let values = vec![SqlValue::Int4(3), SqlValue::Int4(1), SqlValue::Int4(2)];
        let result = max(&values).unwrap();
        assert_eq!(result, SqlValue::Int4(3));
    }
}
