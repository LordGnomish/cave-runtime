# ADR-RUNTIME-CLI-CONSOLIDATION-001 — Single `cavectl` Binary, Native + Compatibility Surfaces

**Status:** Decided (2026-04-27, Burak Tartan)
**Scope:** Cave Runtime CLI (`cave-cli` crate → `cavectl` binary)
**Category:** Developer experience / consolidation
**Supersedes:** ADR-076 (cave_ctl CLI MCP Server Architecture) on the binary-name and surface question.

## Context

Cave Runtime reimplements ~60 upstream projects under one roof. Each upstream has its own
CLI conventions: `kubectl get pods`, `helm install`, `argocd app sync`, `vault read kv/db`,
`docker push`. Operators arriving at Cave today carry years of muscle memory built on those
verbs.

At the same time, Cave's value proposition is *cross-cutting* — federated watch, tenant-scoped
quota, consistent cross-module snapshot, self-improvement playbooks — and these have **no
upstream-CLI equivalent**. Forcing them into a `kubectl plugin` shape would distort them.

The naming has drifted across drafts: `cave-cli`, `cavectl`, `cave-ctl`. ADR-076 left the
binary undecided. The result: shipping doc references three names, and the in-tree
`cave-cli` crate has wavered between binary names `cave` (current) and `cavectl` (planned).

## Decision

Cave ships **one binary** — `cavectl` — with **two coexisting subcommand surfaces**:

1. **Native (canonical):** Cave-domain verbs that compose across modules.
   `cavectl deploy …`, `cavectl get pods`, `cavectl logs <crate>`, `cavectl flag toggle`,
   `cavectl chaos run`, `cavectl topology …`, `cavectl secrets list`, `cavectl describe …`,
   `cavectl events …`. These are upstream-agnostic, multi-tenant by construction, and
   first-class targets for cross-module composition.

2. **Compatibility (bridge):** Drop-in shims for the upstream CLIs Cave reimplements.
   `cavectl kubectl get pods`, `cavectl helm install …`, `cavectl argocd app sync`,
   `cavectl vault read kv/db`, `cavectl harbor push …`. Each shim accepts the upstream's
   exact flag set and output format, then maps internally to native actions. Operators
   are productive in minute one without re-learning.

The crate is renamed `cave-cli` → `cavectl` to remove the binary-vs-crate split. The
`[[bin]] name` and `[package] name` agree.

### Why two surfaces, not one

A pure-native CLI strands kubectl muscle-memory and slows adoption. A pure-compat CLI
hides Cave's actual differentiators behind upstream verbs that don't have an equivalent.
Two surfaces let each one optimise for its job:

- Native is what Cave-fluent operators reach for. It exists because `kubectl` cannot
  express `cavectl topology --tenant acme --since 1h` cleanly.
- Compat is what newcomers reach for. It exists because `cavectl kubectl get pods` is
  exactly what a kubectl user wants to type, and we can satisfy that without owning
  `kubectl` itself.

Both surfaces share the same library underneath — same auth, same tenant scope, same
output formatters, same audit trail. A compat invocation routes to the same code path
the equivalent native invocation would.

### Why one binary, not many

A separate `cavehelm` / `cavekubectl` / `cavevault` set duplicates: install path, auth
config, tenant scope, output format, telemetry, completion install, version skew. One
binary with subcommands keeps those concerns single-source.

## Consequences

### Positive
- One install, one auth flow, one audit trail.
- Compat surface absorbs upstream changes via flag mapping rather than fork drift.
- Native surface is free to express cross-module verbs without `kubectl plugin`
  contortions.
- Shell completion ships once for the whole CLI.

### Negative
- The crate ships *both* surfaces, so the test surface is roughly 2× a single-surface
  CLI. Mitigated by a shared core: each compat shim is a thin mapping layer over the
  native action, tested at the shim boundary plus the native unit tests it delegates to.
- Operators may type `cavectl get` (native, Cave resources) when they meant `cavectl
  kubectl get` (compat, classic Kubernetes API). We accept this: the native verb is the
  canonical one, and the compat shim is opt-in via the `kubectl` prefix.

### Neutral
- ADR-076's MCP-server architecture is unchanged; it now plugs into `cavectl mcp …` as
  a native subcommand.
- Existing `cave-cli` crate references in the workspace migrate to `cavectl` mechanically
  (Cargo.toml package name, workspace member entry). The crate's directory stays
  `crates/cave-cli` only as a historical convenience and will follow in a tree-wide
  rename pass.

## Milestones (M-series)

| M | Surface | Scope |
|---|---------|-------|
| M1 | Native | `deploy / get / describe / logs / events / secrets / flag / chaos / topology` — Cave-domain, upstream-agnostic |
| M2 | Compat | `kubectl` shim — flag/format mapping onto native actions |
| M3 | Compat | `helm` and `argocd` shims |
| M4 | Compat | `vault`/`openbao` and `harbor`/`registry` shims |
| M5 | Native | TUI mode (k9s-style, terminal-first; cave-portal not required) |
| M6 | Native | Shell completion, telemetry, tenant-scope flag plumbing |

Each milestone adds 30–50 tests. The compat shims are tested at the mapping boundary;
the native verbs they delegate to keep their own tests.

## Naming summary

| Use | Name |
|---|---|
| Crate (Cargo package) | `cavectl` |
| Binary | `cavectl` |
| Library (`use ...`) | `cavectl` |
| Workspace dir (kept for now) | `crates/cave-cli/` |
| Doc references (deprecated) | ~~`cave-cli`~~, ~~`cave-ctl`~~, ~~`cave`~~ |
