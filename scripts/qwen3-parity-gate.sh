#!/usr/bin/env bash
# qwen3-parity-gate.sh
# Asks Qwen3-Coder-Next to implement 3 known functions, compiles each, checks logic.
# Exit 0 = 3/3 pass. Exit 1 = <3/3.

set -euo pipefail

MODEL="${1:-qwen3-coder-next:Q4_K_M}"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
PASS=0
RESULTS=()
TMPDIR_GATE=$(mktemp -d /tmp/qwen3-gate-XXXXXX)
trap 'rm -rf "$TMPDIR_GATE"' EXIT

ollama_generate() {
    local model="$1" prompt="$2"
    local prompt_json
    prompt_json=$(python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" <<< "$prompt")
    curl -s "$OLLAMA_URL/api/generate" \
        -H 'Content-Type: application/json' \
        -d "{\"model\":\"$model\",\"prompt\":$prompt_json,\"stream\":false}" \
        | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('response',''))" \
        2>/dev/null || true
}

extract_rust() {
    python3 -c "
import sys, re
content = sys.stdin.read()
m = re.search(r'\`\`\`(?:rust)?\n(.*?)\`\`\`', content, re.DOTALL)
if m:
    print(m.group(1).strip())
else:
    print(content.strip())
"
}

# run_gate label fn_sig prompt logic_check_pattern
run_gate() {
    local label="$1"
    local fn_sig="$2"
    local prompt="$3"
    local logic_check="$4"   # grep pattern that must appear in the code

    echo ""
    echo "═══════════════════════════════════════════════════"
    echo "  GATE: $label"
    echo "═══════════════════════════════════════════════════"
    echo "  → Prompting $MODEL ..."

    local response
    response=$(ollama_generate "$MODEL" "$prompt")

    if [[ -z "$response" ]]; then
        echo "  ✗ FAIL — empty response from model"
        RESULTS+=("FAIL:$label:empty_response")
        return
    fi

    local rust_code
    rust_code=$(echo "$response" | extract_rust)

    if [[ -z "$rust_code" ]]; then
        echo "  ✗ FAIL — no Rust code block in response"
        RESULTS+=("FAIL:$label:no_rust_block")
        return
    fi

    echo "  → Got ${#rust_code} chars. Compiling..."

    # Write to temp file as a Rust library crate.
    local tmpfile="$TMPDIR_GATE/gate_${label//::/_}.rs"
    echo "$rust_code" > "$tmpfile"

    # Try to compile as a library.
    local compile_out
    if compile_out=$(rustc --edition 2021 --crate-type lib -o "$TMPDIR_GATE/out_${label//::/_}.rlib" "$tmpfile" 2>&1); then
        echo "  → Compiles OK."
    else
        echo "  ✗ FAIL — compile error:"
        echo "$compile_out" | tail -10 | sed 's/^/    /'
        RESULTS+=("FAIL:$label:compile_error")
        return
    fi

    # Logic check: verify a key pattern is present in the code.
    if [[ -n "$logic_check" ]] && ! echo "$rust_code" | grep -q "$logic_check"; then
        echo "  ✗ FAIL — logic check failed (pattern not found: $logic_check)"
        RESULTS+=("FAIL:$label:logic_check")
        return
    fi

    echo "  ✓ PASS"
    PASS=$((PASS + 1))
    RESULTS+=("PASS:$label")
}

# ── Gate 1: cave-sign::is_valid_digest ───────────────────────────────────────
run_gate \
    "cave-sign::is_valid_digest" \
    'pub fn is_valid_digest(digest: &str) -> bool' \
    "You are implementing Cave Runtime, a sovereign cloud OS in Rust.

Implement the function \`is_valid_digest\` in crate \`cave-sign\`:

\`\`\`rust
pub fn is_valid_digest(digest: &str) -> bool
\`\`\`

Rules:
- Return true only if the string has the format 'sha256:' followed by exactly 64 lowercase hex characters.
- Return false for everything else (empty, wrong prefix, wrong length, non-hex chars).

Output ONLY a \`\`\`rust ... \`\`\` block containing the complete function implementation. No prose." \
    "sha256"

# ── Gate 2: cave-pii::redact ─────────────────────────────────────────────────
run_gate \
    "cave-pii::redact" \
    'pub fn redact(matched: &str) -> String' \
    "You are implementing Cave Runtime, a sovereign cloud OS in Rust.

Implement the function \`redact\` in crate \`cave-pii\`:

\`\`\`rust
pub fn redact(matched: &str) -> String
\`\`\`

Rules:
- If the input has 4 or fewer characters, return a string of '*' of the same length.
- Otherwise, keep the first 2 and last 2 characters, replace everything in between with '*'.
- Example: 'hello' → 'he*lo', 'test@email.com' → 'te**********om'

Output ONLY a \`\`\`rust ... \`\`\` block containing the complete function implementation. No prose." \
    "\\*"

# ── Gate 3: cave-cost::cpu_efficiency ────────────────────────────────────────
run_gate \
    "cave-cost::cpu_efficiency" \
    'pub fn cpu_efficiency(cpu_cores: f64, cpu_cores_used: f64) -> f64' \
    "You are implementing Cave Runtime, a sovereign cloud OS in Rust.

Implement the function \`cpu_efficiency\` in crate \`cave-cost\`:

\`\`\`rust
pub fn cpu_efficiency(cpu_cores: f64, cpu_cores_used: f64) -> f64
\`\`\`

Rules:
- Return 0.0 if cpu_cores is 0.0.
- Otherwise return cpu_cores_used / cpu_cores, capped at 1.0 (never exceed 1.0).

Output ONLY a \`\`\`rust ... \`\`\` block containing the complete function implementation. No prose." \
    "min"

# ── Final verdict ─────────────────────────────────────────────────────────────
echo ""
echo "╔══════════════════════════════════════════════════╗"
printf "║  PARITY GATE RESULTS: %d/3 passed              ║\n" "$PASS"
echo "╚══════════════════════════════════════════════════╝"
for r in "${RESULTS[@]}"; do
    status="${r%%:*}"
    rest="${r#*:}"
    echo "  [$status] $rest"
done

if [[ $PASS -eq 3 ]]; then
    echo ""
    echo "✅ GATE PASS — proceed with Qwen3-Coder-Next model swap"
    exit 0
else
    echo ""
    echo "❌ GATE FAIL ($PASS/3) — keep Qwen2.5-Coder:32b, do not swap"
    exit 1
fi
