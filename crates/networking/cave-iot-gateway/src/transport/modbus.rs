// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Modbus codec — application PDU + Modbus/TCP MBAP framing.
//!
//! Ports the device-integration codec for Modbus master polling: function-
//! code PDUs (read coils / discrete / holding / input registers, write single
//! register), exception responses (FC|0x80 + exception code) and the Modbus/
//! TCP MBAP header (transaction id, protocol id, length, unit id). No serial
//! line / TCP socket — the runtime data-plane drives the transport.

use crate::{IotError, Result};

/// Modbus function code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionCode {
    ReadCoils,
    ReadDiscreteInputs,
    ReadHoldingRegisters,
    ReadInputRegisters,
    WriteSingleCoil,
    WriteSingleRegister,
    WriteMultipleRegisters,
}

impl FunctionCode {
    pub fn as_u8(&self) -> u8 {
        match self {
            FunctionCode::ReadCoils => 0x01,
            FunctionCode::ReadDiscreteInputs => 0x02,
            FunctionCode::ReadHoldingRegisters => 0x03,
            FunctionCode::ReadInputRegisters => 0x04,
            FunctionCode::WriteSingleCoil => 0x05,
            FunctionCode::WriteSingleRegister => 0x06,
            FunctionCode::WriteMultipleRegisters => 0x10,
        }
    }
}

/// Build a read request PDU: `fc | start_addr(be) | quantity(be)`.
pub fn build_read_request(fc: FunctionCode, start: u16, quantity: u16) -> Vec<u8> {
    let mut pdu = vec![fc.as_u8()];
    pdu.extend_from_slice(&start.to_be_bytes());
    pdu.extend_from_slice(&quantity.to_be_bytes());
    pdu
}

/// Build a write-single-register PDU (FC06): `06 | addr(be) | value(be)`.
pub fn build_write_single_register(addr: u16, value: u16) -> Vec<u8> {
    let mut pdu = vec![FunctionCode::WriteSingleRegister.as_u8()];
    pdu.extend_from_slice(&addr.to_be_bytes());
    pdu.extend_from_slice(&value.to_be_bytes());
    pdu
}

/// Parse a register read response PDU into the big-endian u16 register values.
/// An exception response (function code with the high bit set) becomes an error.
pub fn parse_register_response(pdu: &[u8]) -> Result<Vec<u16>> {
    let fc = *pdu
        .first()
        .ok_or_else(|| IotError::Codec("empty Modbus PDU".into()))?;
    if fc & 0x80 != 0 {
        let code = pdu.get(1).copied().unwrap_or(0);
        return Err(IotError::Codec(format!(
            "Modbus exception {code} for function 0x{:02x}",
            fc & 0x7F
        )));
    }
    let byte_count = *pdu
        .get(1)
        .ok_or_else(|| IotError::Codec("missing byte count".into()))? as usize;
    let data = pdu
        .get(2..)
        .ok_or_else(|| IotError::Codec("missing register data".into()))?;
    if data.len() != byte_count {
        return Err(IotError::Codec(format!(
            "byte count {byte_count} != data length {}",
            data.len()
        )));
    }
    if byte_count % 2 != 0 {
        return Err(IotError::Codec("register byte count must be even".into()));
    }
    Ok(data
        .chunks_exact(2)
        .map(|c| u16::from_be_bytes([c[0], c[1]]))
        .collect())
}

/// A parsed Modbus/TCP frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpFrame {
    pub transaction_id: u16,
    pub unit_id: u8,
    pub pdu: Vec<u8>,
}

/// Build a Modbus/TCP frame: MBAP header (txn, proto=0, length, unit) + PDU.
pub fn build_tcp_frame(transaction_id: u16, unit_id: u8, pdu: &[u8]) -> Vec<u8> {
    let length = (pdu.len() + 1) as u16; // unit id + PDU
    let mut out = Vec::with_capacity(7 + pdu.len());
    out.extend_from_slice(&transaction_id.to_be_bytes());
    out.extend_from_slice(&0u16.to_be_bytes()); // protocol id
    out.extend_from_slice(&length.to_be_bytes());
    out.push(unit_id);
    out.extend_from_slice(pdu);
    out
}

/// Parse a Modbus/TCP frame, validating the protocol id and length field.
pub fn parse_tcp_frame(buf: &[u8]) -> Result<TcpFrame> {
    if buf.len() < 8 {
        return Err(IotError::Codec("MBAP frame shorter than 8 bytes".into()));
    }
    let transaction_id = u16::from_be_bytes([buf[0], buf[1]]);
    let protocol_id = u16::from_be_bytes([buf[2], buf[3]]);
    if protocol_id != 0 {
        return Err(IotError::Codec(format!(
            "non-Modbus protocol id 0x{protocol_id:04x}"
        )));
    }
    let length = u16::from_be_bytes([buf[4], buf[5]]) as usize;
    let unit_id = buf[6];
    let pdu = buf
        .get(7..6 + length)
        .ok_or_else(|| IotError::Codec("MBAP length exceeds frame".into()))?
        .to_vec();
    Ok(TcpFrame {
        transaction_id,
        unit_id,
        pdu,
    })
}

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
