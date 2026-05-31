# cave-iot-gateway

Pure-Rust IoT device-management gateway modelled on
[ThingsBoard](https://github.com/thingsboard/thingsboard) v4.3.1.2
(Apache-2.0), with an Eclipse Kura (EPL-1.0) telemetry-codec compatibility
layer.

Ports the **control-plane domain logic** of an IoT platform — no live
sockets or radios:

| Area | Module |
|------|--------|
| Device registry, profiles, credentials | `registry` |
| Provisioning (3 strategies + bulk) | `provisioning` |
| Claiming (secret + window → customer) | `claiming` |
| MQTT topic grammar + 3.1.1 PUBLISH codec | `transport::mqtt` |
| HTTP device-API path router | `transport::http` |
| CoAP RFC 7252 message codec | `transport::coap` |
| LoRaWAN MAC header + Cayenne LPP | `transport::lorawan` |
| Modbus PDU + Modbus/TCP MBAP | `transport::modbus` |
| Rule engine (filter/transform/action) | `rule_engine` |
| Device twin + attribute scopes | `twin` |
| Alarms (severity, ack/clear lifecycle) | `alarm` |
| OTA campaigns + update state machine | `ota` |
| Time-series ts_kv + aggregation | `timeseries` |
| Dashboards (widget data resolution) | `dashboard` |
| Multi-tenant quotas + rate limiting | `tenant` |

Live transport listeners (MQTT broker socket, CoAP/DTLS server, the LoRaWAN
network-server join procedure), widget UI rendering, and notification
delivery are out of scope — see `parity.manifest.toml`.

License: AGPL-3.0-or-later.
