// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! MQTT transport codec — ThingsBoard device-API topic grammar +
//! MQTT 3.1.1 fixed-header / PUBLISH framing. (RED: codec fns pending.)

use crate::{KvMap, KvValue, Result};

/// A parsed ThingsBoard device-API topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTopic {
    Telemetry,
    PostAttributes,
    AttributesRequest(i32),
    AttributesResponse(i32),
    RpcRequest(i32),
    RpcResponse(i32),
}

/// MQTT control-packet type (high nibble of byte 1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketType {
    Connect,
    ConnAck,
    Publish,
    PubAck,
    Subscribe,
    SubAck,
    PingReq,
    PingResp,
    Disconnect,
}

/// A decoded PUBLISH packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    pub payload: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_device_topics() {
        assert_eq!(DeviceTopic::parse("v1/devices/me/telemetry").unwrap(), DeviceTopic::Telemetry);
        assert_eq!(
            DeviceTopic::parse("v1/devices/me/attributes").unwrap(),
            DeviceTopic::PostAttributes
        );
        assert_eq!(
            DeviceTopic::parse("v1/devices/me/rpc/request/42").unwrap(),
            DeviceTopic::RpcRequest(42)
        );
        assert_eq!(
            DeviceTopic::parse("v1/devices/me/attributes/response/7").unwrap(),
            DeviceTopic::AttributesResponse(7)
        );
    }

    #[test]
    fn rejects_unknown_topic_and_bad_id() {
        assert!(DeviceTopic::parse("v2/whatever").is_err());
        assert!(DeviceTopic::parse("v1/devices/me/rpc/request/notanint").is_err());
    }

    #[test]
    fn remaining_length_varint_roundtrips() {
        for len in [0usize, 127, 128, 16383, 16384, 2097151] {
            let enc = encode_remaining_length(len);
            let (dec, n) = decode_remaining_length(&enc).unwrap();
            assert_eq!(dec, len);
            assert_eq!(n, enc.len());
        }
        assert_eq!(encode_remaining_length(64), vec![0x40]);
        assert_eq!(encode_remaining_length(321), vec![0xC1, 0x02]);
    }

    #[test]
    fn publish_encode_decode_roundtrip_qos0() {
        let pkt = encode_publish("v1/devices/me/telemetry", b"{\"t\":21}");
        assert_eq!(packet_type(pkt[0]).unwrap(), PacketType::Publish);
        let p = decode_publish(&pkt).unwrap();
        assert_eq!(p.topic, "v1/devices/me/telemetry");
        assert_eq!(p.qos, 0);
        assert_eq!(p.payload, b"{\"t\":21}");
    }

    #[test]
    fn decode_publish_with_qos1_skips_packet_id() {
        let mut var = vec![0x00, 0x01, b'a', 0x00, 0x05];
        var.extend_from_slice(b"hi");
        let mut pkt = vec![0x32];
        pkt.extend_from_slice(&encode_remaining_length(var.len()));
        pkt.extend_from_slice(&var);
        let p = decode_publish(&pkt).unwrap();
        assert_eq!(p.topic, "a");
        assert_eq!(p.qos, 1);
        assert_eq!(p.payload, b"hi");
    }

    #[test]
    fn parse_telemetry_flat_and_timestamped() {
        let flat = parse_telemetry(br#"{"temp":21.5,"on":true,"n":3}"#).unwrap();
        assert_eq!(flat.get("temp"), Some(&KvValue::Double(21.5)));
        assert_eq!(flat.get("on"), Some(&KvValue::Bool(true)));
        assert_eq!(flat.get("n"), Some(&KvValue::Long(3)));

        let ts = parse_telemetry(br#"{"ts":1700000000000,"values":{"temp":9}}"#).unwrap();
        assert_eq!(ts.get("temp"), Some(&KvValue::Long(9)));
        assert!(ts.get("ts").is_none());
    }

    #[test]
    fn parse_telemetry_rejects_non_object() {
        assert!(parse_telemetry(b"[1,2,3]").is_err());
        assert!(parse_telemetry(b"not json").is_err());
    }
}
