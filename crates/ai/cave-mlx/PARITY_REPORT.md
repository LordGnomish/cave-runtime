<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-mlx — Charter v2 Parity Report

**Upstream:** [ml-explore/mlx](https://github.com/ml-explore/mlx) `v0.31.2`
(`68cf2fddd8de5edd8ab3d926391772b2e2cedad8`, 2026-04-22) — MIT.
**Backend:** pure-Rust, CPU, eager. No FFI, no Metal/GPU, no lazy graph.
**Audit date:** 2026-05-30.

## Self-audit gates — 9/9 PASS

| # | Gate | Result |
|---|------|--------|
| 1 | SPDX on 100% of `src/*.rs` | ✅ |
| 2 | `source_sha` pinned (`v0.31.2`) | ✅ |
| 3 | `last_audit` is a 2026 date | ✅ |
| 4 | `parity_ratio_source = "manifest"` | ✅ |
| 5 | `fill_ratio >= 0.95` | ✅ (0.9615) |
| 6 | `mapped+partial+skipped+unmapped == total` | ✅ |
| 6b | `honest_ratio` consistent and `<= fill_ratio` | ✅ |
| 7 | no `unimplemented!()` / `todo!()` in `src/` | ✅ |
| 8 | this report exists | ✅ |
| 9 | Charter v2 composite | ✅ |

## Parity ledger

- **fill_ratio = 0.9615** = (16 mapped + 1 partial + 8 skipped) / 26
- **honest_ratio = 0.9231** = (16 mapped + 8 skipped) / 26

### Mapped (16, all strict-TDD)
`Array` N-dim tensor · broadcasting elementwise (add/sub/mul/div) · unary math
(exp/log/sqrt/neg) · activations (relu/sigmoid/tanh) · matmul · transpose ·
reductions (sum/mean/max) · softmax · reverse-mode autograd
(`grad`/`value_and_grad`) · `nn.Linear` · `nn.Activation` · `Sgd` · `Adam` ·
`AdamW` · group-wise affine quantization (4/8-bit) · `cave-mlx` CLI.

### Partial (1)
`mx.random` — only a seeded Kaiming-uniform initializer is present (used by
`Linear::new`); the full distribution suite is not ported.

### Skipped (8)
Seven architectural cuts for a sovereign CPU eager core (lazy-eval graph /
`mx.eval`, Metal/GPU + unified memory, `mx.compile`, `mx.distributed`,
`mx.fft`, `mx.linalg`, `vmap`/jacobian transforms) plus one parallel-track
delegation (safetensors/gguf weight loading → `cave-local-llm/src/gguf.rs`).

### Unmapped (1)
Convolution (`conv1d`/`conv2d`/pooling) — an honestly-tracked in-scope gap
deferred to a follow-up slice. It is **not** dressed as a justified scope-cut;
it intentionally lowers both ratios.

## Tests

46 crate tests pass (8 array · 12 ops · 8 autograd · 7 nn · 6 optim · 5 quant)
plus the 10-assertion self-audit. Every feature landed as a RED (failing test)
commit followed by a GREEN (implementation) commit.

## Verification

```
cavectl mlx demo --steps 60      # autograd + Adam affine fit, loss -> ~0.006
cave-mlx info                    # capability surface
```
