// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026 Cave Runtime contributors
//
// Source: apache/kafka@9f8b3ad416bd416f3706f3d7a1f425b9dd8bc5f2
//! Kafka-specific submodules. New Kafka work is placed under
//! `src/kafka/`; existing flat files at `src/` (kafka_wire.rs,
//! kafka_protocol.rs, consumer_group.rs, …) stay where they are
//! per ADR — directory reorg is deferred to Phase-2 backlog.

pub mod kip848;
