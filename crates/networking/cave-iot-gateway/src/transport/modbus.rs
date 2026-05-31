// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Modbus codec — application PDU + Modbus/TCP MBAP framing. (RED.)

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_read_holding_registers_request() {
        // FC03, start 0x006B, qty 0x0003
        let pdu = build_read_request(FunctionCode::ReadHoldingRegisters, 0x006B, 3);
        assert_eq!(pdu, vec![0x03, 0x00, 0x6B, 0x00, 0x03]);
    }

    #[test]
    fn parses_register_response() {
        // FC03 | byte count 6 | 3 registers
        let pdu = vec![0x03, 0x06, 0x02, 0x2B, 0x00, 0x00, 0x00, 0x64];
        let regs = parse_register_response(&pdu).unwrap();
        assert_eq!(regs, vec![0x022B, 0x0000, 0x0064]);
    }

    #[test]
    fn parses_exception_response() {
        // FC03 | 0x80 = 0x83, exception code 0x02 (illegal data address)
        let pdu = vec![0x83, 0x02];
        let err = parse_register_response(&pdu).unwrap_err();
        match err {
            crate::IotError::Codec(msg) => assert!(msg.contains("exception 2")),
            other => panic!("expected codec exception, got {other:?}"),
        }
    }

    #[test]
    fn builds_write_single_register() {
        let pdu = build_write_single_register(0x0001, 0x0003);
        assert_eq!(pdu, vec![0x06, 0x00, 0x01, 0x00, 0x03]);
    }

    #[test]
    fn byte_count_mismatch_is_rejected() {
        // byte count says 4 but only 2 data bytes present
        let pdu = vec![0x03, 0x04, 0x00, 0x01];
        assert!(parse_register_response(&pdu).is_err());
    }

    #[test]
    fn mbap_frame_roundtrips_and_sets_length() {
        let pdu = build_read_request(FunctionCode::ReadHoldingRegisters, 0, 2);
        let frame = build_tcp_frame(0x0001, 0x11, &pdu);
        // MBAP length field = unit id (1) + pdu len.
        let length = u16::from_be_bytes([frame[4], frame[5]]);
        assert_eq!(length as usize, 1 + pdu.len());
        let parsed = parse_tcp_frame(&frame).unwrap();
        assert_eq!(parsed.transaction_id, 0x0001);
        assert_eq!(parsed.unit_id, 0x11);
        assert_eq!(parsed.pdu, pdu);
    }

    #[test]
    fn parse_tcp_rejects_nonzero_protocol_id() {
        // protocol id must be 0x0000 for Modbus.
        let bad = vec![0x00, 0x01, 0x00, 0x09, 0x00, 0x02, 0x11, 0x03];
        assert!(parse_tcp_frame(&bad).is_err());
    }
}
