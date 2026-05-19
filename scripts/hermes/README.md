# scripts/hermes/

Source-of-truth reference files for the Hermes Agent install on the
cave-runtime macOS host. See [`docs/adr/ADR-150_Hermes_Agent_Adoption_AC_Path.md`](../../docs/adr/ADR-150_Hermes_Agent_Adoption_AC_Path.md)
for the why.

## Files

| Path | Lives on host as | Role |
|---|---|---|
| `../hermes-pump-bridge.sh` | (run from repo)  | Read-only bridge: pump-state inspection, optional Hermes routing/recovery. Ships chmod 644 — Burak `chmod +x` after manual validation. |
| `../com.cave.hermes-orchestrator.plist` | `~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist` | launchd unit, ships **`Disabled=true`**. Burak bootstraps it manually. |
| `config.yaml.reference` | `~/.hermes/config.yaml` (only the edits) | The surgical edits applied to the installer-default config; lets us reproduce the host state from a fresh install without re-deriving them. |

## Smoke test (host)

```sh
~/.local/bin/hermes --version
~/.local/bin/hermes doctor
~/.local/bin/hermes -z "ping"
bash scripts/hermes-pump-bridge.sh status
```

## Activation (Burak, when ready)

```sh
chmod +x scripts/hermes-pump-bridge.sh
plutil -lint scripts/com.cave.hermes-orchestrator.plist
# Edit the host copy at ~/Library/LaunchAgents/... to remove
# <key>Disabled</key><true/>, OR:
/bin/launchctl enable    gui/$UID/com.cave.hermes-orchestrator
/bin/launchctl bootstrap gui/$UID ~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist
```

## Rollback

```sh
/bin/launchctl bootout gui/$UID/com.cave.hermes-orchestrator
rm ~/Library/LaunchAgents/com.cave.hermes-orchestrator.plist
~/.local/bin/hermes uninstall  # or: rm -rf ~/.hermes ~/.local/bin/hermes
```

Pump units (`com.btartan.cave-upstream-watchd`,
`com.caveruntime.qwen-pump`, etc.) are untouched by this work and
continue running independently.
