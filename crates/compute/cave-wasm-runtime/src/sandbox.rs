//! Capability-based sandbox bridge.
//!
//! A WASI guest only reaches the host surfaces a [`Capabilities`] set grants.
//! This is the in-crate half of the bridge to the cave-sandbox triumvirate
//! (seccomp / namespace / landlock isolation): the runtime denies host calls in
//! software here, while cave-sandbox enforces OS-level confinement around the
//! whole process. Denied calls surface as [`WasmError::CapabilityDenied`].

use crate::limits::ResourceLimits;
use serde::{Deserialize, Serialize};

/// The host surfaces a guest is permitted to use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub stdout: bool,
    pub stderr: bool,
    pub args: bool,
    pub env: bool,
    pub clock: bool,
    /// Filesystem reads (currently gates `fd_read`).
    pub fs: bool,
    /// Fuel budget applied to the run.
    pub fuel: Option<u64>,
    /// Linear-memory page cap.
    pub max_memory_pages: u32,
}

impl Default for Capabilities {
    fn default() -> Self {
        Capabilities::permissive()
    }
}

impl Capabilities {
    /// Allow all host surfaces, unmetered (the default for trusted code).
    pub fn permissive() -> Self {
        Capabilities {
            stdout: true,
            stderr: true,
            args: true,
            env: true,
            clock: true,
            fs: true,
            fuel: None,
            max_memory_pages: 65536,
        }
    }

    /// Deny every host surface; a tightly metered, minimal-memory sandbox.
    pub fn none() -> Self {
        Capabilities {
            stdout: false,
            stderr: false,
            args: false,
            env: false,
            clock: false,
            fs: false,
            fuel: Some(0),
            max_memory_pages: 1,
        }
    }

    /// Builder: grant stdout/stderr.
    pub fn allow_stdio(mut self) -> Self {
        self.stdout = true;
        self.stderr = true;
        self
    }

    /// Builder: set a fuel budget.
    pub fn with_fuel(mut self, fuel: u64) -> Self {
        self.fuel = Some(fuel);
        self
    }

    /// Builder: set the memory page cap.
    pub fn with_memory_pages(mut self, pages: u32) -> Self {
        self.max_memory_pages = pages;
        self
    }

    /// Translate to the interpreter's resource limits.
    pub fn to_limits(&self) -> ResourceLimits {
        ResourceLimits {
            fuel: self.fuel,
            max_memory_pages: self.max_memory_pages,
        }
    }

    /// Whether a WASI function (with an optional file descriptor for `fd_write`)
    /// is permitted. `proc_exit` is always allowed so a guest can terminate.
    pub fn allows(&self, name: &str, fd: Option<i32>) -> bool {
        match name {
            "proc_exit" => true,
            "fd_write" => match fd {
                Some(1) => self.stdout,
                Some(2) => self.stderr,
                _ => false,
            },
            "fd_read" => self.fs,
            "args_sizes_get" | "args_get" => self.args,
            "environ_sizes_get" | "environ_get" => self.env,
            "clock_time_get" => self.clock,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permissive_allows_everything() {
        let c = Capabilities::permissive();
        assert!(c.allows("fd_write", Some(1)));
        assert!(c.allows("fd_write", Some(2)));
        assert!(c.allows("clock_time_get", None));
        assert!(c.allows("environ_get", None));
    }

    #[test]
    fn none_denies_but_allows_exit() {
        let c = Capabilities::none();
        assert!(!c.allows("fd_write", Some(1)));
        assert!(!c.allows("clock_time_get", None));
        assert!(c.allows("proc_exit", None));
    }

    #[test]
    fn fd_specific_gating() {
        let mut c = Capabilities::none();
        c.stdout = true;
        assert!(c.allows("fd_write", Some(1)));
        assert!(!c.allows("fd_write", Some(2)));
    }

    #[test]
    fn to_limits_maps_fuel_and_memory() {
        let c = Capabilities::none().with_fuel(500).with_memory_pages(4);
        let l = c.to_limits();
        assert_eq!(l.fuel, Some(500));
        assert_eq!(l.max_memory_pages, 4);
    }
}
