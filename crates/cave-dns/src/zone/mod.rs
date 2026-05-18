// SPDX-License-Identifier: AGPL-3.0-or-later
pub mod file;
pub mod manager;
pub mod transfer;
pub mod update;
pub mod zone;

pub use manager::ZoneManager;
pub use zone::{LookupResult, Zone};
