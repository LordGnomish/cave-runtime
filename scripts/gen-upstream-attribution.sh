#!/usr/bin/env bash
# Generate docs/upstream-attribution.md from all crates/*/parity.manifest.toml files.
set -euo pipefail
out="docs/upstream-attribution.md"
{
  echo "# Upstream Attribution"
  echo
  echo "Cave Runtime is a from-scratch Rust reimplementation. The following upstream"
  echo "open-source projects are referenced as parity targets — Cave Runtime does NOT"
  echo "vendor or redistribute their source code. Each upstream is licensed under its"
  echo "own terms in its own repository."
  echo
  echo "This file is generated from \`crates/*/parity.manifest.toml\`. Re-run"
  echo "\`scripts/gen-upstream-attribution.sh\` to refresh."
  echo
  echo "| Cave crate | Upstream | Version | Upstream repo |"
  echo "|---|---|---|---|"
} > "$out"

# awk extractor: scan [upstream] section, capture org/repo/version (TOML "string")
extract() {
  awk -F'"' '
    /^\[upstream\]/ { in_up=1; next }
    /^\[/           { in_up=0; next }
    in_up && /^[[:space:]]*org[[:space:]]*=/      { org=$2 }
    in_up && /^[[:space:]]*repo[[:space:]]*=/     { repo=$2 }
    in_up && /^[[:space:]]*version[[:space:]]*=/  { ver=$2 }
    END {
      if (org && repo) printf "%s\t%s\t%s\n", org, repo, ver
    }
  ' "$1"
}

count=0
for m in $(find crates -maxdepth 3 -name 'parity.manifest.toml' | sort); do
  crate=$(basename "$(dirname "$m")")
  line=$(extract "$m")
  if [ -n "$line" ]; then
    org=$(echo "$line" | cut -f1)
    repo=$(echo "$line" | cut -f2)
    ver=$(echo "$line" | cut -f3)
    url="https://github.com/${org}/${repo}"
    echo "| \`$crate\` | $org/$repo | ${ver:--} | [$url]($url) |" >> "$out"
    count=$((count+1))
  fi
done

{
  echo
  echo "Total: $count crates with declared upstream parity targets."
  echo
  echo "## Licenses"
  echo
  echo "All upstream projects listed above are licensed under permissive open-source"
  echo "licenses (Apache-2.0, MIT, BSD-style). Refer to each upstream repository's"
  echo "\`LICENSE\` file for the authoritative text. No upstream source code is"
  echo "redistributed by this project; only the parity-target reference is recorded."
} >> "$out"

echo "Generated $out ($count entries)"
