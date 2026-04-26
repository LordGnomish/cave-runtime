#!/usr/bin/env bash
# qwen3-smoke-test.sh — cold-load latency + warm inference timing for Qwen3-Coder-Next
set -euo pipefail

MODEL="${1:-qwen3-coder-next:Q4_K_M}"
OLLAMA_URL="${OLLAMA_URL:-http://localhost:11434}"
PROMPT='write a Rust function: pub fn add(a: i32, b: i32) -> i32'

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

echo "══════════════════════════════════════════════════════"
echo "  Qwen3-Coder-Next Smoke Test"
echo "  Model: $MODEL"
echo "══════════════════════════════════════════════════════"
echo ""

# Cold load (first inference — model loads into VRAM)
echo "[1/2] Cold load inference..."
COLD_START=$(python3 -c "import time; print(int(time.time()*1000))")
COLD_OUTPUT=$(ollama_generate "$MODEL" "$PROMPT")
COLD_END=$(python3 -c "import time; print(int(time.time()*1000))")
COLD_MS=$(( COLD_END - COLD_START ))

if [[ -z "$COLD_OUTPUT" ]]; then
    echo "✗ Cold inference failed (empty output or timeout)"
    exit 1
fi

echo "✓ Cold output (${#COLD_OUTPUT} chars):"
echo "$COLD_OUTPUT" | head -8
echo ""
echo "  Cold latency: ${COLD_MS}ms ($(( COLD_MS / 1000 ))s)"

# Warm inference (model already in memory)
echo ""
echo "[2/2] Warm inference..."
WARM_START=$(python3 -c "import time; print(int(time.time()*1000))")
WARM_OUTPUT=$(ollama_generate "$MODEL" "$PROMPT")
WARM_END=$(python3 -c "import time; print(int(time.time()*1000))")
WARM_MS=$(( WARM_END - WARM_START ))

echo "  Warm latency: ${WARM_MS}ms ($(( WARM_MS / 1000 ))s)"

# Memory from ollama ps
echo ""
echo "[3/3] Ollama process memory:"
ollama ps 2>/dev/null || echo "  (ollama ps not available)"

echo ""
echo "══════════════════════════════════════════════════════"
echo "  SMOKE TEST SUMMARY"
echo "  Cold load: ${COLD_MS}ms | Warm: ${WARM_MS}ms"
echo "  Model size: 51GB Q4_K_M (80B MoE, 3B active)"
echo "══════════════════════════════════════════════════════"
