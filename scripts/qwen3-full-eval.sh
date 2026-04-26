#!/usr/bin/env bash
# qwen3-full-eval.sh — runs smoke + parity gate + 4track in sequence
# Call after model download completes.
set -euo pipefail
MODEL="${1:-qwen3-coder-next:Q4_K_M}"
SCRIPTS="$(cd "$(dirname "$0")" && pwd)"
REPO="$(dirname "$SCRIPTS")"

echo "╔═══════════════════════════════════════════════════╗"
echo "║  Qwen3-Coder-Next Full Evaluation Pipeline        ║"
echo "╚═══════════════════════════════════════════════════╝"
echo "  Model: $MODEL"
echo "  Time:  $(date '+%Y-%m-%dT%H:%M:%S')"
echo ""

# 1. Smoke test
echo "Step 1/3: Smoke test (cold/warm latency)"
bash "$SCRIPTS/qwen3-smoke-test.sh" "$MODEL" 2>&1 | tee /tmp/qwen3-smoke.log

# 2. Parity gate
echo ""
echo "Step 2/3: Parity gate (STOP gate)"
PARITY_EXIT=0
bash "$SCRIPTS/qwen3-parity-gate.sh" "$MODEL" 2>&1 | tee /tmp/qwen3-parity.log || PARITY_EXIT=$?

echo ""
if [[ $PARITY_EXIT -eq 0 ]]; then
    echo "🟢 PARITY GATE: PASS — proceeding to 4-track test"
else
    echo "🔴 PARITY GATE: FAIL — stopping. Keep Qwen2.5-Coder as default."
    exit 1
fi

# 3. 4-track test
echo ""
echo "Step 3/3: Agentic 4-track test"
TRACK_EXIT=0
bash "$SCRIPTS/qwen3-4track-test.sh" "$MODEL" 2>&1 | tee /tmp/qwen3-4track.log || TRACK_EXIT=$?

echo ""
echo "╔═══════════════════════════════════════════════════╗"
echo "║  EVALUATION COMPLETE                              ║"
echo "╚═══════════════════════════════════════════════════╝"
echo "  Logs: /tmp/qwen3-smoke.log, /tmp/qwen3-parity.log, /tmp/qwen3-4track.log"
if [[ $TRACK_EXIT -eq 0 ]]; then
    echo "  Action: merge feat/local-llm-model-qwen3-switch + expand to 4-track template"
else
    echo "  Action: merge feat/local-llm-model-qwen3-switch (backend-only mode)"
fi
