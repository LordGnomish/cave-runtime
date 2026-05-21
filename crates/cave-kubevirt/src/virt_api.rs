// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! virt-api: subresource HTTP surface.
//!
//! Upstream: kubevirt/kubevirt v1.8.2
//!   cmd/virt-api/virt-api.go (entrypoint)
//!   pkg/virt-api/rest/subresource.go (subresource APIs)
//!
//! virt-api is an aggregated APIService that hosts the per-VMI
//! subresources: console, vnc, pause/unpause, restart, migrate, freeze/
//! unfreeze, soft-reboot, screenshot, guestosinfo, userlist, filesystemlist.
//! Each maps to a `/apis/subresources.kubevirt.io/v1/...` URL.
//!
//! This module owns the route table + per-subresource handler enum.
//! cave-runtime supplies the websocket bridge for streaming consoles.

use serde::{Deserialize, Serialize};

/// All subresources exposed by virt-api. Each one corresponds to a URL
/// fragment after `/namespaces/{ns}/virtualmachineinstances/{name}/`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Subresource {
    /// Stream the serial console over a WebSocket.
    Console,
    /// Stream the VNC framebuffer over a WebSocket.
    Vnc,
    /// Pause libvirt domain.
    Pause,
    /// Unpause a paused domain.
    Unpause,
    /// Restart the VM (delete + recreate the VMI).
    Restart,
    /// Start a stopped VM.
    Start,
    /// Stop a running VM.
    Stop,
    /// Trigger a soft reboot (ACPI signal).
    SoftReboot,
    /// Freeze filesystems via QEMU guest agent.
    Freeze,
    /// Resume frozen filesystems.
    Unfreeze,
    /// Initiate live migration.
    Migrate,
    /// Take a screenshot of the framebuffer.
    Screenshot,
    /// Query guest-agent reported OS info.
    GuestOsInfo,
    /// Query guest-agent reported user list.
    UserList,
    /// Query guest-agent reported filesystem mounts.
    FilesystemList,
    /// Add a hotplug volume.
    AddVolume,
    /// Remove a hotplug volume.
    RemoveVolume,
    /// Query VMI status (synthetic — not in upstream's enum).
    Status,
}

impl Subresource {
    /// URL fragment for this subresource.
    pub fn url_fragment(&self) -> &'static str {
        match self {
            Subresource::Console => "console",
            Subresource::Vnc => "vnc",
            Subresource::Pause => "pause",
            Subresource::Unpause => "unpause",
            Subresource::Restart => "restart",
            Subresource::Start => "start",
            Subresource::Stop => "stop",
            Subresource::SoftReboot => "softreboot",
            Subresource::Freeze => "freeze",
            Subresource::Unfreeze => "unfreeze",
            Subresource::Migrate => "migrate",
            Subresource::Screenshot => "screenshot",
            Subresource::GuestOsInfo => "guestosinfo",
            Subresource::UserList => "userlist",
            Subresource::FilesystemList => "filesystemlist",
            Subresource::AddVolume => "addvolume",
            Subresource::RemoveVolume => "removevolume",
            Subresource::Status => "status",
        }
    }

    /// HTTP method expected for this subresource.
    pub fn http_method(&self) -> &'static str {
        match self {
            Subresource::Console
            | Subresource::Vnc
            | Subresource::Screenshot
            | Subresource::GuestOsInfo
            | Subresource::UserList
            | Subresource::FilesystemList
            | Subresource::Status => "GET",
            Subresource::AddVolume | Subresource::RemoveVolume => "PUT",
            _ => "PUT",
        }
    }

    /// Whether this subresource uses a WebSocket upgrade.
    pub fn is_websocket(&self) -> bool {
        matches!(
            self,
            Subresource::Console | Subresource::Vnc | Subresource::Screenshot
        )
    }

    /// Which target the API server should reach to fulfil this subresource:
    /// either the per-pod virt-launcher (for streaming + lifecycle), or
    /// virt-handler (for VMI lifecycle), or the controller (for VM-level).
    pub fn dispatch_target(&self) -> DispatchTarget {
        use DispatchTarget::*;
        match self {
            Subresource::Console
            | Subresource::Vnc
            | Subresource::Screenshot
            | Subresource::GuestOsInfo
            | Subresource::UserList
            | Subresource::FilesystemList
            | Subresource::Freeze
            | Subresource::Unfreeze => VirtLauncher,
            Subresource::Pause | Subresource::Unpause | Subresource::Migrate | Subresource::SoftReboot => {
                VirtHandler
            }
            Subresource::Start
            | Subresource::Stop
            | Subresource::Restart
            | Subresource::AddVolume
            | Subresource::RemoveVolume => VirtController,
            Subresource::Status => VirtController,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DispatchTarget {
    /// Streaming + per-pod operations.
    VirtLauncher,
    /// Per-node VMI lifecycle.
    VirtHandler,
    /// VM-level reconcile actions.
    VirtController,
}

/// Build the full subresource URL given the VMI coordinates.
pub fn subresource_url(namespace: &str, name: &str, sub: Subresource) -> String {
    format!(
        "/apis/subresources.kubevirt.io/v1/namespaces/{}/virtualmachineinstances/{}/{}",
        namespace,
        name,
        sub.url_fragment()
    )
}

/// Full list of subresources. Used by the OpenAPI registration.
pub const ALL_SUBRESOURCES: &[Subresource] = &[
    Subresource::Console,
    Subresource::Vnc,
    Subresource::Pause,
    Subresource::Unpause,
    Subresource::Restart,
    Subresource::Start,
    Subresource::Stop,
    Subresource::SoftReboot,
    Subresource::Freeze,
    Subresource::Unfreeze,
    Subresource::Migrate,
    Subresource::Screenshot,
    Subresource::GuestOsInfo,
    Subresource::UserList,
    Subresource::FilesystemList,
    Subresource::AddVolume,
    Subresource::RemoveVolume,
    Subresource::Status,
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn url_fragment_unique_per_subresource() {
        let frags: Vec<&str> = ALL_SUBRESOURCES.iter().map(|s| s.url_fragment()).collect();
        let mut uniq = frags.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(frags.len(), uniq.len(), "duplicate URL fragment");
    }

    #[test]
    fn console_and_vnc_are_get() {
        assert_eq!(Subresource::Console.http_method(), "GET");
        assert_eq!(Subresource::Vnc.http_method(), "GET");
    }

    #[test]
    fn lifecycle_subresources_are_put() {
        assert_eq!(Subresource::Pause.http_method(), "PUT");
        assert_eq!(Subresource::Restart.http_method(), "PUT");
        assert_eq!(Subresource::Migrate.http_method(), "PUT");
    }

    #[test]
    fn websocket_subresources() {
        assert!(Subresource::Console.is_websocket());
        assert!(Subresource::Vnc.is_websocket());
        assert!(Subresource::Screenshot.is_websocket());
        assert!(!Subresource::Pause.is_websocket());
        assert!(!Subresource::Start.is_websocket());
    }

    #[test]
    fn dispatch_targets_split_by_subresource() {
        assert_eq!(
            Subresource::Console.dispatch_target(),
            DispatchTarget::VirtLauncher
        );
        assert_eq!(
            Subresource::Pause.dispatch_target(),
            DispatchTarget::VirtHandler
        );
        assert_eq!(
            Subresource::Start.dispatch_target(),
            DispatchTarget::VirtController
        );
    }

    #[test]
    fn guest_agent_subresources_go_to_launcher() {
        for sub in &[
            Subresource::GuestOsInfo,
            Subresource::UserList,
            Subresource::FilesystemList,
            Subresource::Freeze,
            Subresource::Unfreeze,
        ] {
            assert_eq!(sub.dispatch_target(), DispatchTarget::VirtLauncher);
        }
    }

    #[test]
    fn subresource_url_format() {
        let url = subresource_url("default", "vm-1", Subresource::Console);
        assert_eq!(
            url,
            "/apis/subresources.kubevirt.io/v1/namespaces/default/virtualmachineinstances/vm-1/console"
        );
    }

    #[test]
    fn all_subresources_covers_documented_set() {
        let names: Vec<&str> = ALL_SUBRESOURCES.iter().map(|s| s.url_fragment()).collect();
        for required in &[
            "console",
            "vnc",
            "pause",
            "unpause",
            "restart",
            "start",
            "stop",
            "migrate",
            "freeze",
            "unfreeze",
            "guestosinfo",
            "addvolume",
            "removevolume",
        ] {
            assert!(names.contains(required), "missing {required}");
        }
    }

    #[test]
    fn subresource_serde() {
        let s = serde_json::to_string(&Subresource::Console).unwrap();
        assert_eq!(s, "\"console\"");
        let back: Subresource = serde_json::from_str(&s).unwrap();
        assert_eq!(back, Subresource::Console);
    }
}
