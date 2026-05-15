// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: FerretDB/FerretDB@27653503a28b0d2f2a09d7105887edd3a8af1060 internal/wire/wire.go
//! Wire-protocol facade.
//!
//! Re-exports the existing `wire` + `bson` + `commands` surfaces and
//! adds a FerretDB-ported `hello` (a.k.a. `isMaster`) reply builder
//! that returns the canonical fields drivers consume during the
//! initial handshake.

pub use crate::bson::Document;
pub use crate::wire::{decode_op_msg, encode_op_msg, OpMsg, Section, WireError};

pub mod hello_reply;

pub use hello_reply::{hello_reply, HelloReply};

/// OP_MSG opcode (publicly documented; spec value 2013).
pub const OP_MSG_OPCODE: i32 = 2013;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn re_exports_resolve() {
        let mut d = Document::new();
        d.insert("ok".to_string(), serde_json::Value::Number(1.into()));
        let m = OpMsg::new(d);
        let enc = encode_op_msg(&m, 1, 0).unwrap();
        let _ = decode_op_msg(&enc).unwrap();
    }

    #[test]
    fn opcode_constant_value_is_spec_documented() {
        assert_eq!(OP_MSG_OPCODE, 2013);
    }
}
