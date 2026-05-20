// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Bitmap commands: SETBIT, GETBIT, BITCOUNT, BITOP, BITPOS, BITFIELD.

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{Entry, Value, bytes_to_i64};

// ── SETBIT ───────────────────────────────────────────────────────────────────

pub fn cmd_setbit(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 4 {
        return Err(CacheError::wrong_arity("setbit"));
    }
    let key = args[1].clone();
    let offset = bytes_to_i64(&args[2]).ok_or(CacheError::BitOffset)? as usize;
    let bit_val = bytes_to_i64(&args[3]).ok_or(CacheError::BitValue)?;

    if bit_val != 0 && bit_val != 1 {
        return Err(CacheError::BitValue);
    }
    if offset > 2u64.pow(32) as usize {
        return Err(CacheError::BitOffset);
    }

    let byte_idx = offset / 8;
    let bit_idx = 7 - (offset % 8); // Redis stores bits MSB first

    let old_bit = match db.get_typed_mut(&key, "string")? {
        Some(entry) => match &mut entry.value {
            Value::String(v) => {
                if v.len() <= byte_idx {
                    v.resize(byte_idx + 1, 0);
                }
                let old = (v[byte_idx] >> bit_idx) & 1;
                if bit_val == 1 {
                    v[byte_idx] |= 1 << bit_idx;
                } else {
                    v[byte_idx] &= !(1 << bit_idx);
                }
                old
            }
            _ => unreachable!(),
        },
        None => {
            let mut v = vec![0u8; byte_idx + 1];
            if bit_val == 1 {
                v[byte_idx] |= 1 << bit_idx;
            }
            db.insert(key, Entry::new(Value::String(v)));
            0
        }
    };

    Ok(Resp::Integer(old_bit as i64))
}

// ── GETBIT ───────────────────────────────────────────────────────────────────

pub fn cmd_getbit(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() != 3 {
        return Err(CacheError::wrong_arity("getbit"));
    }
    let offset = bytes_to_i64(&args[2]).ok_or(CacheError::BitOffset)? as usize;

    match db.get_typed(&args[1], "string")? {
        Some(e) => match &e.value {
            Value::String(v) => {
                let byte_idx = offset / 8;
                let bit_idx = 7 - (offset % 8);
                if byte_idx >= v.len() {
                    return Ok(Resp::Integer(0));
                }
                Ok(Resp::Integer(((v[byte_idx] >> bit_idx) & 1) as i64))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

// ── BITCOUNT ─────────────────────────────────────────────────────────────────

pub fn cmd_bitcount(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 || args.len() > 5 {
        return Err(CacheError::wrong_arity("bitcount"));
    }

    match db.get_typed(&args[1], "string")? {
        Some(e) => match &e.value {
            Value::String(v) => {
                let (start, end) = if args.len() >= 4 {
                    let s = bytes_to_i64(&args[2]).ok_or(CacheError::NotInteger)?;
                    let e = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;
                    // Optional BYTE/BIT unit
                    let is_bit = args
                        .get(4)
                        .map(|u| u.to_ascii_uppercase() == b"BIT")
                        .unwrap_or(false);
                    if is_bit {
                        bit_range_to_bytes(s, e, v.len())
                    } else {
                        byte_range(s, e, v.len())
                    }
                } else {
                    (0, v.len().saturating_sub(1))
                };

                if start > end || start >= v.len() {
                    return Ok(Resp::Integer(0));
                }

                let count: i64 = v[start..=end.min(v.len() - 1)]
                    .iter()
                    .map(|b| b.count_ones() as i64)
                    .sum();
                Ok(Resp::Integer(count))
            }
            _ => unreachable!(),
        },
        None => Ok(Resp::Integer(0)),
    }
}

fn byte_range(start: i64, end: i64, len: usize) -> (usize, usize) {
    let len = len as i64;
    let s = if start < 0 {
        (len + start).max(0)
    } else {
        start.min(len - 1)
    } as usize;
    let e = if end < 0 {
        (len + end).max(0)
    } else {
        end.min(len - 1)
    } as usize;
    (s, e)
}

fn bit_range_to_bytes(start: i64, end: i64, len: usize) -> (usize, usize) {
    let bits = (len * 8) as i64;
    let s = if start < 0 {
        (bits + start).max(0)
    } else {
        start.min(bits - 1)
    };
    let e = if end < 0 {
        (bits + end).max(0)
    } else {
        end.min(bits - 1)
    };
    (s as usize / 8, e as usize / 8)
}

// ── BITOP ────────────────────────────────────────────────────────────────────

pub fn cmd_bitop(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    // args: [BITOP, operation, destkey, srckey ...]
    if args.len() < 4 {
        return Err(CacheError::wrong_arity("bitop"));
    }
    let op = &args[1].to_ascii_uppercase();
    let dst = args[2].clone();

    match op.as_slice() {
        b"NOT" => {
            if args.len() != 4 {
                return Err(CacheError::wrong_arity("bitop not"));
            }
            let src = get_bytes(db, &args[3])?;
            let result: Vec<u8> = src.iter().map(|b| !b).collect();
            let len = result.len() as i64;
            db.insert(dst, Entry::new(Value::String(result)));
            Ok(Resp::Integer(len))
        }
        b"AND" | b"OR" | b"XOR" => {
            let srcs: Vec<Vec<u8>> = args[3..]
                .iter()
                .map(|k| get_bytes(db, k))
                .collect::<CacheResult<_>>()?;

            let max_len = srcs.iter().map(|s| s.len()).max().unwrap_or(0);
            let mut result = vec![0u8; max_len];

            match op.as_slice() {
                b"AND" => {
                    for (i, b) in result.iter_mut().enumerate() {
                        *b = srcs
                            .iter()
                            .map(|s| s.get(i).copied().unwrap_or(0))
                            .fold(0xFF, |acc, x| acc & x);
                    }
                }
                b"OR" => {
                    for (i, b) in result.iter_mut().enumerate() {
                        *b = srcs
                            .iter()
                            .map(|s| s.get(i).copied().unwrap_or(0))
                            .fold(0, |acc, x| acc | x);
                    }
                }
                b"XOR" => {
                    for (i, b) in result.iter_mut().enumerate() {
                        *b = srcs
                            .iter()
                            .map(|s| s.get(i).copied().unwrap_or(0))
                            .fold(0, |acc, x| acc ^ x);
                    }
                }
                _ => unreachable!(),
            }

            let len = result.len() as i64;
            db.insert(dst, Entry::new(Value::String(result)));
            Ok(Resp::Integer(len))
        }
        _ => Err(CacheError::generic(format!(
            "ERR Unknown operation '{}'. Please specify AND, OR, XOR, or NOT",
            std::str::from_utf8(op).unwrap_or("?")
        ))),
    }
}

fn get_bytes(db: &mut Db, key: &[u8]) -> CacheResult<Vec<u8>> {
    match db.get_typed(key, "string")? {
        Some(e) => match &e.value {
            Value::String(v) => Ok(v.clone()),
            _ => unreachable!(),
        },
        None => Ok(vec![]),
    }
}

// ── BITPOS ───────────────────────────────────────────────────────────────────

pub fn cmd_bitpos(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 3 {
        return Err(CacheError::wrong_arity("bitpos"));
    }
    let target = bytes_to_i64(&args[2]).ok_or(CacheError::BitValue)?;
    if target != 0 && target != 1 {
        return Err(CacheError::BitValue);
    }

    let bytes = match db.get_typed(&args[1], "string")? {
        Some(e) => match &e.value {
            Value::String(v) => v.clone(),
            _ => unreachable!(),
        },
        None => vec![],
    };

    let (start_byte, end_byte) = if args.len() >= 5 {
        let s = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;
        let e = bytes_to_i64(&args[4]).ok_or(CacheError::NotInteger)?;
        byte_range(s, e, bytes.len())
    } else if args.len() >= 4 {
        let s = bytes_to_i64(&args[3]).ok_or(CacheError::NotInteger)?;
        let e = if s < 0 {
            bytes.len() as i64 - 1
        } else {
            bytes.len() as i64 - 1
        };
        byte_range(s, e, bytes.len())
    } else {
        (0, bytes.len().saturating_sub(1))
    };

    let search_bytes = if bytes.is_empty() {
        &[][..]
    } else if start_byte < bytes.len() {
        &bytes[start_byte..=(end_byte.min(bytes.len() - 1))]
    } else {
        &[][..]
    };

    for (bi, &byte) in search_bytes.iter().enumerate() {
        for bit in (0..8).rev() {
            let b = (byte >> bit) & 1;
            if b as i64 == target {
                return Ok(Resp::Integer(((start_byte + bi) * 8 + (7 - bit)) as i64));
            }
        }
    }

    // Not found
    if target == 1 {
        Ok(Resp::Integer(-1))
    } else {
        Ok(Resp::Integer(bytes.len() as i64 * 8))
    }
}

// ── BITFIELD ─────────────────────────────────────────────────────────────────

pub fn cmd_bitfield(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("bitfield"));
    }
    let key = args[1].clone();

    let mut results = Vec::new();
    let mut overflow = OverflowPolicy::Wrap;
    let mut i = 2;

    while i < args.len() {
        match args[i].to_ascii_uppercase().as_slice() {
            b"GET" => {
                let (is_signed, bits) = parse_bitfield_type(&args[i + 1])?;
                let offset = parse_bitfield_offset(&args[i + 2])?;
                let val = bitfield_get(db, &key, is_signed, bits, offset)?;
                results.push(Resp::Integer(val));
                i += 3;
            }
            b"SET" => {
                let (is_signed, bits) = parse_bitfield_type(&args[i + 1])?;
                let offset = parse_bitfield_offset(&args[i + 2])?;
                let value = bytes_to_i64(&args[i + 3]).ok_or(CacheError::NotInteger)?;
                let old = bitfield_set(db, &key, is_signed, bits, offset, value)?;
                results.push(Resp::Integer(old));
                i += 4;
            }
            b"INCRBY" => {
                let (is_signed, bits) = parse_bitfield_type(&args[i + 1])?;
                let offset = parse_bitfield_offset(&args[i + 2])?;
                let delta = bytes_to_i64(&args[i + 3]).ok_or(CacheError::NotInteger)?;
                let new_val = bitfield_incrby(db, &key, is_signed, bits, offset, delta, overflow)?;
                match new_val {
                    Some(v) => results.push(Resp::Integer(v)),
                    None => results.push(Resp::nil()),
                }
                i += 4;
            }
            b"OVERFLOW" => {
                overflow = match args[i + 1].to_ascii_uppercase().as_slice() {
                    b"WRAP" => OverflowPolicy::Wrap,
                    b"SAT" => OverflowPolicy::Sat,
                    b"FAIL" => OverflowPolicy::Fail,
                    _ => return Err(CacheError::Syntax),
                };
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(Resp::Array(Some(results)))
}

#[derive(Clone, Copy)]
enum OverflowPolicy {
    Wrap,
    Sat,
    Fail,
}

fn parse_bitfield_type(t: &[u8]) -> CacheResult<(bool, u8)> {
    let s = std::str::from_utf8(t).map_err(|_| CacheError::Syntax)?;
    if s.starts_with('i') || s.starts_with('I') {
        let bits: u8 = s[1..].parse().map_err(|_| CacheError::Syntax)?;
        Ok((true, bits))
    } else if s.starts_with('u') || s.starts_with('U') {
        let bits: u8 = s[1..].parse().map_err(|_| CacheError::Syntax)?;
        Ok((false, bits))
    } else {
        Err(CacheError::Syntax)
    }
}

fn parse_bitfield_offset(o: &[u8]) -> CacheResult<usize> {
    let s = std::str::from_utf8(o).map_err(|_| CacheError::Syntax)?;
    if s.starts_with('#') {
        // Multiply by type width — handled by caller
        s[1..].parse::<usize>().map_err(|_| CacheError::BitOffset)
    } else {
        s.parse::<usize>().map_err(|_| CacheError::BitOffset)
    }
}

fn get_bits(bytes: &[u8], offset: usize, bits: u8) -> u64 {
    let mut val: u64 = 0;
    for i in 0..bits as usize {
        let bit_pos = offset + i;
        let byte_idx = bit_pos / 8;
        let bit_idx = 7 - (bit_pos % 8);
        if byte_idx < bytes.len() {
            val |= (((bytes[byte_idx] >> bit_idx) & 1) as u64) << (bits as usize - 1 - i);
        }
    }
    val
}

fn set_bits(bytes: &mut Vec<u8>, offset: usize, bits: u8, val: u64) {
    let needed = (offset + bits as usize + 7) / 8;
    if bytes.len() < needed {
        bytes.resize(needed, 0);
    }
    for i in 0..bits as usize {
        let bit_pos = offset + i;
        let byte_idx = bit_pos / 8;
        let bit_idx = 7 - (bit_pos % 8);
        let bit = (val >> (bits as usize - 1 - i)) & 1;
        if bit == 1 {
            bytes[byte_idx] |= 1 << bit_idx;
        } else {
            bytes[byte_idx] &= !(1 << bit_idx);
        }
    }
}

fn bitfield_get(
    db: &mut Db,
    key: &[u8],
    is_signed: bool,
    bits: u8,
    offset: usize,
) -> CacheResult<i64> {
    let bytes = match db.get_typed(key, "string")? {
        Some(e) => match &e.value {
            Value::String(v) => v.clone(),
            _ => unreachable!(),
        },
        None => vec![],
    };
    let raw = get_bits(&bytes, offset, bits);
    if is_signed && bits < 64 && (raw >> (bits - 1)) & 1 == 1 {
        let sign_ext = !((1u64 << bits) - 1);
        Ok((raw | sign_ext) as i64)
    } else {
        Ok(raw as i64)
    }
}

fn bitfield_set(
    db: &mut Db,
    key: &[u8],
    is_signed: bool,
    bits: u8,
    offset: usize,
    value: i64,
) -> CacheResult<i64> {
    let old = bitfield_get(db, key, is_signed, bits, offset)?;
    let bytes = match db.get_typed_mut(key, "string")? {
        Some(e) => match &mut e.value {
            Value::String(v) => v,
            _ => unreachable!(),
        },
        None => {
            db.insert(key.to_vec(), Entry::new(Value::String(vec![])));
            match db.get_typed_mut(key, "string")? {
                Some(e) => match &mut e.value {
                    Value::String(v) => v,
                    _ => unreachable!(),
                },
                None => unreachable!(),
            }
        }
    };
    let mask = if bits == 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 1
    };
    set_bits(bytes, offset, bits, (value as u64) & mask);
    Ok(old)
}

fn bitfield_incrby(
    db: &mut Db,
    key: &[u8],
    is_signed: bool,
    bits: u8,
    offset: usize,
    delta: i64,
    overflow: OverflowPolicy,
) -> CacheResult<Option<i64>> {
    let old = bitfield_get(db, key, is_signed, bits, offset)?;
    let new_val = old.wrapping_add(delta);

    let (result, overflowed) = if is_signed {
        let min = if bits == 64 {
            i64::MIN
        } else {
            -(1i64 << (bits - 1))
        };
        let max = if bits == 64 {
            i64::MAX
        } else {
            (1i64 << (bits - 1)) - 1
        };
        if new_val < min || new_val > max {
            match overflow {
                OverflowPolicy::Wrap => (new_val, false), // wrapping already done
                OverflowPolicy::Sat => (if delta > 0 { max } else { min }, false),
                OverflowPolicy::Fail => return Ok(None),
            }
        } else {
            (new_val, false)
        }
    } else {
        let max = if bits == 64 {
            u64::MAX as i64
        } else {
            ((1u64 << bits) - 1) as i64
        };
        if new_val < 0 || new_val > max {
            match overflow {
                OverflowPolicy::Wrap => (new_val & max, false),
                OverflowPolicy::Sat => (if delta > 0 { max } else { 0 }, false),
                OverflowPolicy::Fail => return Ok(None),
            }
        } else {
            (new_val, false)
        }
    };

    bitfield_set(db, key, is_signed, bits, offset, result)?;
    Ok(Some(result))
}
