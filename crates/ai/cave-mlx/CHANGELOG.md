<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Changelog — cave-mlx

All notable changes to this crate are documented here.

## [Unreleased]

### Added
- Initial strict-TDD port of the ml-explore/mlx (v0.31.2) array core onto a
  pure-Rust CPU backend: `Array` N-dim tensor, elementwise/broadcast/reduction
  ops, matmul, activations, reverse-mode autograd, `nn` modules, and
  `Sgd`/`Adam`/`AdamW` optimizers.
- `cave-mlx` CLI binary.
