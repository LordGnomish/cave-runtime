//! PostgreSQL array functions.

use crate::error::{Error, PgError, Result, SqlState};
use crate::types::{oid, PgValue};

fn get_array(args: &[PgValue], i: usize) -> Option<(&Vec<PgValue>, u32)> {
    match args.get(i) {
        Some(PgValue::Array { elements, element_oid }) => Some((elements, *element_oid)),
        _ => None,
    }
}

pub fn append(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    if let PgValue::Array { mut elements, element_oid } = args[0].clone() {
        elements.push(args[1].clone());
        Ok(PgValue::Array { elements, element_oid })
    } else {
        Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "array_append requires array")))
    }
}

pub fn prepend(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[1].is_null() { return Ok(PgValue::Null); }
    if let PgValue::Array { mut elements, element_oid } = args[1].clone() {
        elements.insert(0, args[0].clone());
        Ok(PgValue::Array { elements, element_oid })
    } else {
        Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "array_prepend requires array")))
    }
}

pub fn cat(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 { return Ok(PgValue::Null); }
    match (args[0].clone(), args[1].clone()) {
        (PgValue::Null, b) | (b, PgValue::Null) => Ok(b),
        (PgValue::Array { mut elements, element_oid }, PgValue::Array { elements: other, .. }) => {
            elements.extend(other);
            Ok(PgValue::Array { elements, element_oid })
        }
        _ => Err(Error::Pg(PgError::error(SqlState::DATATYPE_MISMATCH, "array_cat requires two arrays"))),
    }
}

pub fn length(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    let _dim = args.get(1).and_then(|v| v.to_i64()).unwrap_or(1);
    match &args[0] {
        PgValue::Array { elements, .. } => Ok(PgValue::Int4(elements.len() as i32)),
        _ => Ok(PgValue::Null),
    }
}

pub fn ndims(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Array { .. } => Ok(PgValue::Int4(1)), // We support 1-D arrays
        _ => Ok(PgValue::Null),
    }
}

pub fn dims(args: Vec<PgValue>) -> Result<PgValue> {
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Array { elements, .. } => Ok(PgValue::Text(format!("[1:{}]", elements.len()))),
        _ => Ok(PgValue::Null),
    }
}

pub fn lower(args: Vec<PgValue>) -> Result<PgValue> {
    // array_lower(array, dim) — always returns 1 for 1-D
    Ok(PgValue::Int4(1))
}

pub fn upper(args: Vec<PgValue>) -> Result<PgValue> {
    // array_upper(array, dim) — returns length for 1-D
    length(args)
}

pub fn unnest(args: Vec<PgValue>) -> Result<PgValue> {
    // Returns the array as-is; the executor handles set-returning function expansion
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    Ok(args[0].clone())
}

pub fn position(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let target = &args[1];
    let start = args.get(2).and_then(|v| v.to_i64()).unwrap_or(1) as usize;
    match &args[0] {
        PgValue::Array { elements, .. } => {
            for (i, el) in elements.iter().enumerate().skip(start.saturating_sub(1)) {
                if el == target { return Ok(PgValue::Int4((i + 1) as i32)); }
            }
            Ok(PgValue::Null)
        }
        _ => Ok(PgValue::Null),
    }
}

pub fn positions(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let target = &args[1];
    match &args[0] {
        PgValue::Array { elements, .. } => {
            let found: Vec<PgValue> = elements.iter().enumerate()
                .filter(|(_, el)| *el == target)
                .map(|(i, _)| PgValue::Int4((i + 1) as i32))
                .collect();
            Ok(PgValue::Array { element_oid: oid::INT4, elements: found })
        }
        _ => Ok(PgValue::Null),
    }
}

pub fn remove(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let target = &args[1];
    match args[0].clone() {
        PgValue::Array { mut elements, element_oid } => {
            elements.retain(|el| el != target);
            Ok(PgValue::Array { elements, element_oid })
        }
        _ => Ok(PgValue::Null),
    }
}

pub fn replace(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 3 || args[0].is_null() { return Ok(PgValue::Null); }
    let search = &args[1];
    let replacement = args[2].clone();
    match args[0].clone() {
        PgValue::Array { mut elements, element_oid } => {
            for el in &mut elements {
                if el == search { *el = replacement.clone(); }
            }
            Ok(PgValue::Array { elements, element_oid })
        }
        _ => Ok(PgValue::Null),
    }
}

pub fn fill(args: Vec<PgValue>) -> Result<PgValue> {
    if args.len() < 2 || args[0].is_null() { return Ok(PgValue::Null); }
    let val = args[0].clone();
    let oid = val.oid();
    let len = match &args[1] {
        PgValue::Array { elements, .. } => {
            elements.get(0).and_then(|v| v.to_i64()).unwrap_or(0)
        }
        v => v.to_i64().unwrap_or(0),
    };
    Ok(PgValue::Array {
        element_oid: oid,
        elements: (0..len.max(0)).map(|_| val.clone()).collect(),
    })
}

pub fn cardinality(args: Vec<PgValue>) -> Result<PgValue> {
    // Total number of elements in an array (including multi-dim)
    if args.is_empty() || args[0].is_null() { return Ok(PgValue::Null); }
    match &args[0] {
        PgValue::Array { elements, .. } => Ok(PgValue::Int4(elements.len() as i32)),
        _ => Ok(PgValue::Null),
    }
}

pub fn agg(args: Vec<PgValue>) -> Result<PgValue> {
    // Not a normal function — aggregate handled by executor. Return arg as-is.
    if args.is_empty() { return Ok(PgValue::Null); }
    Ok(args[0].clone())
}

/// Array containment: a @> b — a contains all elements of b
pub fn contains(a: &PgValue, b: &PgValue) -> bool {
    match (a, b) {
        (PgValue::Array { elements: ae, .. }, PgValue::Array { elements: be, .. }) => {
            be.iter().all(|bv| ae.iter().any(|av| av == bv))
        }
        _ => false,
    }
}

/// Array overlap: a && b — a and b have common elements
pub fn overlaps(a: &PgValue, b: &PgValue) -> bool {
    match (a, b) {
        (PgValue::Array { elements: ae, .. }, PgValue::Array { elements: be, .. }) => {
            be.iter().any(|bv| ae.iter().any(|av| av == bv))
        }
        _ => false,
    }
}
