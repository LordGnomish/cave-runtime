// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! MQTT transport codec — ThingsBoard device-API topic grammar +
//! MQTT 3.1.1 fixed-header / PUBLISH framing.
//!
//! Ports:
//! - `transport/mqtt/.../MqttTransportHandler` topic routing
//!   (`v1/devices/me/telemetry`, `.../attributes`, request/response RPC).
//! - The MQTT control-packet fixed header (packet type + flags + remaining
//!   length varint per MQTT-3.1.1 §2.2) and PUBLISH variable header.
//!
//! No broker socket — this decodes bytes the runtime data-plane feeds in.

use crate::{IotError, KvMap, KvValue, Result};

/// A parsed ThingsBoard device-API topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceTopic {
    /// `v1/devices/me/telemetry`
    Telemetry,
    /// `v1/devices/me/attributes` (publish client-side attribute updates)
    PostAttributes,
    /// `v1/devices/me/attributes/request/{id}` (request shared attributes)
    AttributesRequest(i32),
    /// `v1/devices/me/attributes/response/{id}`
    AttributesResponse(i32),
    /// `v1/devices/me/rpc/request/{id}` (server→device RPC)
    RpcRequest(i32),
    /// `v1/devices/me/rpc/response/{id}` (device→server RPC reply)
    RpcResponse(i32),
}

impl DeviceTopic {
    /// Parse a ThingsBoard MQTT topic string.
    pub fn parse(topic: &str) -> Result<DeviceTopic> {
        let t = topic.trim_matches('/');
        match t {
            "v1/devices/me/telemetry" => Ok(DeviceTopic::Telemetry),
            "v1/devices/me/attributes" => Ok(DeviceTopic::PostAttributes),
            _ => {
                if let Some(id) = t.strip_prefix("v1/devices/me/attributes/request/") {
                    return parse_id(id).map(DeviceTopic::AttributesRequest);
                }
                if let Some(id) = t.strip_prefix("v1/devices/me/attributes/response/") {
                    return parse_id(id).map(DeviceTopic::AttributesResponse);
                }
                if let Some(id) = t.strip_prefix("v1/devices/me/rpc/request/") {
                    return parse_id(id).map(DeviceTopic::RpcRequest);
                }
                if let Some(id) = t.strip_prefix("v1/devices/me/rpc/response/") {
                    return parse_id(id).map(DeviceTopic::RpcResponse);
                }
                Err(IotError::Codec(format!("unknown MQTT topic '{topic}'")))
            }
        }
    }
}

fn parse_id(s: &str) -> Result<i32> {
    s.parse::<i32>()
        .map_err(|_| IotError::Codec(format!("bad request id '{s}'")))
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

impl PacketType {
    fn from_nibble(n: u8) -> Result<PacketType> {
        Ok(match n {
            1 => PacketType::Connect,
            2 => PacketType::ConnAck,
            3 => PacketType::Publish,
            4 => PacketType::PubAck,
            8 => PacketType::Subscribe,
            9 => PacketType::SubAck,
            12 => PacketType::PingReq,
            13 => PacketType::PingResp,
            14 => PacketType::Disconnect,
            other => return Err(IotError::Codec(format!("unknown MQTT packet type {other}"))),
        })
    }
}

/// A decoded PUBLISH packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Publish {
    pub topic: String,
    pub qos: u8,
    pub retain: bool,
    pub payload: Vec<u8>,
}

/// Decode the MQTT "remaining length" varint (1-4 bytes, 7 bits each).
/// Returns `(value, bytes_consumed)`.
pub fn decode_remaining_length(buf: &[u8]) -> Result<(usize, usize)> {
    let mut multiplier: usize = 1;
    let mut value: usize = 0;
    let mut i = 0;
    loop {
        let byte = *buf
            .get(i)
            .ok_or_else(|| IotError::Codec("truncated remaining length".into()))?;
        value += (byte & 0x7F) as usize * multiplier;
        i += 1;
        if multiplier > 128 * 128 * 128 {
            return Err(IotError::Codec("malformed remaining length".into()));
        }
        multiplier *= 128;
        if byte & 0x80 == 0 {
            break;
        }
    }
    Ok((value, i))
}

/// Encode a length as an MQTT remaining-length varint.
pub fn encode_remaining_length(mut len: usize) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let mut byte = (len % 128) as u8;
        len /= 128;
        if len > 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if len == 0 {
            break;
        }
    }
    out
}

/// Decode the packet type from byte 1 of a fixed header.
pub fn packet_type(first_byte: u8) -> Result<PacketType> {
    PacketType::from_nibble(first_byte >> 4)
}

/// Decode a full PUBLISH packet (fixed header + variable header + payload).
pub fn decode_publish(buf: &[u8]) -> Result<Publish> {
    let first = *buf
        .first()
        .ok_or_else(|| IotError::Codec("empty packet".into()))?;
    if first >> 4 != 3 {
        return Err(IotError::Codec("not a PUBLISH packet".into()));
    }
    let retain = first & 0x01 != 0;
    let qos = (first >> 1) & 0x03;
    let (rem_len, consumed) = decode_remaining_length(&buf[1..])?;
    let body_start = 1 + consumed;
    let body = buf
        .get(body_start..body_start + rem_len)
        .ok_or_else(|| IotError::Codec("truncated PUBLISH body".into()))?;
    // Variable header: topic name = 2-byte length + UTF-8.
    if body.len() < 2 {
        return Err(IotError::Codec("missing topic length".into()));
    }
    let tlen = ((body[0] as usize) << 8) | body[1] as usize;
    let topic_bytes = body
        .get(2..2 + tlen)
        .ok_or_else(|| IotError::Codec("truncated topic".into()))?;
    let topic = String::from_utf8(topic_bytes.to_vec())
        .map_err(|_| IotError::Codec("topic not UTF-8".into()))?;
    let mut payload_start = 2 + tlen;
    // QoS > 0 carries a 2-byte packet identifier before the payload.
    if qos > 0 {
        payload_start += 2;
    }
    let payload = body.get(payload_start..).unwrap_or(&[]).to_vec();
    Ok(Publish { topic, qos, retain, payload })
}

/// Encode a PUBLISH packet (QoS 0) for a topic + payload.
pub fn encode_publish(topic: &str, payload: &[u8]) -> Vec<u8> {
    let mut var = Vec::with_capacity(2 + topic.len() + payload.len());
    var.extend_from_slice(&(topic.len() as u16).to_be_bytes());
    var.extend_from_slice(topic.as_bytes());
    var.extend_from_slice(payload);
    let mut out = vec![0x30]; // PUBLISH, QoS 0, no retain/dup
    out.extend_from_slice(&encode_remaining_length(var.len()));
    out.extend_from_slice(&var);
    out
}

/// Parse a telemetry JSON payload into a KvMap. Accepts both the flat
/// `{"key": value}` form and the timestamped `{"ts":N,"values":{...}}` form
/// (only the values are returned here; the ts belongs to the TS layer).
pub fn parse_telemetry(payload: &[u8]) -> Result<KvMap> {
    let v: serde_json::Value = serde_json::from_slice(payload)
        .map_err(|e| IotError::Codec(format!("telemetry not JSON: {e}")))?;
    let obj = match &v {
        serde_json::Value::Object(m) if m.contains_key("values") => m
            .get("values")
            .and_then(|x| x.as_object())
            .ok_or_else(|| IotError::Codec("'values' is not an object".into()))?,
        serde_json::Value::Object(m) => m,
        _ => return Err(IotError::Codec("telemetry payload must be an object".into())),
    };
    let mut out = KvMap::new();
    for (k, val) in obj {
        out.insert(k.clone(), json_to_kv(val));
    }
    Ok(out)
}

fn json_to_kv(v: &serde_json::Value) -> KvValue {
    match v {
        serde_json::Value::Bool(b) => KvValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                KvValue::Long(i)
            } else {
                KvValue::Double(n.as_f64().unwrap_or(0.0))
            }
        }
        serde_json::Value::String(s) => KvValue::Str(s.clone()),
        other => KvValue::Json(other.clone()),
    }
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
        // Spec examples: 64 → 1 byte, 321 → 2 bytes (0xC1 0x02).
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
        // Manually frame a QoS1 PUBLISH: topic "a", packet id 0x0005, payload "hi"
        let topic = b"a";
        let mut var = vec![0x00, 0x01, b'a', 0x00, 0x05];
        var.extend_from_slice(b"hi");
        let mut pkt = vec![0x32]; // PUBLISH | qos1
        pkt.extend_from_slice(&encode_remaining_length(var.len()));
        pkt.extend_from_slice(&var);
        let p = decode_publish(&pkt).unwrap();
        assert_eq!(p.topic, String::from_utf8_lossy(topic));
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
