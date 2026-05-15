// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: keycloak/keycloak@b825ba97 federation/ldap (uses JNDI; we reimplement
//   the LDAP message layer in Rust because no JNDI exists in std)
//
// Minimal BER/DER codec for RFC 4511 LDAPv3 messages.  Only the
// subset used by bind / search / sync is implemented.  Encoder
// is DER (canonical) — receivers (OpenLDAP, AD) all accept it.
// Decoder is BER-lenient on length forms but rejects indefinite
// length (LDAP forbids it per RFC 4511 §5.1).
//
// Tag conventions used here:
//   Universal:           class=0
//   Application:         class=1   (LDAPMessage component types)
//   Context-specific:    class=2   (e.g. SearchResultEntry attrs)
//   Primitive vs Constructed encoded in the high bit of the tag byte.

use std::io::{self, Read};

/// ASN.1 class bits (top 2 bits of the identifier octet).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Universal = 0b00,
    Application = 0b01,
    Context = 0b10,
    Private = 0b11,
}

impl Class {
    fn from_bits(b: u8) -> Self {
        match (b >> 6) & 0b11 {
            0 => Class::Universal,
            1 => Class::Application,
            2 => Class::Context,
            _ => Class::Private,
        }
    }
}

/// Form: primitive (0) or constructed (1) — bit 6.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Form {
    Primitive,
    Constructed,
}

impl Form {
    fn from_bits(b: u8) -> Self {
        if (b >> 5) & 0b1 == 0 { Form::Primitive } else { Form::Constructed }
    }
}

/// A parsed BER tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tag {
    pub class: Class,
    pub form: Form,
    pub number: u32,
}

impl Tag {
    pub const fn new(class: Class, form: Form, number: u32) -> Self {
        Self { class, form, number }
    }

    pub const fn universal(number: u32, form: Form) -> Self {
        Self::new(Class::Universal, form, number)
    }

    pub const fn application(number: u32, form: Form) -> Self {
        Self::new(Class::Application, form, number)
    }

    pub const fn context(number: u32, form: Form) -> Self {
        Self::new(Class::Context, form, number)
    }

    fn encode_into(self, out: &mut Vec<u8>) {
        let class_bits = match self.class {
            Class::Universal => 0b00,
            Class::Application => 0b01,
            Class::Context => 0b10,
            Class::Private => 0b11,
        } << 6;
        let form_bit = match self.form {
            Form::Primitive => 0,
            Form::Constructed => 0b0010_0000,
        };
        if self.number < 31 {
            out.push(class_bits | form_bit | (self.number as u8));
        } else {
            out.push(class_bits | form_bit | 0b0001_1111);
            // High-tag-number form: base-128 big-endian, last byte
            // has MSB clear.
            let mut buf = [0u8; 6];
            let mut n = self.number;
            let mut idx = buf.len();
            loop {
                idx -= 1;
                buf[idx] = (n & 0x7f) as u8;
                n >>= 7;
                if n == 0 {
                    break;
                }
            }
            for i in idx..buf.len() - 1 {
                out.push(buf[i] | 0x80);
            }
            out.push(buf[buf.len() - 1]);
        }
    }
}

fn encode_length_into(len: usize, out: &mut Vec<u8>) {
    if len < 128 {
        out.push(len as u8);
    } else {
        let mut buf = [0u8; 8];
        let mut n = len;
        let mut idx = buf.len();
        while n > 0 {
            idx -= 1;
            buf[idx] = (n & 0xff) as u8;
            n >>= 8;
        }
        let nbytes = buf.len() - idx;
        out.push(0x80 | nbytes as u8);
        out.extend_from_slice(&buf[idx..]);
    }
}

/// Length-delimited element ready to write.  Owns its payload so
/// callers can build trees without lifetime juggling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Element {
    pub tag: Tag,
    pub bytes: Vec<u8>,
}

impl Element {
    pub fn new(tag: Tag, bytes: Vec<u8>) -> Self {
        Self { tag, bytes }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.bytes.len() + 6);
        self.tag.encode_into(&mut out);
        encode_length_into(self.bytes.len(), &mut out);
        out.extend_from_slice(&self.bytes);
        out
    }
}

// ── Primitive encoders ──────────────────────────────────────────────

pub fn integer(n: i64) -> Element {
    // Minimal two's-complement encoding.
    let mut buf = n.to_be_bytes().to_vec();
    while buf.len() > 1 {
        let first = buf[0];
        let next = buf[1];
        // Drop a redundant leading byte if it can be deduced from
        // the next byte's MSB.
        if (first == 0x00 && next & 0x80 == 0) || (first == 0xff && next & 0x80 != 0) {
            buf.remove(0);
        } else {
            break;
        }
    }
    Element::new(Tag::universal(2, Form::Primitive), buf)
}

pub fn octet_string(b: &[u8]) -> Element {
    Element::new(Tag::universal(4, Form::Primitive), b.to_vec())
}

pub fn boolean(v: bool) -> Element {
    Element::new(
        Tag::universal(1, Form::Primitive),
        vec![if v { 0xff } else { 0x00 }],
    )
}

pub fn enumerated(n: i64) -> Element {
    let mut e = integer(n);
    e.tag = Tag::universal(10, Form::Primitive);
    e
}

pub fn sequence(children: &[Element]) -> Element {
    let mut buf = Vec::new();
    for c in children {
        buf.extend_from_slice(&c.encode());
    }
    Element::new(Tag::universal(16, Form::Constructed), buf)
}

pub fn set(children: &[Element]) -> Element {
    let mut buf = Vec::new();
    for c in children {
        buf.extend_from_slice(&c.encode());
    }
    Element::new(Tag::universal(17, Form::Constructed), buf)
}

pub fn null() -> Element {
    Element::new(Tag::universal(5, Form::Primitive), Vec::new())
}

// ── Decoder ─────────────────────────────────────────────────────────

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DecodeError {
    #[error("unexpected end of buffer")]
    Eof,
    #[error("indefinite length forbidden by RFC 4511")]
    IndefiniteLength,
    #[error("length overflow")]
    LengthOverflow,
    #[error("tag-number overflow")]
    TagOverflow,
    #[error("expected tag {expected:?} got {actual:?}")]
    UnexpectedTag { expected: Tag, actual: Tag },
    #[error("trailing bytes after element")]
    Trailing,
    #[error("invalid utf-8 in OCTET STRING")]
    Utf8,
}

/// Cursor-style decoder.
pub struct Decoder<'a> {
    buf: &'a [u8],
    pub pos: usize,
}

impl<'a> Decoder<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    pub fn remaining(&self) -> &'a [u8] {
        &self.buf[self.pos..]
    }

    pub fn eof(&self) -> bool {
        self.pos >= self.buf.len()
    }

    fn read_u8(&mut self) -> Result<u8, DecodeError> {
        let b = *self.buf.get(self.pos).ok_or(DecodeError::Eof)?;
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], DecodeError> {
        if self.pos + n > self.buf.len() {
            return Err(DecodeError::Eof);
        }
        let r = &self.buf[self.pos..self.pos + n];
        self.pos += n;
        Ok(r)
    }

    /// Peek the tag without consuming it.
    pub fn peek_tag(&self) -> Result<Tag, DecodeError> {
        let mut clone = Decoder { buf: self.buf, pos: self.pos };
        clone.read_tag()
    }

    pub fn read_tag(&mut self) -> Result<Tag, DecodeError> {
        let first = self.read_u8()?;
        let class = Class::from_bits(first);
        let form = Form::from_bits(first);
        let low = first & 0b0001_1111;
        let number = if low < 31 {
            low as u32
        } else {
            let mut n: u32 = 0;
            loop {
                let b = self.read_u8()?;
                n = n.checked_shl(7).ok_or(DecodeError::TagOverflow)?;
                n |= (b & 0x7f) as u32;
                if b & 0x80 == 0 {
                    break;
                }
            }
            n
        };
        Ok(Tag { class, form, number })
    }

    pub fn read_length(&mut self) -> Result<usize, DecodeError> {
        let first = self.read_u8()?;
        if first < 0x80 {
            return Ok(first as usize);
        }
        if first == 0x80 {
            return Err(DecodeError::IndefiniteLength);
        }
        let nbytes = (first & 0x7f) as usize;
        if nbytes > 8 {
            return Err(DecodeError::LengthOverflow);
        }
        let mut n: u64 = 0;
        for _ in 0..nbytes {
            n = (n << 8) | self.read_u8()? as u64;
        }
        if n > usize::MAX as u64 {
            return Err(DecodeError::LengthOverflow);
        }
        Ok(n as usize)
    }

    /// Read a TLV; return tag + payload slice.
    pub fn read_tlv(&mut self) -> Result<(Tag, &'a [u8]), DecodeError> {
        let tag = self.read_tag()?;
        let len = self.read_length()?;
        let payload = self.read_bytes(len)?;
        Ok((tag, payload))
    }

    pub fn read_expected(&mut self, expected: Tag) -> Result<&'a [u8], DecodeError> {
        let (tag, payload) = self.read_tlv()?;
        if tag != expected {
            return Err(DecodeError::UnexpectedTag { expected, actual: tag });
        }
        Ok(payload)
    }

    pub fn read_integer(&mut self) -> Result<i64, DecodeError> {
        let payload = self.read_expected(Tag::universal(2, Form::Primitive))?;
        Ok(parse_integer_payload(payload))
    }

    pub fn read_enumerated(&mut self) -> Result<i64, DecodeError> {
        let payload = self.read_expected(Tag::universal(10, Form::Primitive))?;
        Ok(parse_integer_payload(payload))
    }

    pub fn read_octet_string(&mut self) -> Result<&'a [u8], DecodeError> {
        let payload = self.read_expected(Tag::universal(4, Form::Primitive))?;
        Ok(payload)
    }

    pub fn read_octet_string_utf8(&mut self) -> Result<String, DecodeError> {
        let bytes = self.read_octet_string()?;
        std::str::from_utf8(bytes).map(|s| s.to_string()).map_err(|_| DecodeError::Utf8)
    }
}

fn parse_integer_payload(bytes: &[u8]) -> i64 {
    if bytes.is_empty() {
        return 0;
    }
    let mut n: i64 = if bytes[0] & 0x80 != 0 { -1 } else { 0 };
    for &b in bytes {
        n = (n << 8) | b as i64;
    }
    n
}

/// Read one LDAPMessage frame from a synchronous transport.  Returns
/// the whole TLV including the tag/length header so the caller can
/// hand it back to `Decoder::new` for nested parsing.
pub fn read_frame<R: Read>(r: &mut R) -> io::Result<Vec<u8>> {
    let mut hdr = [0u8; 1];
    r.read_exact(&mut hdr)?;
    let mut frame = vec![hdr[0]];

    // Read length prefix.
    let mut len_buf = [0u8; 1];
    r.read_exact(&mut len_buf)?;
    frame.push(len_buf[0]);
    let len = if len_buf[0] < 0x80 {
        len_buf[0] as usize
    } else if len_buf[0] == 0x80 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "indefinite length"));
    } else {
        let nbytes = (len_buf[0] & 0x7f) as usize;
        if nbytes > 8 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "length overflow"));
        }
        let mut tmp = vec![0u8; nbytes];
        r.read_exact(&mut tmp)?;
        frame.extend_from_slice(&tmp);
        let mut n: usize = 0;
        for b in &tmp {
            n = (n << 8) | *b as usize;
        }
        n
    };
    let mut payload = vec![0u8; len];
    r.read_exact(&mut payload)?;
    frame.extend_from_slice(&payload);
    Ok(frame)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integer_encodes_zero_as_single_byte() {
        let bytes = integer(0).encode();
        // Tag 0x02, length 0x01, payload 0x00.
        assert_eq!(bytes, vec![0x02, 0x01, 0x00]);
    }

    #[test]
    fn integer_encodes_positive_minimally() {
        // 127 fits in one byte with no leading sign.
        let bytes = integer(127).encode();
        assert_eq!(bytes, vec![0x02, 0x01, 0x7f]);
        // 128 needs a leading zero so the MSB-set byte stays positive.
        let bytes = integer(128).encode();
        assert_eq!(bytes, vec![0x02, 0x02, 0x00, 0x80]);
    }

    #[test]
    fn integer_encodes_negative_two_complement() {
        let bytes = integer(-1).encode();
        assert_eq!(bytes, vec![0x02, 0x01, 0xff]);
        let bytes = integer(-128).encode();
        assert_eq!(bytes, vec![0x02, 0x01, 0x80]);
    }

    #[test]
    fn octet_string_round_trip() {
        let e = octet_string(b"cn=admin");
        let bytes = e.encode();
        let mut d = Decoder::new(&bytes);
        assert_eq!(d.read_octet_string().unwrap(), b"cn=admin");
        assert!(d.eof());
    }

    #[test]
    fn sequence_nests_children_with_constructed_form() {
        let s = sequence(&[integer(1), octet_string(b"foo")]);
        let bytes = s.encode();
        assert_eq!(bytes[0] & 0b0010_0000, 0b0010_0000, "constructed bit set");
        let mut d = Decoder::new(&bytes);
        let (tag, payload) = d.read_tlv().unwrap();
        assert_eq!(tag, Tag::universal(16, Form::Constructed));
        let mut inner = Decoder::new(payload);
        assert_eq!(inner.read_integer().unwrap(), 1);
        assert_eq!(inner.read_octet_string().unwrap(), b"foo");
    }

    #[test]
    fn long_form_length_round_trip() {
        let payload = vec![0u8; 300];
        let e = octet_string(&payload);
        let bytes = e.encode();
        // First length byte signals 2 follow.
        assert_eq!(bytes[1], 0x82);
        assert_eq!(bytes[2], 0x01);
        assert_eq!(bytes[3], 0x2c);
        let mut d = Decoder::new(&bytes);
        assert_eq!(d.read_octet_string().unwrap().len(), 300);
    }

    #[test]
    fn application_tag_encodes_class_bits() {
        let bytes = Element::new(Tag::application(0, Form::Constructed), Vec::new()).encode();
        // class=01, form=1, number=0 → 0b0110_0000 = 0x60.
        assert_eq!(bytes[0], 0x60);
    }

    #[test]
    fn context_tag_encodes_class_bits() {
        let bytes = Element::new(Tag::context(2, Form::Primitive), vec![1]).encode();
        // class=10, form=0, number=2 → 0b1000_0010 = 0x82.
        assert_eq!(bytes[0], 0x82);
    }

    #[test]
    fn decoder_rejects_indefinite_length() {
        let buf = [0x30, 0x80, 0x00, 0x00];
        let mut d = Decoder::new(&buf);
        let _ = d.read_tag();
        assert_eq!(d.read_length().unwrap_err(), DecodeError::IndefiniteLength);
    }

    #[test]
    fn high_tag_number_round_trip() {
        let bytes = Element::new(Tag::context(31, Form::Primitive), vec![0xaa]).encode();
        let mut d = Decoder::new(&bytes);
        let tag = d.read_tag().unwrap();
        assert_eq!(tag, Tag::context(31, Form::Primitive));
    }

    #[test]
    fn read_frame_returns_whole_tlv() {
        // Build a SEQUENCE { INTEGER 1, OCTET STRING "hi" }.
        let frame = sequence(&[integer(1), octet_string(b"hi")]).encode();
        let mut cursor = std::io::Cursor::new(frame.clone());
        let got = read_frame(&mut cursor).unwrap();
        assert_eq!(got, frame);
    }

    #[test]
    fn boolean_encodes_to_ff_or_00() {
        assert_eq!(boolean(true).encode(), vec![0x01, 0x01, 0xff]);
        assert_eq!(boolean(false).encode(), vec![0x01, 0x01, 0x00]);
    }

    #[test]
    fn enumerated_uses_tag_10() {
        let bytes = enumerated(49).encode();
        assert_eq!(bytes[0], 0x0a);
    }
}
