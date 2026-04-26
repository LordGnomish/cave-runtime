#!/usr/bin/env bash
# qwen3-4track-test.sh
# Checks if Qwen3-Coder-Next can return all 4 tracks in a single prompt.
# Outputs PASS if all 4 tracks are present, FAIL otherwise.

MODEL="${1:-qwen3-coder-next:Q4_K_M}"

PROMPT='You are implementing Cave Runtime, a sovereign cloud OS in Rust.

Task: implement `cave_quota::throttle` — a quota throttling function for the Cave platform.
You must generate ALL FOUR tracks in your response:

## Track 1: Rust Backend (cave-quota crate)
```rust
// In crates/cave-quota/src/throttle.rs
```
Implement:
- `pub fn throttle(namespace: &str, limit: u64, current: u64) -> bool`
  Returns true if the request should be throttled (current >= limit).
- `#[test] fn test_throttle_at_limit()` — passes when current == limit
- `#[test] fn test_throttle_below_limit()` — passes when current < limit
- `#[test] fn test_throttle_above_limit()` — passes when current > limit

## Track 2: Portal Component (React/TypeScript)
```tsx
// In portal/src/pages/QuotaPanel.tsx
```
A minimal React functional component `QuotaPanel` that:
- Accepts `namespace: string` and `quotaUsed: number` and `quotaLimit: number` props
- Renders a table row showing the quota status

## Track 3: CLI Command (cavectl)
```rust
// In crates/cave-cli/src/quota.rs
```
A clap subcommand handler:
- `cave quota show --namespace <ns>` prints namespace, quota-used, quota-limit

## Track 4: Prometheus Metric
```rust
// In crates/cave-quota/src/metrics.rs
```
- A `Counter` named `cave_quota_throttle_total` with label `namespace`
- Incremented when throttle() returns true

Output all four tracks in order. Each must be inside a code fence with the file path comment as shown.'

OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"

echo "Prompting $MODEL for 4-track agentic test..."
echo "ETA: 2-5 minutes for 80B MoE..."
echo ""

PROMPT_JSON=$(python3 -c "import json,sys; print(json.dumps(sys.stdin.read()))" <<< "$PROMPT")
RESPONSE=$(curl -s "$OLLAMA_URL/api/generate" \
    -H 'Content-Type: application/json' \
    -d "{\"model\":\"$MODEL\",\"prompt\":$PROMPT_JSON,\"stream\":false}" \
    | python3 -c "import sys,json; d=json.load(sys.stdin); print(d.get('response',''))" \
    2>/dev/null || true)

if [[ -z "$RESPONSE" ]]; then
    echo "RESULT: FAIL — empty response"
    exit 1
fi

# Count how many of the 4 tracks are present
FOUND=0

if echo "$RESPONSE" | grep -q "fn throttle"; then
    echo "✓ Track 1 (Rust backend): present"
    FOUND=$((FOUND + 1))
else
    echo "✗ Track 1 (Rust backend): MISSING"
fi

if echo "$RESPONSE" | grep -q "QuotaPanel"; then
    echo "✓ Track 2 (React component): present"
    FOUND=$((FOUND + 1))
else
    echo "✗ Track 2 (React component): MISSING"
fi

if echo "$RESPONSE" | grep -q "quota show\|quota_show\|QuotaShow"; then
    echo "✓ Track 3 (CLI command): present"
    FOUND=$((FOUND + 1))
else
    echo "✗ Track 3 (CLI command): MISSING"
fi

if echo "$RESPONSE" | grep -q "cave_quota_throttle_total\|throttle_total"; then
    echo "✓ Track 4 (Prometheus metric): present"
    FOUND=$((FOUND + 1))
else
    echo "✗ Track 4 (Prometheus metric): MISSING"
fi

echo ""
echo "═══════════════════════════════════════════════"
echo "  4-TRACK RESULT: $FOUND/4 tracks present"
echo "═══════════════════════════════════════════════"

if [[ $FOUND -eq 4 ]]; then
    echo "✅ PASS — expand daemon template to 4-track mode"
    # Save response for manual review
    echo "$RESPONSE" > /tmp/qwen3-4track-response.txt
    echo "Response saved to /tmp/qwen3-4track-response.txt"
    exit 0
else
    echo "⚠️  PARTIAL ($FOUND/4) — keep backend-only mode, Sonnet refactor pass for other tracks"
    echo "$RESPONSE" > /tmp/qwen3-4track-response.txt
    exit 1
fi
