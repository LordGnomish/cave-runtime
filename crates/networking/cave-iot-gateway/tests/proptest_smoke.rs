// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! Property tests for the wire codecs — round-trip invariants under
//! arbitrary inputs.

use cave_iot_gateway::transport::coap::{CoapCode, CoapMessage, CoapType};
use cave_iot_gateway::transport::mqtt;
use cave_iot_gateway::transport::modbus;
use proptest::prelude::*;

proptest! {
    #[test]
    fn mqtt_remaining_length_roundtrips(len in 0usize..=268_435_455) {
        let enc = mqtt::encode_remaining_length(len);
        let (dec, n) = mqtt::decode_remaining_length(&enc).unwrap();
        prop_assert_eq!(dec, len);
        prop_assert_eq!(n, enc.len());
        prop_assert!(enc.len() <= 4);
    }

    #[test]
    fn mqtt_publish_roundtrips(topic in "[a-z/]{1,40}", payload in proptest::collection::vec(any::<u8>(), 0..64)) {
        let pkt = mqtt::encode_publish(&topic, &payload);
        let p = mqtt::decode_publish(&pkt).unwrap();
        prop_assert_eq!(p.topic, topic);
        prop_assert_eq!(p.payload, payload);
        prop_assert_eq!(p.qos, 0);
    }

    #[test]
    fn coap_message_roundtrips(
        ty in 0u8..=3,
        class in 0u8..=7,
        detail in 0u8..=31,
        mid in any::<u16>(),
        token in proptest::collection::vec(any::<u8>(), 0..=8),
        payload in proptest::collection::vec(any::<u8>(), 0..40),
    ) {
        let msg = CoapMessage {
            version: 1,
            msg_type: CoapType::from_bits(ty).unwrap(),
            code: CoapCode::new(class, detail),
            message_id: mid,
            token,
            options: vec![],
            payload,
        };
        let back = CoapMessage::decode(&msg.encode()).unwrap();
        prop_assert_eq!(back, msg);
    }

    #[test]
    fn modbus_tcp_frame_roundtrips(
        txn in any::<u16>(),
        unit in any::<u8>(),
        start in any::<u16>(),
        qty in 1u16..=125,
    ) {
        let pdu = modbus::build_read_request(modbus::FunctionCode::ReadHoldingRegisters, start, qty);
        let frame = modbus::build_tcp_frame(txn, unit, &pdu);
        let parsed = modbus::parse_tcp_frame(&frame).unwrap();
        prop_assert_eq!(parsed.transaction_id, txn);
        prop_assert_eq!(parsed.unit_id, unit);
        prop_assert_eq!(parsed.pdu, pdu);
    }
}
