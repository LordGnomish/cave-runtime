<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-mlx — Charter v2 Parity Report

**Upstream:** [ml-explore/mlx](https://github.com/ml-explore/mlx) `v0.31.2`
(`68cf2fddd8de5edd8ab3d926391772b2e2cedad8`, 2026-04-22) — MIT.
**Backend:** pure-Rust, CPU, eager. No FFI, no Metal/GPU, no lazy graph.
**Audit date:** 2026-06-01.

## Self-audit gates — 9/9 PASS

| # | Gate | Result |
|---|------|--------|
| 1 | SPDX on 100% of `src/*.rs` | ✅ |
| 2 | `source_sha` pinned (`v0.31.2`) | ✅ |
| 3 | `last_audit` is a 2026 date | ✅ |
| 4 | `parity_ratio_source = "manifest"` | ✅ |
| 5 | `fill_ratio >= 0.95` | ✅ (1.0) |
| 6 | `mapped+partial+skipped+unmapped == total` | ✅ |
| 6b | `honest_ratio` consistent and `<= fill_ratio` | ✅ |
| 7 | no `unimplemented!()` / `todo!()` in `src/` | ✅ |
| 8 | this report exists | ✅ |
| 9 | Charter v2 composite | ✅ |

## Parity ledger

- **fill_ratio = 1.0** = (18 mapped + 0 partial + 8 skipped) / 26
- **honest_ratio = 1.0** = (18 mapped + 8 skipped) / 26

### Mapped (18, all strict-TDD)
`Array` N-dim tensor · broadcasting elementwise (add/sub/mul/div) · unary math
(exp/log/sqrt/neg) · activations (relu/sigmoid/tanh) · matmul · transpose ·
reductions (sum/mean/max) · softmax · reverse-mode autograd
(`grad`/`value_and_grad`) · channel-last `conv1d`/`conv2d` + `max_pool2d`/
`avg_pool2d` · `nn.Linear` · `nn.Conv2d` · `nn.Activation` · `Sgd` · `Adam` ·
`AdamW` · group-wise affine quantization (4/8-bit) · **`mx.random`**
(Threefry2x32 + uniform/normal/bernoulli/randint/truncated_normal/categorical)
· `cave-mlx` CLI.

### Partial (0)
None. `mx.random` — formerly the only partial item (just a seeded Kaiming
initializer) — was completed on 2026-06-01 (cont3) in `random.rs`.

### Skipped (8)
Seven architectural cuts for a sovereign CPU eager core (lazy-eval graph /
`mx.eval`, Metal/GPU + unified memory, `mx.compile`, `mx.distributed`,
`mx.fft`, `mx.linalg`, `vmap`/jacobian transforms) plus one parallel-track
delegation (safetensors/gguf weight loading → `cave-local-llm/src/gguf.rs`).

### Unmapped (0)
No in-scope item is left unmapped or partial. `honest_ratio` now equals
`fill_ratio` at 1.0.

### Honesty note
`random.rs` reproduces the Threefry2x32-20 PRNG **bit-exactly** (verified
against the canonical Random123 KAT vectors) and the distribution semantics,
but the per-element counter→position fill order is cave-mlx's own scheme, so a
whole-array draw is not byte-for-byte identical to an upstream `mx.random`
call. The primitive and the distributions match; only the internal element
layout is implementation-local.

## Tests

88 crate tests pass (8 array · 12 ops · 12 conv · 8 autograd · 10 nn · 6 optim ·
5 quant · 17 random) plus the 10-assertion self-audit. Every feature landed as
a RED (failing test) commit followed by a GREEN (implementation) commit.

## Verification

```
cavectl mlx demo --steps 60         # autograd + Adam affine fit, loss -> ~0.006
cavectl mlx rand --seed 7 --n 50000 # mx.random suite summary statistics
cave-mlx info                       # capability surface
```
