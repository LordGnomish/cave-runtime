// SPDX-License-Identifier: AGPL-3.0-or-later
// cave-desktop — native admin shell for CAVE Runtime.
//
// See docs/adr/ADR-PORTAL-DESKTOP-001-gpui-native-admin-shell.md for the design.
// The web portal at crates/cave-runtime/src/portal_index.html remains the
// primary UI; this binary is a GPUI-based companion for power-admins.

mod ui;
mod screens;

#[cfg(feature = "gpui-runtime")]
fn main() {
    // TODO(adr-portal-desktop-001): real GPUI bring-up.
    // Once we pin a Zed rev that builds clean in this workspace, this becomes:
    //
    //     gpui::App::new().run(|cx| {
    //         cx.open_window(Default::default(), |cx| {
    //             cx.new_view(screens::cluster_overview::ClusterOverview::new)
    //         });
    //     });
    //
    // Until that pin lands, the gpui-runtime feature still compiles (the dep
    // is pulled), but we exit immediately so CI doesn't open a window.
    eprintln!("cave-desktop: GPUI runtime feature is on but bring-up is pending");
    eprintln!("  see ADR-PORTAL-DESKTOP-001 for current status");
}

#[cfg(not(feature = "gpui-runtime"))]
fn main() {
    eprintln!("cave-desktop: built without `gpui-runtime` feature — nothing to run");
    eprintln!("  rebuild with: cargo run -p cave-desktop --features gpui-runtime");
    eprintln!("  see docs/adr/ADR-PORTAL-DESKTOP-001-gpui-native-admin-shell.md");
}
