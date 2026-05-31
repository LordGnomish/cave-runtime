// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//
//! End-to-end gateway pipeline: provision → MQTT decode → authenticate →
//! telemetry parse → time-series store → rule-engine routing. Exercises the
//! public surface of every major module as one coherent flow.

use cave_iot_gateway::provisioning::{
    ProvisionConfig, ProvisionRequest, ProvisionResponse, ProvisionService, ProvisionStrategy,
};
use cave_iot_gateway::registry::{DeviceProfile, DeviceRegistry, TransportType};
use cave_iot_gateway::rule_engine::{
    ActionKind, Message, Predicate, RuleChain, RuleNode, TransformOp,
};
use cave_iot_gateway::timeseries::{Aggregation, TsStore};
use cave_iot_gateway::transport::mqtt;
use cave_iot_gateway::KvValue;

#[test]
fn full_ingest_pipeline() {
    // 1. Registry + profile + provisioning config.
    let mut reg = DeviceRegistry::new();
    let pid = reg
        .save_profile(DeviceProfile::new("p", "tenant-a", "sensors", TransportType::Mqtt))
        .unwrap();
    let mut prov = ProvisionService::new();
    prov.register_config(ProvisionConfig {
        tenant_id: "tenant-a".into(),
        device_profile_id: pid,
        strategy: ProvisionStrategy::AllowCreateNewDevices,
        provision_device_key: "FLEET_KEY".into(),
        provision_device_secret: "FLEET_SECRET".into(),
    });

    // 2. A new device provisions itself and receives an access token.
    let token = match prov.provision(
        &mut reg,
        &ProvisionRequest {
            device_name: "boiler-7".into(),
            provision_device_key: "FLEET_KEY".into(),
            provision_device_secret: "FLEET_SECRET".into(),
        },
    ) {
        ProvisionResponse::Success { access_token, .. } => access_token,
        other => panic!("provisioning failed: {other:?}"),
    };

    // 3. The device publishes telemetry over MQTT; the gateway authenticates
    //    it by the token and decodes the PUBLISH packet.
    let device = reg.authenticate(&token).expect("token authenticates").clone();
    let packet = mqtt::encode_publish("v1/devices/me/telemetry", br#"{"temperature":82.5}"#);
    let publish = mqtt::decode_publish(&packet).unwrap();
    assert_eq!(
        mqtt::DeviceTopic::parse(&publish.topic).unwrap(),
        mqtt::DeviceTopic::Telemetry
    );
    let kv = mqtt::parse_telemetry(&publish.payload).unwrap();

    // 4. Persist to the time-series store.
    let mut ts = TsStore::new();
    for (k, v) in &kv {
        ts.insert(&device.id, k, 10_000, v.clone());
    }
    ts.insert(&device.id, "temperature", 20_000, KvValue::Double(90.0));
    let agg = ts.aggregate(&device.id, "temperature", 0, 30_000, 30_000, Aggregation::Max);
    assert_eq!(agg, vec![(0, 90.0)]);

    // 5. Route the message through a rule chain: high-temp → alarm topic.
    let mut chain = RuleChain::new();
    let filter = chain.add_node(RuleNode::Filter {
        predicate: Predicate::Gt("temperature".into(), 80.0),
    });
    let tag = chain.add_node(RuleNode::Transform {
        op: TransformOp::SetMetadata("severity".into(), "critical".into()),
    });
    let alarm = chain.add_node(RuleNode::Action {
        action: ActionKind::PushToTopic("alarms".into()),
    });
    chain.set_root(filter);
    chain.link(filter, "True", tag);
    chain.link(tag, "Success", alarm);

    let mut msg = Message::new(&device.id, "POST_TELEMETRY");
    msg.data = kv;
    let outcome = chain.process(msg);
    assert_eq!(outcome.actions, vec![ActionKind::PushToTopic("alarms".into())]);
    assert_eq!(
        outcome.message.metadata.get("severity").map(String::as_str),
        Some("critical")
    );
}
