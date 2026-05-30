<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# cave-mlx

Pure-Rust, dependency-light port of the **array-programming core** of
[ml-explore/mlx](https://github.com/ml-explore/mlx) (MIT) onto a
cross-platform CPU backend.

## Scope

| Layer | Status |
|-------|--------|
| `Array` — N-dim dense tensor (shape/strides/contiguous `f32`) | ✅ |
| Elementwise + broadcasting + reductions + matmul + activations | ✅ |
| Reverse-mode automatic differentiation (`autograd`) | ✅ |
| `nn` modules (`Linear`, activations) | ✅ |
| Optimizers (`Sgd`, `Adam`, `AdamW`) | ✅ |

MLX's lazy graph, unified-memory model, and Metal/GPU kernels are **not**
ported — cave-mlx is an eager, sovereign, CPU-only implementation. See
`parity.manifest.toml` for the honest mapped/skipped breakdown and
`PARITY_REPORT.md` for the Charter v2 self-audit.

## CLI

```
cave-mlx version
```

## License

AGPL-3.0-or-later. Upstream MLX is MIT — see the repository-root `NOTICE`.
