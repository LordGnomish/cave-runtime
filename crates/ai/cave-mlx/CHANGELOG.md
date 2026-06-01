<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Changelog — cave-mlx

All notable changes to this crate are documented here.

## [Unreleased]

### Added
- Initial strict-TDD port of the ml-explore/mlx (v0.31.2) array core onto a
  pure-Rust CPU backend: `Array` N-dim tensor, elementwise/broadcast/reduction
  ops, matmul, activations, reverse-mode autograd, `nn` modules, and
  `Sgd`/`Adam`/`AdamW` optimizers.
- `mx.random` distribution suite (`random.rs`): a KAT-verified Threefry2x32-20
  counter PRNG with `Key`/`split`, plus `uniform` / `normal` (Box-Muller) /
  `bernoulli` / `randint` / `truncated_normal` (inverse-CDF) / `categorical`
  (Gumbel-max). Closes the last partial parity item — honest_ratio 0.9615 → 1.0.
- `cave-mlx rand` / `cavectl mlx rand` subcommand for sampling the suite.
- `cave-mlx` CLI binary.
