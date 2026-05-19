# Runbook — Optional GitHub App enrichment for `cave-upstream-watchd`

**Status:** Optional. The daemon polls the public Atom feed by
default (ADR-026); the App below is only needed if a downstream
consumer asks for asset URLs, the prerelease flag, or the full
markdown release notes.

## Why

* Upgrade from Atom (HTML body, no asset URLs, no prerelease flag)
  to the REST JSON API on a per-tick basis.
* No PAT required — App auth uses an installation token minted on
  demand from a JWT signed with the App's RSA private key.
* Rate limit: 5000 req/h per installation (vs. the 60 req/h anon
  ceiling).

## One-time setup

### 1. Create the GitHub App

1. https://github.com/settings/apps → **New GitHub App**.
2. Name: `cave-upstream-watchd` (or pick anything — the daemon
   only cares about the ID).
3. Homepage URL: any (https://github.com/LordGnomish/cave-runtime
   is fine).
4. **Uncheck** Webhook (we only pull).
5. Permissions (Repository):
   * **Contents:** Read-only
   * **Metadata:** Read-only (mandatory; auto-selected)
   * Everything else: No access
6. **Where can this GitHub App be installed?** Only on this
   account. (Switch to Any account if you intend to share.)
7. Create. Copy the **App ID** from the top of the settings page.

### 2. Generate + download the private key

1. Same App settings page → scroll to **Private keys** → **Generate
   a private key**. A `.pem` file downloads automatically.
2. Move it somewhere safe — we will hand it to the macOS keychain
   and then delete the file. The keychain is the durable copy.

### 3. Install the App on the tracked repos

1. App settings → **Install App** → Install on your account.
2. Pick **All repositories** (simplest) or **Only select
   repositories** and pick the upstream OSS forks you track.
3. Confirm. GitHub redirects back to the App settings page.

### 4. Stash credentials in the macOS keychain

```sh
APP_ID="123456"   # numeric ID from step 1
PEM_PATH="$HOME/Downloads/cave-upstream-watchd.2026-05-19.private-key.pem"

security add-generic-password -U \
  -s cave-upstream-github-app-id \
  -a "$USER" \
  -w "$APP_ID"

security add-generic-password -U \
  -s cave-upstream-github-app \
  -a "$USER" \
  -w "$(cat "$PEM_PATH")"

# Verify (-w prints the secret to stdout):
security find-generic-password -s cave-upstream-github-app-id -a "$USER" -w
security find-generic-password -s cave-upstream-github-app    -a "$USER" -w | head -1

# Once verified, destroy the on-disk PEM:
shred -uz "$PEM_PATH" 2>/dev/null || rm -P "$PEM_PATH"
```

### 5. Opt the daemon into the App path

```sh
launchctl setenv CAVE_WATCHD_PRIMARY auto

# Reload the poller plist so the new env takes effect:
launchctl unload ~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist
launchctl load   ~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist
```

The daemon will log `GitHub App detected (Phase 2 …)` at boot. The
Atom path remains the actual fetcher until Phase 2 lands the
token-exchange wiring; the App detection is exercised end-to-end
today.

## Verification

Right after the next 5-min tick:

```sh
tail -20 ~/Library/Logs/cave-upstream-watchd-poller.log | \
  grep -E 'poll strategy|GitHub App|atom'
```

Expected lines:

* `poll strategy resolved strategy=Atom`  (default + Phase 1 of App)
* `GitHub App detected (Phase 2 …)`       (if step 4 succeeded)

## Rolling back

The App is purely additive. To roll back:

```sh
security delete-generic-password -s cave-upstream-github-app    -a "$USER"
security delete-generic-password -s cave-upstream-github-app-id -a "$USER"
launchctl unsetenv CAVE_WATCHD_PRIMARY
launchctl unload && launchctl load \
  ~/Library/LaunchAgents/com.cave.upstream-watchd-poller.plist
```

The daemon falls back to the pure Atom path with the next tick.

## Disposing of the legacy PATs

The pre-2026-05-19 leaked PATs are sitting in the keychain under
`cave-upstream-legacy-poller` / `cave-upstream-legacy-watchd`. Once
the App path is happy you can revoke + delete them:

```sh
# Optional: see the values before revoking, to confirm they're
# the right tokens.
security find-generic-password -s cave-upstream-legacy-poller -a "$USER" -w
security find-generic-password -s cave-upstream-legacy-watchd -a "$USER" -w

# Then on github.com/settings/tokens → revoke the PATs above.
# Then:
security delete-generic-password -s cave-upstream-legacy-poller -a "$USER"
security delete-generic-password -s cave-upstream-legacy-watchd -a "$USER"
```
