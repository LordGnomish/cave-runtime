# Changelog — cave-iot-gateway

## 2026-05-31 — initial port (honest_ratio 0.75)

Fresh pure-Rust port of the ThingsBoard v4.3.1.2 IoT control plane, built
strict-TDD (RED test commit → GREEN impl commit) across 15 subsystems:

- device registry / provisioning / claiming
- telemetry ingestion codecs: MQTT, HTTP, CoAP (RFC 7252), LoRaWAN
  (MAC header + Cayenne LPP), Modbus (PDU + MBAP)
- rule engine, device twin + attribute scopes, alarms
- OTA campaigns, time-series ts_kv + aggregation, dashboards, multi-tenancy

103 tests (98 unit + 1 end-to-end pipeline + 4 codec property tests) + a
9-gate Charter v2 self-audit.

`fill_ratio = 1.0`, `honest_ratio = 0.75` ((15 mapped) / 20 total); the 5
`[[skipped]]` entries are live data-plane / radio / UI counterparts of
delivered features, genuinely out of a pure-Rust domain crate.
