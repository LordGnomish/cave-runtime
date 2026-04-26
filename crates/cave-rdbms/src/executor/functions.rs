//! Built-in SQL functions.

use crate::types::SqlValue;
use chrono::Local;

pub fn call_builtin(name: &str, args: &[SqlValue]) -> Result<SqlValue, String> {
    match name.to_lowercase().as_str() {
        "now" | "current_timestamp" => Ok(SqlValue::Timestamp(
            Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        )),
        "current_date" => Ok(SqlValue::Date(
            Local::now().format("%Y-%m-%d").to_string(),
        )),
        "coalesce" => {
            for arg in args {
                if !matches!(arg, SqlValue::Null) {
                    return Ok(arg.clone());
                }
            }
            Ok(SqlValue::Null)
        }
        "nullif" => {
            if args.len() < 2 {
                return Err("nullif requires 2 arguments".to_string());
            }
            if args[0] == args[1] {
                Ok(SqlValue::Null)
            } else {
                Ok(args[0].clone())
            }
        }
        "lower" => {
            if args.is_empty() {
                return Err("lower requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Text(s) => Ok(SqlValue::Text(s.to_lowercase())),
                _ => Err("lower requires text argument".to_string()),
            }
        }
        "upper" => {
            if args.is_empty() {
                return Err("upper requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Text(s) => Ok(SqlValue::Text(s.to_uppercase())),
                _ => Err("upper requires text argument".to_string()),
            }
        }
        "trim" => {
            if args.is_empty() {
                return Err("trim requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Text(s) => Ok(SqlValue::Text(s.trim().to_string())),
                _ => Err("trim requires text argument".to_string()),
            }
        }
        "length" => {
            if args.is_empty() {
                return Err("length requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Text(s) => Ok(SqlValue::Int4(s.len() as i32)),
                _ => Err("length requires text argument".to_string()),
            }
        }
        "concat" => {
            let mut result = String::new();
            for arg in args {
                match arg {
                    SqlValue::Text(s) => result.push_str(s),
                    SqlValue::Null => {}
                    _ => result.push_str(&arg.to_string()),
                }
            }
            Ok(SqlValue::Text(result))
        }
        "substring" => {
            if args.len() < 2 {
                return Err("substring requires at least 2 arguments".to_string());
            }
            match (&args[0], &args[1]) {
                (SqlValue::Text(s), SqlValue::Int4(start)) => {
                    let start = (*start - 1).max(0) as usize;
                    let len = if args.len() > 2 {
                        args[2].as_i32().unwrap_or(s.len() as i32) as usize
                    } else {
                        s.len()
                    };
                    Ok(SqlValue::Text(s[start..].chars().take(len).collect()))
                }
                _ => Err("substring requires text and int arguments".to_string()),
            }
        }
        "abs" => {
            if args.is_empty() {
                return Err("abs requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Int4(n) => Ok(SqlValue::Int4(n.abs())),
                SqlValue::Int8(n) => Ok(SqlValue::Int8(n.abs())),
                SqlValue::Numeric(f) => Ok(SqlValue::Numeric(f.abs())),
                _ => Err("abs requires numeric argument".to_string()),
            }
        }
        "round" => {
            if args.is_empty() {
                return Err("round requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Numeric(f) => {
                    let decimals = args
                        .get(1)
                        .and_then(|v| v.as_i32())
                        .unwrap_or(0);
                    let multiplier = 10_f64.powi(decimals);
                    Ok(SqlValue::Numeric((f * multiplier).round() / multiplier))
                }
                SqlValue::Int4(n) => Ok(SqlValue::Int4(*n)),
                SqlValue::Int8(n) => Ok(SqlValue::Int8(*n)),
                _ => Err("round requires numeric argument".to_string()),
            }
        }
        "ceil" => {
            if args.is_empty() {
                return Err("ceil requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Numeric(f) => Ok(SqlValue::Numeric(f.ceil())),
                _ => Err("ceil requires numeric argument".to_string()),
            }
        }
        "floor" => {
            if args.is_empty() {
                return Err("floor requires 1 argument".to_string());
            }
            match &args[0] {
                SqlValue::Numeric(f) => Ok(SqlValue::Numeric(f.floor())),
                _ => Err("floor requires numeric argument".to_string()),
            }
        }
        _ => Err(format!("unknown function: {}", name)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coalesce() {
        let args = vec![SqlValue::Null, SqlValue::Int4(5)];
        let result = call_builtin("coalesce", &args).unwrap();
        assert_eq!(result, SqlValue::Int4(5));
    }

    #[test]
    fn test_lower() {
        let args = vec![SqlValue::Text("HELLO".to_string())];
        let result = call_builtin("lower", &args).unwrap();
        assert_eq!(result, SqlValue::Text("hello".to_string()));
    }

    #[test]
    fn test_upper() {
        let args = vec![SqlValue::Text("hello".to_string())];
        let result = call_builtin("upper", &args).unwrap();
        assert_eq!(result, SqlValue::Text("HELLO".to_string()));
    }

    #[test]
    fn test_length() {
        let args = vec![SqlValue::Text("hello".to_string())];
        let result = call_builtin("length", &args).unwrap();
        assert_eq!(result, SqlValue::Int4(5));
    }

    #[test]
    fn test_abs() {
        let args = vec![SqlValue::Int4(-5)];
        let result = call_builtin("abs", &args).unwrap();
        assert_eq!(result, SqlValue::Int4(5));
    }

    #[test]
    fn test_round() {
        let args = vec![SqlValue::Numeric(3.7)];
        let result = call_builtin("round", &args).unwrap();
        assert!(matches!(result, SqlValue::Numeric(f) if (f - 4.0).abs() < 0.01));
    }
}
