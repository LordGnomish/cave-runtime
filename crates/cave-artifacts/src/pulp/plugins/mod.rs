// SPDX-License-Identifier: AGPL-3.0-or-later
// Source: META — cave-artifacts pulp plugins root (multi-upstream)
//! Plugin implementations — one per Pulp content plugin.

pub mod ansible;
pub mod container;
pub mod deb;
pub mod file;
pub mod maven;
pub mod python;
pub mod rpm;

pub use ansible::AnsiblePlugin;
pub use container::ContainerPlugin;
pub use deb::DebPlugin;
pub use file::FilePlugin;
pub use maven::MavenPlugin;
pub use python::PythonPlugin;
pub use rpm::RpmPlugin;
