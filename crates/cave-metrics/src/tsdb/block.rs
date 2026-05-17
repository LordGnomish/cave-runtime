// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Gorilla-style XOR float compression for block chunks.
//! Implements the Facebook Gorilla paper delta-of-delta timestamps + XOR values.

/// Encode a sequence of (timestamp_ms, value) pairs using Gorilla compression.
pub struct ChunkWriter {
    data: Vec<u8>,
    bit_pos: usize,
    prev_ts: i64,
    prev_ts_delta: i64,
    prev_val_bits: u64,
    prev_leading: u32,
    prev_trailing: u32,
    count: u32,
}

impl ChunkWriter {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            bit_pos: 0,
            prev_ts: 0,
            prev_ts_delta: 0,
            prev_val_bits: 0,
            prev_leading: 0,
            prev_trailing: 0,
            count: 0,
        }
    }

    /// Write a bit into the buffer.
    fn write_bit(&mut self, bit: bool) {
        if self.bit_pos % 8 == 0 {
            self.data.push(0);
        }
        if bit {
            let byte_idx = self.bit_pos / 8;
            let bit_idx  = 7 - (self.bit_pos % 8);
            self.data[byte_idx] |= 1 << bit_idx;
        }
        self.bit_pos += 1;
    }

    /// Write `n` bits of `value` (MSB first).
    fn write_bits(&mut self, value: u64, n: u32) {
        for i in (0..n).rev() {
            self.write_bit((value >> i) & 1 == 1);
        }
    }

    /// Append a (timestamp_ms, value) sample.
    pub fn append(&mut self, ts: i64, val: f64) {
        if self.count == 0 {
            // First sample: write full 64-bit ts and value.
            self.write_bits(ts as u64, 64);
            self.write_bits(val.to_bits(), 64);
            self.prev_ts = ts;
            self.prev_val_bits = val.to_bits();
        } else {
            // Timestamp: delta-of-delta encoding.
            let delta = ts - self.prev_ts;
            let dod   = delta - self.prev_ts_delta;
            self.encode_dod(dod);
            self.prev_ts_delta = delta;
            self.prev_ts = ts;

            // Value: XOR encoding.
            let xor = val.to_bits() ^ self.prev_val_bits;
            self.encode_xor(xor);
            self.prev_val_bits = val.to_bits();
        }
        self.count += 1;
    }

    fn encode_dod(&mut self, dod: i64) {
        match dod {
            0 => {
                self.write_bit(false);
            }
            -63..=64 => {
                self.write_bits(0b10, 2);
                self.write_bits((dod as u64) & 0x7f, 7);
            }
            -255..=256 => {
                self.write_bits(0b110, 3);
                self.write_bits((dod as u64) & 0x1ff, 9);
            }
            -2047..=2048 => {
                self.write_bits(0b1110, 4);
                self.write_bits((dod as u64) & 0xfff, 12);
            }
            _ => {
                self.write_bits(0b1111, 4);
                self.write_bits(dod as u64, 64);
            }
        }
    }

    fn encode_xor(&mut self, xor: u64) {
        if xor == 0 {
            self.write_bit(false);
            return;
        }
        self.write_bit(true);

        let leading  = xor.leading_zeros();
        let trailing = xor.trailing_zeros();

        // Can we reuse previous leading/trailing?
        if leading >= self.prev_leading && trailing >= self.prev_trailing {
            self.write_bit(false);
            let significant = 64 - self.prev_leading - self.prev_trailing;
            self.write_bits(xor >> self.prev_trailing, significant);
        } else {
            self.write_bit(true);
            self.write_bits(leading as u64, 5);
            let significant = 64 - leading - trailing;
            self.write_bits(significant as u64 - 1, 6);
            self.write_bits(xor >> trailing, significant);
            self.prev_leading  = leading;
            self.prev_trailing = trailing;
        }
    }

    pub fn finish(self) -> (u32, Vec<u8>) {
        (self.count, self.data)
    }
}

impl Default for ChunkWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// Decode a Gorilla-compressed chunk back to (timestamp_ms, value) pairs.
pub struct ChunkReader<'a> {
    data: &'a [u8],
    bit_pos: usize,
    count: u32,
    read: u32,
    prev_ts: i64,
    prev_ts_delta: i64,
    prev_val_bits: u64,
    prev_leading: u32,
    prev_trailing: u32,
}

impl<'a> ChunkReader<'a> {
    pub fn new(count: u32, data: &'a [u8]) -> Self {
        Self {
            data,
            bit_pos: 0,
            count,
            read: 0,
            prev_ts: 0,
            prev_ts_delta: 0,
            prev_val_bits: 0,
            prev_leading: 0,
            prev_trailing: 0,
        }
    }

    fn read_bit(&mut self) -> bool {
        if self.bit_pos / 8 >= self.data.len() { return false; }
        let byte_idx = self.bit_pos / 8;
        let bit_idx  = 7 - (self.bit_pos % 8);
        self.bit_pos += 1;
        (self.data[byte_idx] >> bit_idx) & 1 == 1
    }

    fn read_bits(&mut self, n: u32) -> u64 {
        let mut val: u64 = 0;
        for _ in 0..n {
            val = (val << 1) | (self.read_bit() as u64);
        }
        val
    }

    fn read_dod(&mut self) -> i64 {
        if !self.read_bit() { return 0; }
        if !self.read_bit() {
            let bits = self.read_bits(7);
            return sign_extend(bits, 7);
        }
        if !self.read_bit() {
            let bits = self.read_bits(9);
            return sign_extend(bits, 9);
        }
        if !self.read_bit() {
            let bits = self.read_bits(12);
            return sign_extend(bits, 12);
        }
        self.read_bits(64) as i64
    }

    fn read_xor(&mut self) -> u64 {
        if !self.read_bit() { return 0; }
        if !self.read_bit() {
            // Reuse previous leading/trailing
            let significant = 64 - self.prev_leading - self.prev_trailing;
            let bits = self.read_bits(significant);
            return bits << self.prev_trailing;
        }
        self.prev_leading  = self.read_bits(5) as u32;
        let significant    = self.read_bits(6) as u32 + 1;
        self.prev_trailing = 64 - self.prev_leading - significant;
        let bits = self.read_bits(significant);
        bits << self.prev_trailing
    }

    /// Decode all samples.
    pub fn decode_all(mut self) -> Vec<(i64, f64)> {
        if self.count == 0 { return Vec::new(); }
        let mut out = Vec::with_capacity(self.count as usize);

        // First sample
        let ts  = self.read_bits(64) as i64;
        let val = f64::from_bits(self.read_bits(64));
        self.prev_ts        = ts;
        self.prev_val_bits  = val.to_bits();
        out.push((ts, val));
        self.read += 1;

        while self.read < self.count {
            let dod   = self.read_dod();
            self.prev_ts_delta += dod;
            self.prev_ts       += self.prev_ts_delta;

            let xor = self.read_xor();
            self.prev_val_bits ^= xor;

            out.push((self.prev_ts, f64::from_bits(self.prev_val_bits)));
            self.read += 1;
        }
        out
    }
}

fn sign_extend(val: u64, bits: u32) -> i64 {
    let shift = 64 - bits;
    ((val as i64) << shift) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gorilla_roundtrip() {
        let samples = vec![
            (1_700_000_000_000i64, 1.5f64),
            (1_700_000_015_000,    1.5),
            (1_700_000_030_000,    2.0),
            (1_700_000_045_000,    2.5),
            (1_700_000_060_000,    2.5),
            (1_700_000_075_000,    0.0),
            (1_700_000_090_000,    f64::INFINITY),
        ];

        let mut writer = ChunkWriter::new();
        for (ts, v) in &samples {
            writer.append(*ts, *v);
        }
        let (count, data) = writer.finish();
        assert_eq!(count, samples.len() as u32);

        let decoded = ChunkReader::new(count, &data).decode_all();
        assert_eq!(decoded.len(), samples.len());
        for (i, (ts, v)) in decoded.iter().enumerate() {
            assert_eq!(*ts, samples[i].0);
            // NaN-safe comparison
            if samples[i].1.is_nan() {
                assert!(v.is_nan());
            } else {
                assert_eq!(*v, samples[i].1);
            }
        }
    }
}
