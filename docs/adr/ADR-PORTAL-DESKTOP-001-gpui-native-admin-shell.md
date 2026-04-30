# ADR-PORTAL-DESKTOP-001: GPUI Native Desktop Shell Alongside Web Portal

**Status:** Accepted

**Scope:** Runtime

**Category:** Platform / Operator UX

**Date:** 2026-04-30

**Related ADRs:** ADR-011 (Backstage portal), ADR-145 (web composition), ADR-034 (tenant analytics)

## Context

CAVE Runtime ships `cave-runtime` (an HTTP service that serves `crates/cave-runtime/src/portal_index.html` — a single-file vanilla HTML/CSS/JS SPA) as the operator-facing UI. The web portal works on every device, supports multiple concurrent users, and reaches operators on mobile during incidents — that audience is non-negotiable.

But the same UI is also the daily driver for power-admins (platform engineers, SREs) who spend hours in cluster overview, log streaming, and resource graphs. For that audience, a browser tab competes with everything else they have open, latency is dominated by JSON fetches, and the UI cannot use OS-level affordances (global hotkeys, dock icon with badge, native notifications, fast windowing). Tools that this audience already loves — Linear (native + web), Tailscale (system-tray native), k9s (terminal), Zed (GPUI native) — all chose a native shell precisely for the daily-driver case.

Two options were considered:

1. **Web-only**: keep `portal_index.html` as the single UI. Mobile/multi-user works, power-admin keeps the tab open.
2. **Native-only**: replace web with a GPUI app. Power-admin gets a great experience, but mobile and ad-hoc multi-user access dies.
3. **Hybrid (chosen)**: web portal stays as the primary surface (mobile, multi-user, public links, embedding). A new `cave-desktop` crate adds a GPUI native shell tuned for power-admins, sharing the same backend HTTP API as the web portal.

## Candidates

| Criteria | Web-only | Native-only | **Hybrid (chosen)** |
|---|---|---|---|
| Mobile / on-call access | ✅ | ❌ | ✅ |
| Multi-user / shareable links | ✅ | ❌ | ✅ |
| Power-admin daily-driver UX | ⚠️ tab fatigue | ✅ | ✅ |
| Native notifications / dock | ❌ | ✅ | ✅ |
| Build-and-distribute cost | low | medium | medium |
| API divergence risk | none | none | **must enforce single backend** |
| Reference apps | Backstage, Grafana | k9s, Tailscale | Linear, 1Password, Zed |

## Decision

Adopt the **hybrid**. Web portal (`crates/cave-runtime/src/portal_index.html`) remains the primary, canonical UI. A new crate `crates/cave-desktop` ships a GPUI native admin shell as an *additional* surface for power-admins.

**Constraints — both surfaces share one backend:**

- All UI surfaces hit the same `cave-runtime` HTTP API. No desktop-only endpoints, no web-only endpoints.
- Auth, RBAC, and tenant scoping flow through the existing API. Desktop is just another HTTP client.
- Feature parity is *not* required — desktop can ship power-admin views the web doesn't have, but every endpoint it consumes must be reachable from the web portal too.
- Web portal is the deprecation gate: a feature is "shipped" when it works in the web portal. Desktop polish lands afterwards.

**GPUI rationale:**

- Zed's GPUI is the only mature Rust-native GPU-accelerated UI framework. Egui is too immediate-mode for our screens (lots of state); Slint and Iced lack the polish for a daily-driver tool.
- We accept the cost: GPUI is not on crates.io, must be pulled via git from `zed-industries/zed`, and pulls heavy native deps (Metal/Cocoa on macOS, Vulkan on Linux). Build time and link time will be non-trivial.
- We accept the risk: GPUI's API is not yet 1.0, so we will pin a specific `rev` and upgrade deliberately.

**References we are explicitly cribbing from:**

- **Zed** (`zed-industries/zed`) — GPUI itself, plus their pattern of treating the native app as the primary editor while a web reader exists separately.
- **Linear** — hybrid surface model (native macOS/Windows/Linux apps + linear.app web) sharing one GraphQL backend.
- **Tailscale** — native client that's a thin shell over a daemon HTTP API; the GUI never bypasses the daemon.
- **k9s** — proves operators want a fast, focused, keyboard-driven shell over kube-apiserver, even though `kubectl` and dashboard UIs exist.

## Consequences

### Positive

- Power-admins get a native daily-driver without losing mobile/multi-user access.
- Single backend means no data drift between surfaces.
- Crate is opt-in: if GPUI build cost becomes painful, it can be excluded from default workspace builds without affecting `cave-runtime`.

### Negative / accepted

- Two UI codebases to maintain (HTML/JS in `portal_index.html`, Rust+GPUI in `cave-desktop`).
- GPUI is unstable; we will track Zed's `main` and pin to known-good revs.
- Desktop binary distribution (notarization on macOS, signing on Windows) is future work — out of scope for the scaffold.

### Out of scope (this ADR)

- Auto-update mechanism for the desktop binary.
- Mobile native (iOS/Android) — web portal covers that.
- Replacing the web portal with desktop. Web stays primary.

## Implementation status

- This ADR introduced the scaffold: empty `crates/cave-desktop` with `App::new().run()` entry, three UI primitives (Panel / MetricCard / Table) as skeletons, and a `cluster_overview` placeholder screen. No real backend wiring yet.
- If the GPUI git dep does not resolve in our workspace (Zed workspace conflicts, native deps), the crate ships with the dep commented out and a `TODO(adr-portal-desktop-001)` marker — see the crate README for current state.
