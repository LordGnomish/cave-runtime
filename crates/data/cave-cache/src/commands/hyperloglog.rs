// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! HyperLogLog commands: PFADD, PFCOUNT, PFMERGE.
//!
//! Implements the HyperLogLog++ algorithm compatible with Redis.
//! HLL data is stored as a String value with a 4-byte "HYLL" magic header.

use crate::db::Db;
use crate::error::{CacheError, CacheResult};
use crate::resp::Resp;
use crate::types::{Entry, Value};

// HLL parameters
const HLL_P: usize = 14;
const HLL_M: usize = 1 << HLL_P; // 16384 registers
const HLL_HEADER_SIZE: usize = 16;
const HLL_MAGIC: &[u8] = b"HYLL";
const HLL_SPARSE_XZERO_MAX_LEN: usize = 16384;

pub fn cmd_pfadd(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("pfadd"));
    }
    let key = args[1].clone();

    // Get or create HLL
    let mut hll = match db.get_typed(&key, "string")? {
        Some(e) => match &e.value {
            Value::String(v) => {
                if v.starts_with(HLL_MAGIC) {
                    HyperLogLog::from_bytes(v)?
                } else {
                    return Err(CacheError::generic(
                        "WRONGTYPE Key is not a valid HyperLogLog string value.",
                    ));
                }
            }
            _ => unreachable!(),
        },
        None => HyperLogLog::new(),
    };

    let mut changed = false;
    for element in &args[2..] {
        if hll.add(element) {
            changed = true;
        }
    }

    let new_bytes = hll.to_bytes();
    db.insert(key, Entry::new(Value::String(new_bytes)));

    Ok(Resp::Integer(if changed { 1 } else { 0 }))
}

pub fn cmd_pfcount(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("pfcount"));
    }

    if args.len() == 2 {
        // Single key
        let count = match db.get_typed(&args[1], "string")? {
            Some(e) => match &e.value {
                Value::String(v) => {
                    if v.starts_with(HLL_MAGIC) {
                        HyperLogLog::from_bytes(v)?.count()
                    } else {
                        return Err(CacheError::generic(
                            "WRONGTYPE Key is not a valid HyperLogLog string value.",
                        ));
                    }
                }
                _ => unreachable!(),
            },
            None => 0,
        };
        Ok(Resp::Integer(count as i64))
    } else {
        // Merge multiple keys and count
        let mut merged = HyperLogLog::new();
        for key in &args[1..] {
            match db.get_typed(key, "string")? {
                Some(e) => match &e.value {
                    Value::String(v) => {
                        if v.starts_with(HLL_MAGIC) {
                            let hll = HyperLogLog::from_bytes(v)?;
                            merged.merge(&hll);
                        } else {
                            return Err(CacheError::generic(
                                "WRONGTYPE Key is not a valid HyperLogLog string value.",
                            ));
                        }
                    }
                    _ => unreachable!(),
                },
                None => {}
            }
        }
        Ok(Resp::Integer(merged.count() as i64))
    }
}

pub fn cmd_pfmerge(args: &[Vec<u8>], db: &mut Db) -> CacheResult<Resp> {
    if args.len() < 2 {
        return Err(CacheError::wrong_arity("pfmerge"));
    }
    let dst = args[1].clone();

    let mut merged = HyperLogLog::new();

    // Include destination if it exists
    if let Some(e) = db.get_typed(&dst, "string")? {
        if let Value::String(v) = &e.value {
            if v.starts_with(HLL_MAGIC) {
                merged.merge(&HyperLogLog::from_bytes(v)?);
            }
        }
    }

    for key in &args[2..] {
        match db.get_typed(key, "string")? {
            Some(e) => match &e.value {
                Value::String(v) => {
                    if v.starts_with(HLL_MAGIC) {
                        merged.merge(&HyperLogLog::from_bytes(v)?);
                    } else {
                        return Err(CacheError::generic(
                            "WRONGTYPE Key is not a valid HyperLogLog string value.",
                        ));
                    }
                }
                _ => unreachable!(),
            },
            None => {}
        }
    }

    db.insert(dst, Entry::new(Value::String(merged.to_bytes())));
    Ok(Resp::ok())
}

// ── HyperLogLog implementation ────────────────────────────────────────────────

struct HyperLogLog {
    registers: Vec<u8>, // HLL_M registers, each storing max leading zeros
}

impl HyperLogLog {
    fn new() -> Self {
        HyperLogLog {
            registers: vec![0u8; HLL_M],
        }
    }

    fn add(&mut self, element: &[u8]) -> bool {
        let hash = murmur3_64(element, 0xadc83b19);
        let index = (hash & ((HLL_M as u64) - 1)) as usize;
        let w = hash >> HLL_P;
        let leading = count_leading_zeros(w, 64 - HLL_P) as u8 + 1;

        if leading > self.registers[index] {
            self.registers[index] = leading;
            true
        } else {
            false
        }
    }

    fn count(&self) -> u64 {
        // HyperLogLog++ estimator
        let m = HLL_M as f64;
        let alpha = 0.7213 / (1.0 + 1.079 / m);

        let sum: f64 = self
            .registers
            .iter()
            .map(|&r| 2.0f64.powi(-(r as i32)))
            .sum();

        let estimate = alpha * m * m / sum;

        // Small range correction
        if estimate <= 2.5 * m {
            let zeros = self.registers.iter().filter(|&&r| r == 0).count() as f64;
            if zeros > 0.0 {
                return (m * (m / zeros).ln()) as u64;
            }
        }

        // Large range correction
        if estimate > (1.0 / 30.0) * (2.0f64.powi(32)) {
            let corrected = -(2.0f64.powi(32)) * (1.0 - estimate / 2.0f64.powi(32)).ln();
            return corrected as u64;
        }

        estimate as u64
    }

    fn merge(&mut self, other: &HyperLogLog) {
        for (a, b) in self.registers.iter_mut().zip(other.registers.iter()) {
            *a = (*a).max(*b);
        }
    }

    fn to_bytes(&self) -> Vec<u8> {
        // Redis HLL dense representation
        // Header: HYLL (4) + ver (1) + notused (3) + card (8) = 16 bytes
        // Then packed 6-bit registers: HLL_M registers * 6 bits = 12288 bytes
        let mut buf = Vec::with_capacity(HLL_HEADER_SIZE + HLL_M * 6 / 8);
        buf.extend_from_slice(HLL_MAGIC);
        buf.push(1); // version (dense)
        buf.extend_from_slice(&[0u8; 3]); // notused
        // Cached cardinality (8 bytes, 0 = invalid)
        buf.extend_from_slice(&[0u8; 8]);

        // Pack 6-bit registers
        let mut packed = vec![0u8; HLL_M * 6 / 8 + 1];
        for (i, &reg) in self.registers.iter().enumerate() {
            let val = (reg & 0x3F) as usize;
            let bit_pos = i * 6;
            let byte_pos = bit_pos / 8;
            let bit_offset = bit_pos % 8;
            packed[byte_pos] |= (val << bit_offset) as u8;
            if bit_offset > 2 {
                packed[byte_pos + 1] |= (val >> (8 - bit_offset)) as u8;
            }
        }
        buf.extend_from_slice(&packed);
        buf
    }

    fn from_bytes(data: &[u8]) -> CacheResult<Self> {
        if data.len() < HLL_HEADER_SIZE + 4 {
            return Err(CacheError::generic(
                "WRONGTYPE Key is not a valid HyperLogLog string value.",
            ));
        }
        if !data.starts_with(HLL_MAGIC) {
            return Err(CacheError::generic(
                "WRONGTYPE Key is not a valid HyperLogLog string value.",
            ));
        }

        let ver = data[4];
        if ver != 0 && ver != 1 {
            // Unknown version — return empty
            return Ok(HyperLogLog::new());
        }

        // Unpack 6-bit registers
        let packed = &data[HLL_HEADER_SIZE..];
        let mut registers = vec![0u8; HLL_M];
        for i in 0..HLL_M {
            let bit_pos = i * 6;
            let byte_pos = bit_pos / 8;
            let bit_offset = bit_pos % 8;
            if byte_pos >= packed.len() {
                break;
            }
            let mut val = (packed[byte_pos] >> bit_offset) as usize;
            if bit_offset > 2 && byte_pos + 1 < packed.len() {
                val |= (packed[byte_pos + 1] as usize) << (8 - bit_offset);
            }
            registers[i] = (val & 0x3F) as u8;
        }
        Ok(HyperLogLog { registers })
    }
}

// ── Hash functions ────────────────────────────────────────────────────────────

fn murmur3_64(data: &[u8], seed: u64) -> u64 {
    let mut h1: u64 = seed;
    let mut h2: u64 = seed;
    let c1: u64 = 0x87c37b91114253d5;
    let c2: u64 = 0x4cf5ad432745937f;

    let blocks = data.len() / 16;
    for i in 0..blocks {
        let k1 = u64::from_le_bytes(data[i * 16..i * 16 + 8].try_into().unwrap_or([0u8; 8]));
        let k2 = u64::from_le_bytes(data[i * 16 + 8..i * 16 + 16].try_into().unwrap_or([0u8; 8]));

        let k1 = k1.wrapping_mul(c1).rotate_left(31).wrapping_mul(c2);
        h1 ^= k1;
        h1 = h1
            .rotate_left(27)
            .wrapping_add(h2)
            .wrapping_mul(5)
            .wrapping_add(0x52dce729);

        let k2 = k2.wrapping_mul(c2).rotate_left(33).wrapping_mul(c1);
        h2 ^= k2;
        h2 = h2
            .rotate_left(31)
            .wrapping_add(h1)
            .wrapping_mul(5)
            .wrapping_add(0x38495ab5);
    }

    let tail = &data[blocks * 16..];
    let k1: u64 = 0;
    let mut k2: u64 = 0;

    if tail.len() >= 8 {
        k2 ^= (tail[7] as u64) << 56;
    }
    if tail.len() >= 7 {
        k2 ^= (tail[6] as u64) << 48;
    }
    if tail.len() >= 6 {
        k2 ^= (tail[5] as u64) << 40;
    }
    if tail.len() >= 5 {
        k2 ^= (tail[4] as u64) << 32;
    }
    if tail.len() >= 4 {
        k2 ^= (tail[3] as u64) << 24;
    }
    if tail.len() >= 3 {
        k2 ^= (tail[2] as u64) << 16;
    }
    if tail.len() >= 2 {
        k2 ^= (tail[1] as u64) << 8;
    }
    if !tail.is_empty() {
        k2 ^= tail[0] as u64;
    }

    k2 = k2.wrapping_mul(c2).rotate_left(33).wrapping_mul(c1);
    h2 ^= k2;

    h1 ^= data.len() as u64;
    h2 ^= data.len() as u64;
    h1 = h1.wrapping_add(h2);
    h2 = h2.wrapping_add(h1);
    h1 = fmix64(h1);
    h2 = fmix64(h2);
    h1.wrapping_add(h2)
}

fn fmix64(mut k: u64) -> u64 {
    k ^= k >> 33;
    k = k.wrapping_mul(0xff51afd7ed558ccd);
    k ^= k >> 33;
    k = k.wrapping_mul(0xc4ceb9fe1a85ec53);
    k ^= k >> 33;
    k
}

fn count_leading_zeros(value: u64, bits: usize) -> usize {
    if value == 0 {
        return bits;
    }
    let leading = value.leading_zeros() as usize;
    // Adjust for the effective bit width
    if leading >= (64 - bits) {
        bits
    } else {
        leading + bits - 64
    }
}
