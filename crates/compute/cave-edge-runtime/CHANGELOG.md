# Changelog — cave-edge-runtime

## 2026-05-31 — Initial port (K3s edge mode + KubeEdge)

Fresh crate. Strict-TDD ports of the seven edge subsystems: edged (minimal
kubelet), metamanager (offline-first store), eventbus (MQTT bridge), edgehub
(reliable sync), devicetwin (delta engine), autonomy (connection state
machine), constrained (256 MB budget).
