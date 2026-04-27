//! PersistentVolume controllers — `pkg/controller/volume/persistentvolume`
//! and `pkg/controller/volume/expand`.
//!
//! Currently implemented:
//!
//! * [`binder`] — claim/volume binding logic (immediate vs WaitForFirstConsumer).
//! * [`expansion`] — volume expansion state machine.

pub mod binder;
pub mod expansion;
pub mod reclaim;
pub mod snapshot;
