// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! CoAP transport codec (RFC 7252 message format). (RED: codec pending.)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn type_byte_roundtrip() {
        for t in [
            CoapType::Confirmable,
            CoapType::NonConfirmable,
            CoapType::Acknowledgement,
            CoapType::Reset,
        ] {
            assert_eq!(CoapType::from_bits(t as u8), Some(t));
        }
    }

    #[test]
    fn code_class_detail() {
        // GET = 0.01, POST = 0.02, 2.05 Content, 4.04 Not Found.
        assert_eq!(CoapCode::new(0, 1).as_u8(), 0x01);
        assert_eq!(CoapCode::new(0, 2).as_u8(), 0x02);
        assert_eq!(CoapCode::new(2, 5).as_u8(), 0x45);
        let nf = CoapCode::from_u8(0x84);
        assert_eq!(nf.class(), 4);
        assert_eq!(nf.detail(), 4);
    }

    #[test]
    fn message_roundtrip_with_token_and_payload() {
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::Confirmable,
            code: CoapCode::new(0, 2), // POST
            message_id: 0xBEEF,
            token: vec![0x11, 0x22],
            options: vec![],
            payload: b"{\"t\":21}".to_vec(),
        };
        let bytes = msg.encode();
        let back = CoapMessage::decode(&bytes).unwrap();
        assert_eq!(back, msg);
    }

    #[test]
    fn message_roundtrip_with_options() {
        // Uri-Path (11) "telemetry", Content-Format (12) [0x32]
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::NonConfirmable,
            code: CoapCode::new(0, 2),
            message_id: 1,
            token: vec![],
            options: vec![
                CoapOption { number: 11, value: b"telemetry".to_vec() },
                CoapOption { number: 12, value: vec![0x32] },
            ],
            payload: vec![],
        };
        let back = CoapMessage::decode(&msg.encode()).unwrap();
        assert_eq!(back.options.len(), 2);
        assert_eq!(back.options[0].number, 11);
        assert_eq!(back.options[0].value, b"telemetry");
        assert_eq!(back.options[1].number, 12);
    }

    #[test]
    fn option_with_large_delta_uses_extended_nibble() {
        // Option number 270 forces the 2-byte extended delta (>268).
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::Confirmable,
            code: CoapCode::new(0, 1),
            message_id: 5,
            token: vec![],
            options: vec![CoapOption { number: 270, value: b"x".to_vec() }],
            payload: vec![],
        };
        let back = CoapMessage::decode(&msg.encode()).unwrap();
        assert_eq!(back.options[0].number, 270);
    }

    #[test]
    fn decode_rejects_bad_version_and_truncation() {
        // version 2 (top two bits = 10) is invalid.
        let bad = vec![0x80, 0x01, 0x00, 0x00];
        assert!(CoapMessage::decode(&bad).is_err());
        assert!(CoapMessage::decode(&[]).is_err());
    }
}
