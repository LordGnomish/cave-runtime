// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! LoRaWAN codec — MAC-layer PHYPayload header parsing + Cayenne LPP
//! payload decoding.
//!
//! Ports the device-facing pieces of a LoRaWAN integration: the MHDR MType
//! decode + uplink FHDR parse (DevAddr, FCnt, FPort — LoRaWAN 1.0.x §4) and
//! the Cayenne Low Power Payload codec used by most LoRa sensors. The radio
//! gateway bridge, join procedure (AES-128 MIC / session-key derivation) and
//! network-server scheduling stay out of scope (see manifest `[[skipped]]`).

use crate::{IotError, KvMap, KvValue, Result};

/// LoRaWAN message type (MHDR top 3 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MType {
    JoinRequest,
    JoinAccept,
    UnconfirmedDataUp,
    UnconfirmedDataDown,
    ConfirmedDataUp,
    ConfirmedDataDown,
    RejoinRequest,
    Proprietary,
}

impl MType {
    fn from_bits(b: u8) -> MType {
        match b & 0x07 {
            0 => MType::JoinRequest,
            1 => MType::JoinAccept,
            2 => MType::UnconfirmedDataUp,
            3 => MType::UnconfirmedDataDown,
            4 => MType::ConfirmedDataUp,
            5 => MType::ConfirmedDataDown,
            6 => MType::RejoinRequest,
            _ => MType::Proprietary,
        }
    }
}

/// Parse an MHDR byte into `(MType, major_version)`.
pub fn parse_mhdr(mhdr: u8) -> (MType, u8) {
    (MType::from_bits(mhdr >> 5), mhdr & 0x03)
}

/// A parsed uplink frame header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UplinkHeader {
    pub mtype: MType,
    /// DevAddr, most-significant-byte first (the wire form is little-endian).
    pub dev_addr: [u8; 4],
    pub fctrl: u8,
    pub fcnt: u16,
    pub fport: Option<u8>,
    pub frm_payload: Vec<u8>,
}

/// Parse a LoRaWAN uplink PHYPayload (without verifying the trailing MIC).
pub fn parse_uplink(phy: &[u8]) -> Result<UplinkHeader> {
    // MHDR(1) + DevAddr(4) + FCtrl(1) + FCnt(2) = 8 byte minimum FHDR.
    if phy.len() < 8 {
        return Err(IotError::Codec("PHYPayload shorter than FHDR".into()));
    }
    let (mtype, _major) = parse_mhdr(phy[0]);
    // DevAddr is little-endian on the wire.
    let dev_addr = [phy[4], phy[3], phy[2], phy[1]];
    let fctrl = phy[5];
    let fopts_len = (fctrl & 0x0F) as usize;
    let fcnt = u16::from_le_bytes([phy[6], phy[7]]);
    let mut pos = 8 + fopts_len;
    // The remaining bytes (minus the 4-byte MIC, if present) are FPort + FRM.
    let body_end = phy.len().saturating_sub(4).max(pos);
    let (fport, frm_payload) = if pos < body_end {
        let fport = phy[pos];
        pos += 1;
        (Some(fport), phy.get(pos..body_end).unwrap_or(&[]).to_vec())
    } else {
        (None, Vec::new())
    };
    Ok(UplinkHeader {
        mtype,
        dev_addr,
        fctrl,
        fcnt,
        fport,
        frm_payload,
    })
}

/// A Cayenne LPP sensor reading kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LppKind {
    DigitalInput,
    DigitalOutput,
    AnalogInput,
    AnalogOutput,
    Illuminance,
    Presence,
    Temperature,
    Humidity,
    Barometer,
    Gps,
}

impl LppKind {
    fn name(&self) -> &'static str {
        match self {
            LppKind::DigitalInput => "digital_input",
            LppKind::DigitalOutput => "digital_output",
            LppKind::AnalogInput => "analog_input",
            LppKind::AnalogOutput => "analog_output",
            LppKind::Illuminance => "illuminance",
            LppKind::Presence => "presence",
            LppKind::Temperature => "temperature",
            LppKind::Humidity => "humidity",
            LppKind::Barometer => "barometer",
            LppKind::Gps => "gps",
        }
    }
}

/// A decoded Cayenne LPP reading: channel + kind + scaled value.
#[derive(Debug, Clone, PartialEq)]
pub struct LppReading {
    pub channel: u8,
    pub kind: LppKind,
    pub value: KvValue,
}

/// `(LppKind, data_byte_len)` for a Cayenne LPP data type byte.
fn lpp_type(t: u8) -> Option<(LppKind, usize)> {
    Some(match t {
        0x00 => (LppKind::DigitalInput, 1),
        0x01 => (LppKind::DigitalOutput, 1),
        0x02 => (LppKind::AnalogInput, 2),
        0x03 => (LppKind::AnalogOutput, 2),
        0x65 => (LppKind::Illuminance, 2),
        0x66 => (LppKind::Presence, 1),
        0x67 => (LppKind::Temperature, 2),
        0x68 => (LppKind::Humidity, 1),
        0x73 => (LppKind::Barometer, 2),
        0x88 => (LppKind::Gps, 9),
        _ => return None,
    })
}

fn s16(hi: u8, lo: u8) -> i32 {
    let raw = ((hi as u16) << 8 | lo as u16) as i16;
    raw as i32
}

fn s24(b: &[u8]) -> i32 {
    let mut v = ((b[0] as i32) << 16) | ((b[1] as i32) << 8) | b[2] as i32;
    if v & 0x80_0000 != 0 {
        v |= -0x100_0000i32; // sign-extend 24→32 bit
    }
    v
}

/// Decode a Cayenne LPP buffer into readings.
pub fn decode_lpp(buf: &[u8]) -> Result<Vec<LppReading>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < buf.len() {
        let channel = buf[i];
        let t = *buf
            .get(i + 1)
            .ok_or_else(|| IotError::Codec("LPP truncated at type byte".into()))?;
        let (kind, len) = lpp_type(t)
            .ok_or_else(|| IotError::Codec(format!("unknown LPP data type 0x{t:02x}")))?;
        let data = buf
            .get(i + 2..i + 2 + len)
            .ok_or_else(|| IotError::Codec("LPP truncated payload".into()))?;
        let value = match kind {
            LppKind::DigitalInput | LppKind::DigitalOutput | LppKind::Presence => {
                KvValue::Long(data[0] as i64)
            }
            LppKind::Illuminance => KvValue::Long(((data[0] as i64) << 8) | data[1] as i64),
            LppKind::Temperature => KvValue::Double(s16(data[0], data[1]) as f64 / 10.0),
            LppKind::Humidity => KvValue::Double(data[0] as f64 / 2.0),
            LppKind::AnalogInput | LppKind::AnalogOutput => {
                KvValue::Double(s16(data[0], data[1]) as f64 / 100.0)
            }
            LppKind::Barometer => {
                KvValue::Double(((data[0] as i64) << 8 | data[1] as i64) as f64 / 10.0)
            }
            LppKind::Gps => {
                let lat = s24(&data[0..3]) as f64 / 10000.0;
                let lon = s24(&data[3..6]) as f64 / 10000.0;
                let alt = s24(&data[6..9]) as f64 / 100.0;
                KvValue::Json(serde_json::json!({"lat": lat, "lon": lon, "alt": alt}))
            }
        };
        out.push(LppReading {
            channel,
            kind,
            value,
        });
        i += 2 + len;
    }
    Ok(out)
}

/// Flatten readings into a KvMap with `{kind}_{channel}` keys.
pub fn lpp_to_kvmap(readings: &[LppReading]) -> KvMap {
    let mut kv = KvMap::new();
    for r in readings {
        kv.insert(format!("{}_{}", r.kind.name(), r.channel), r.value.clone());
    }
    kv
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::KvValue;

    #[test]
    fn mtype_from_mhdr() {
        // MType is the top 3 bits of the MHDR byte.
        assert_eq!(parse_mhdr(0b000_000_00).0, MType::JoinRequest);
        assert_eq!(parse_mhdr(0b010_000_00).0, MType::UnconfirmedDataUp);
        assert_eq!(parse_mhdr(0b100_000_00).0, MType::ConfirmedDataUp);
        // LoRaWAN R1 major version = 0b00.
        assert_eq!(parse_mhdr(0b010_000_00).1, 0);
    }

    #[test]
    fn parses_uplink_header_devaddr_and_fcnt() {
        // PHYPayload: MHDR(0x40 unconf up) | DevAddr LE 0x01020304 | FCtrl 0x00
        //             | FCnt LE 0x0007 | FPort 0x02 | FRMPayload 0xAA | MIC(4)
        let phy = vec![
            0x40, 0x04, 0x03, 0x02, 0x01, 0x00, 0x07, 0x00, 0x02, 0xAA, 0xDE, 0xAD, 0xBE, 0xEF,
        ];
        let h = parse_uplink(&phy).unwrap();
        assert_eq!(h.mtype, MType::UnconfirmedDataUp);
        // DevAddr is stored most-significant-first after LE decode.
        assert_eq!(h.dev_addr, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(h.fcnt, 7);
        assert_eq!(h.fport, Some(2));
        assert_eq!(h.frm_payload, vec![0xAA]);
    }

    #[test]
    fn decodes_cayenne_temperature_and_humidity() {
        // ch3 temp 0x67 0x0110 = 272 → 27.2C ; ch2 humidity 0x68 0x64 = 100 → 50%
        let buf = vec![0x03, 0x67, 0x01, 0x10, 0x02, 0x68, 0x64];
        let readings = decode_lpp(&buf).unwrap();
        assert_eq!(readings.len(), 2);
        assert_eq!(readings[0].channel, 3);
        assert_eq!(readings[0].kind, LppKind::Temperature);
        assert_eq!(readings[0].value, KvValue::Double(27.2));
        assert_eq!(readings[1].kind, LppKind::Humidity);
        assert_eq!(readings[1].value, KvValue::Double(50.0));
    }

    #[test]
    fn decodes_digital_input() {
        let buf = vec![0x01, 0x00, 0x01];
        let r = decode_lpp(&buf).unwrap();
        assert_eq!(r[0].kind, LppKind::DigitalInput);
        assert_eq!(r[0].value, KvValue::Long(1));
    }

    #[test]
    fn lpp_to_kvmap_uses_channel_keyed_names() {
        let buf = vec![0x03, 0x67, 0x01, 0x10];
        let kv = lpp_to_kvmap(&decode_lpp(&buf).unwrap());
        assert_eq!(kv.get("temperature_3"), Some(&KvValue::Double(27.2)));
    }

    #[test]
    fn lpp_rejects_truncated_buffer() {
        // Temperature claims 2 bytes but only 1 follows.
        assert!(decode_lpp(&[0x03, 0x67, 0x01]).is_err());
        // Unknown data type.
        assert!(decode_lpp(&[0x01, 0xFE, 0x00]).is_err());
    }
}
