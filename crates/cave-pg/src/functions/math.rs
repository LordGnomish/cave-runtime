//! PostgreSQL math functions.

use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{oid, PgValue};

fn num(args: &[PgValue], n: usize) -> Option<f64> {
    args.get(n)?.to_f64()
}

pub fn abs(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(match &args[0] {
        PgValue::Int2(v) => PgValue::Int2(v.abs()),
        PgValue::Int4(v) => PgValue::Int4(v.abs()),
        PgValue::Int8(v) => PgValue::Int8(v.abs()),
        PgValue::Float4(v) => PgValue::Float4(v.abs()),
        PgValue::Float8(v) => PgValue::Float8(v.abs()),
        PgValue::Numeric(v) => PgValue::Numeric(v.abs()),
        _ => return Err(Error::Pg(PgError::error(SqlState::UNDEFINED_FUNCTION, "abs() requires numeric arg"))),
    })
}

pub fn ceil(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(match &args[0] {
        PgValue::Float4(v) => PgValue::Float4(v.ceil()),
        PgValue::Float8(v) => PgValue::Float8(v.ceil()),
        PgValue::Numeric(v) => PgValue::Numeric(v.ceil()),
        v => v.clone(),  // already integral
    })
}

pub fn floor(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(match &args[0] {
        PgValue::Float4(v) => PgValue::Float4(v.floor()),
        PgValue::Float8(v) => PgValue::Float8(v.floor()),
        PgValue::Numeric(v) => PgValue::Numeric(v.floor()),
        v => v.clone(),
    })
}

pub fn round(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let scale = if args.len() > 1 { args[1].to_i64().unwrap_or(0) } else { 0 };
    Ok(match &args[0] {
        PgValue::Float4(v) => {
            let factor = 10f32.powi(scale as i32);
            PgValue::Float4((v * factor).round() / factor)
        }
        PgValue::Float8(v) => {
            let factor = 10f64.powi(scale as i32);
            PgValue::Float8((v * factor).round() / factor)
        }
        PgValue::Numeric(v) => {
            use rust_decimal::prelude::*;
            PgValue::Numeric(v.round_dp(scale as u32))
        }
        PgValue::Int4(v) => PgValue::Int4(*v),
        PgValue::Int8(v) => PgValue::Int8(*v),
        v => v.clone(),
    })
}

pub fn trunc(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let scale = if args.len() > 1 { args[1].to_i64().unwrap_or(0) } else { 0 };
    Ok(match &args[0] {
        PgValue::Float4(v) => {
            let factor = 10f32.powi(scale as i32);
            PgValue::Float4((v * factor).trunc() / factor)
        }
        PgValue::Float8(v) => {
            let factor = 10f64.powi(scale as i32);
            PgValue::Float8((v * factor).trunc() / factor)
        }
        PgValue::Numeric(v) => {
            use rust_decimal::prelude::*;
            PgValue::Numeric(v.trunc_with_scale(scale as u32))
        }
        v => v.clone(),
    })
}

pub fn sign(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let f = args[0].to_f64().unwrap_or(0.0);
    Ok(match &args[0] {
        PgValue::Float4(_) => PgValue::Float4(f.signum() as f32),
        PgValue::Float8(_) => PgValue::Float8(f.signum()),
        _ => PgValue::Int4(if f > 0.0 { 1 } else if f < 0.0 { -1 } else { 0 }),
    })
}

pub fn mod_fn(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let b = args[1].to_f64().unwrap_or(1.0);
    if b == 0.0 { return Err(Error::Pg(PgError::division_by_zero())); }
    Ok(match (&args[0], &args[1]) {
        (PgValue::Int4(a), PgValue::Int4(b)) => PgValue::Int4(a % b),
        (PgValue::Int8(a), PgValue::Int8(b)) => PgValue::Int8(a % b),
        (PgValue::Float8(a), PgValue::Float8(b)) => PgValue::Float8(a % b),
        _ => PgValue::Float8(args[0].to_f64().unwrap_or(0.0) % b),
    })
}

pub fn power(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let base = args[0].to_f64().unwrap_or(0.0);
    let exp = args[1].to_f64().unwrap_or(0.0);
    Ok(PgValue::Float8(base.powf(exp)))
}

pub fn sqrt(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let v = args[0].to_f64().unwrap_or(0.0);
    if v < 0.0 {
        return Err(Error::Pg(PgError::error(SqlState::INVALID_ARGUMENT_FOR_LOGARITHM,
            "cannot take square root of a negative number")));
    }
    Ok(PgValue::Float8(v.sqrt()))
}

pub fn cbrt(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).cbrt()))
}

pub fn log(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    if args.len() == 2 {
        // log(base, x)
        let base = args[0].to_f64().unwrap_or(0.0);
        let x = args[1].to_f64().unwrap_or(0.0);
        if x <= 0.0 { return Err(Error::Pg(PgError::error(SqlState::INVALID_ARGUMENT_FOR_LOGARITHM, "logarithm of non-positive number"))); }
        return Ok(PgValue::Float8(x.log(base)));
    }
    let v = args[0].to_f64().unwrap_or(0.0);
    if v <= 0.0 { return Err(Error::Pg(PgError::error(SqlState::INVALID_ARGUMENT_FOR_LOGARITHM, "logarithm of non-positive number"))); }
    Ok(PgValue::Float8(v.log10()))
}

pub fn log10(args: Vec<PgValue>) -> Result<PgValue> { log(args) }

pub fn ln(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let v = args[0].to_f64().unwrap_or(0.0);
    if v <= 0.0 { return Err(Error::Pg(PgError::error(SqlState::INVALID_ARGUMENT_FOR_LOGARITHM, "logarithm of non-positive number"))); }
    Ok(PgValue::Float8(v.ln()))
}

pub fn exp(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).exp()))
}

pub fn pi(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Float8(std::f64::consts::PI))
}

pub fn random(args: Vec<PgValue>) -> Result<PgValue> {
    Ok(PgValue::Float8(rand::random::<f64>()))
}

pub fn setseed(args: Vec<PgValue>) -> Result<PgValue> {
    // Not truly settable without global state; accept call and ignore
    Ok(PgValue::Void)
}

pub fn greatest(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() { return Ok(PgValue::Null); }
    let mut best = PgValue::Null;
    for a in args {
        if a.is_null() { continue; }
        if best.is_null() { best = a; continue; }
        if a.compare(&best) == Some(std::cmp::Ordering::Greater) { best = a; }
    }
    Ok(best)
}

pub fn least(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() { return Ok(PgValue::Null); }
    let mut best = PgValue::Null;
    for a in args {
        if a.is_null() { continue; }
        if best.is_null() { best = a; continue; }
        if a.compare(&best) == Some(std::cmp::Ordering::Less) { best = a; }
    }
    Ok(best)
}

pub fn width_bucket(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 4 { return Ok(PgValue::Null); }
    let v = args[0].to_f64().unwrap_or(0.0);
    let low = args[1].to_f64().unwrap_or(0.0);
    let high = args[2].to_f64().unwrap_or(1.0);
    let count = args[3].to_i64().unwrap_or(1).max(1) as f64;
    if v < low { return Ok(PgValue::Int4(0)); }
    if v >= high { return Ok(PgValue::Int4(count as i32 + 1)); }
    Ok(PgValue::Int4((((v - low) / (high - low) * count).floor() + 1.0) as i32))
}

pub fn degrees(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).to_degrees()))
}

pub fn radians(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).to_radians()))
}

pub fn sin(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).sin()))
}

pub fn cos(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).cos()))
}

pub fn tan(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).tan()))
}

pub fn asin(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).asin()))
}

pub fn acos(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).acos()))
}

pub fn atan(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).atan()))
}

pub fn atan2(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let y = args[0].to_f64().unwrap_or(0.0);
    let x = args[1].to_f64().unwrap_or(0.0);
    Ok(PgValue::Float8(y.atan2(x)))
}

pub fn sinh(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).sinh()))
}

pub fn cosh(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).cosh()))
}

pub fn tanh(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(PgValue::Float8(args[0].to_f64().unwrap_or(0.0).tanh()))
}

pub fn factorial(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let n = args[0].to_i64().unwrap_or(0);
    if n < 0 { return Err(Error::Pg(PgError::error(SqlState::NUMERIC_VALUE_OUT_OF_RANGE, "factorial of negative number"))); }
    let mut result: i64 = 1;
    for i in 2..=(n.min(20)) { result *= i; }
    Ok(PgValue::Numeric(rust_decimal::Decimal::from(result)))
}

pub fn gcd(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let mut a = args[0].to_i64().unwrap_or(0).abs();
    let mut b = args[1].to_i64().unwrap_or(0).abs();
    while b != 0 { let t = b; b = a % b; a = t; }
    Ok(PgValue::Int8(a))
}

pub fn lcm(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let a = args[0].to_i64().unwrap_or(0).abs();
    let b = args[1].to_i64().unwrap_or(0).abs();
    if a == 0 || b == 0 { return Ok(PgValue::Int8(0)); }
    let g = { let (mut x, mut y) = (a, b); while y != 0 { let t = y; y = x % y; x = t; } x };
    Ok(PgValue::Int8(a / g * b))
}

pub fn min_scale(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Numeric(v) => Ok(PgValue::Int4(v.scale() as i32)),
        _ => Ok(PgValue::Int4(0)),
    }
}

pub fn trim_scale(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Numeric(v) => Ok(PgValue::Numeric(v.normalize())),
        v => Ok(v.clone()),
    }
}

pub fn scale_fn(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Numeric(v) => Ok(PgValue::Int4(v.scale() as i32)),
        _ => Ok(PgValue::Int4(0)),
    }
}

pub fn div(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() || args[1].is_null() { return Ok(PgValue::Null); }
    let b = args[1].to_f64().unwrap_or(1.0);
    if b == 0.0 { return Err(Error::Pg(PgError::division_by_zero())); }
    let a = args[0].to_f64().unwrap_or(0.0);
    Ok(PgValue::Int8((a / b).trunc() as i64))
}
