// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! LoRaWAN codec — MAC-layer PHYPayload header parsing + Cayenne LPP
//! payload decoding. (RED: codec pending.)

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
        //             | FCnt LE 0x0007 | FPort 0x02 | FRMPayload...
        let phy = vec![0x40, 0x04, 0x03, 0x02, 0x01, 0x00, 0x07, 0x00, 0x02, 0xAA];
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
