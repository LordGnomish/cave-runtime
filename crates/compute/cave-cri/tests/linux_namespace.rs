// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
//! Linux code paths of `cave_cri::namespace`, exercised without privileges.
//!
//! `build_clone_flags` is a pure function of the `NamespaceConfig`, so its
//! Linux branch can be checked bit for bit. The expected values are the
//! `CLONE_NEW*` constants of the kernel ABI (`include/uapi/linux/sched.h`,
//! listed in clone(2) and namespaces(7)); they are spelled out here instead
//! of being imported, so the test does not share a source with the code.

#![cfg(target_os = "linux")]

use cave_cri::namespace::{build_clone_flags, create_namespaces, NamespaceConfig};

const CLONE_NEWNS: i32 = 0x0002_0000;
const CLONE_NEWUTS: i32 = 0x0400_0000;
const CLONE_NEWIPC: i32 = 0x0800_0000;
const CLONE_NEWUSER: i32 = 0x1000_0000;
const CLONE_NEWPID: i32 = 0x2000_0000;
const CLONE_NEWNET: i32 = 0x4000_0000;

fn no_namespaces() -> NamespaceConfig {
    NamespaceConfig {
        pid: false,
        net: false,
        mnt: false,
        uts: false,
        ipc: false,
        user: false,
    }
}

#[test]
fn each_namespace_maps_to_its_own_clone_flag() {
    let cases = [
        (
            "pid",
            NamespaceConfig {
                pid: true,
                ..no_namespaces()
            },
            CLONE_NEWPID,
        ),
        (
            "net",
            NamespaceConfig {
                net: true,
                ..no_namespaces()
            },
            CLONE_NEWNET,
        ),
        (
            "mnt",
            NamespaceConfig {
                mnt: true,
                ..no_namespaces()
            },
            CLONE_NEWNS,
        ),
        (
            "uts",
            NamespaceConfig {
                uts: true,
                ..no_namespaces()
            },
            CLONE_NEWUTS,
        ),
        (
            "ipc",
            NamespaceConfig {
                ipc: true,
                ..no_namespaces()
            },
            CLONE_NEWIPC,
        ),
        (
            "user",
            NamespaceConfig {
                user: true,
                ..no_namespaces()
            },
            CLONE_NEWUSER,
        ),
    ];

    for (name, config, expected) in cases {
        assert_eq!(
            build_clone_flags(&config),
            expected,
            "{name} namespace: got {:#010x}, expected {expected:#010x}",
            build_clone_flags(&config)
        );
    }
}

/// A network namespace must not be requested as a mount namespace: with the
/// two confused, a container asking only for `net` would share the host's
/// network stack.
#[test]
fn a_network_namespace_is_not_a_mount_namespace() {
    let net_only = NamespaceConfig {
        net: true,
        ..no_namespaces()
    };

    let flags = build_clone_flags(&net_only);

    assert_eq!(flags & CLONE_NEWNET, CLONE_NEWNET);
    assert_eq!(flags & CLONE_NEWNS, 0);
}

#[test]
fn the_default_config_isolates_everything_but_the_user_namespace() {
    let flags = build_clone_flags(&NamespaceConfig::default());

    assert_eq!(
        flags,
        CLONE_NEWPID | CLONE_NEWNET | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC
    );
    assert_eq!(flags & CLONE_NEWUSER, 0);
}

#[test]
fn all_namespaces_combine_into_one_flag_word() {
    let all = NamespaceConfig {
        pid: true,
        net: true,
        mnt: true,
        uts: true,
        ipc: true,
        user: true,
    };

    assert_eq!(
        build_clone_flags(&all),
        CLONE_NEWPID | CLONE_NEWNET | CLONE_NEWNS | CLONE_NEWUTS | CLONE_NEWIPC | CLONE_NEWUSER
    );
}

#[test]
fn create_namespaces_records_the_flags_for_its_config() {
    let config = NamespaceConfig {
        net: true,
        uts: true,
        ..no_namespaces()
    };

    let set = create_namespaces(&config).unwrap();

    assert_eq!(set.clone_flags, CLONE_NEWNET | CLONE_NEWUTS);
}
