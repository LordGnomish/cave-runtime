// SPDX-License-Identifier: AGPL-3.0-or-later
//! Clean-room MongoDB-specific feature surface.
//!
//! Every file under `clean_room/` is authored fresh from the public
//! MongoDB wire-protocol RFC + mongodb.com/docs *only*. Do **not**
//! consult the GPL-licensed `mongodb/mongo` repo — the AGPL-3.0
//! re-licensing depends on this hygiene.
//!
//! Modules:
//! * [`change_streams`] — change-stream cursors (PG LISTEN/NOTIFY or
//!   in-process broadcast)
//! * [`oplog`]          — capped local oplog + replica state machine
//!   skeleton
//! * [`sharding`]       — minimal mongos-style router + chunk map

pub mod change_streams;
pub mod oplog;
pub mod sharding;
