# Contributing to CAVE Platform Runtime

Thank you for your interest in contributing to CAVE! This document provides guidelines and instructions for submitting changes.

## Code of Conduct

We are committed to providing a welcoming and inspiring community for all. Please read and adhere to our Code of Conduct in all interactions.

## Getting Started

### Prerequisites

- Rust 1.85 or later ([install](https://rustup.rs/))
- PostgreSQL 14+ (for local development)
- Git

### Fork and Clone

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/YOUR_USERNAME/cave-runtime.git
   cd cave-runtime
   ```
3. Add upstream remote:
   ```bash
   git remote add upstream https://github.com/cave-runtime/cave-runtime.git
   ```

## Development Workflow

### Create a Feature Branch

```bash
git checkout -b feature/my-feature
```

Branch naming conventions:
- `feature/` for new features
- `fix/` for bug fixes
- `docs/` for documentation improvements
- `perf/` for performance improvements
- `refactor/` for code refactoring
- `test/` for test additions

### Make Changes

- Keep commits atomic and focused
- Write clear commit messages (see [Commit Message Guidelines](#commit-message-guidelines))
- Update documentation as you go
- Add tests for new functionality

### Commit Message Guidelines

Follow the Conventional Commits format:

```
type(scope): subject

body

footer
```

**Types:**
- `feat`: A new feature
- `fix`: A bug fix
- `docs`: Documentation changes
- `test`: Test additions or updates
- `refactor`: Code refactoring
- `perf`: Performance improvements
- `chore`: Build, CI, or tooling changes
- `security`: Security fixes

**Examples:**
```
feat(auth): add JWT token refresh mechanism

Implement automatic token refresh for better UX.
Fixes #123

BREAKING CHANGE: removed legacy auth endpoint
```

```
fix(metrics): correct prometheus gauge initialization

The gauge was initialized with wrong initial value,
causing metric misalignment on startup.
```

### Testing

All changes must include appropriate tests.

```bash
# Run all tests
CAVE_JWT_SECRET=dev-secret cargo test --workspace

# Run tests for specific crate
CAVE_JWT_SECRET=dev-secret cargo test -p cave-auth

# Run tests with logging
RUST_LOG=debug CAVE_JWT_SECRET=dev-secret cargo test -- --nocapture
```

### Code Quality

Ensure code quality before submitting:

```bash
# Format code
cargo fmt --all

# Check formatting
cargo fmt --check --all

# Lint with clippy
cargo clippy --workspace -- -D warnings

# Build release
CAVE_JWT_SECRET=dev-secret cargo build --release --workspace
```

## Submitting Changes

### Create a Pull Request

1. Push your branch to your fork:
   ```bash
   git push origin feature/my-feature
   ```

2. Go to GitHub and create a Pull Request against the main repository

3. Fill in the PR template with:
   - Clear description of changes
   - Related issue (if any)
   - Type of change (feature, fix, docs, etc.)
   - Testing instructions
   - Checklist completion

### PR Requirements

Before submitting a PR, ensure:

- [ ] Code follows the style guidelines (run `cargo fmt`)
- [ ] No clippy warnings (`cargo clippy -- -D warnings`)
- [ ] Tests pass locally (`CAVE_JWT_SECRET=dev-secret cargo test --workspace`)
- [ ] Documentation is updated
- [ ] Commit messages follow conventions
- [ ] No hardcoded secrets, credentials, or personal information
- [ ] Changes are focused and atomic

### Review Process

- Maintainers will review your PR
- Address feedback and push updates to the same branch
- Approvals from at least one maintainer required
- CI must pass before merge

## Architecture & Code Organization

### Crate Structure

- Each module is a separate crate under `crates/`
- Crates are organized by functional area
- Cross-crate dependencies should be minimal
- Use workspace dependency definitions in root `Cargo.toml`

### Module Anatomy

A typical module includes:

```
crates/cave-{name}/
├── Cargo.toml
├── src/
│   ├── lib.rs          # Main library
│   ├── error.rs        # Error types
│   ├── api.rs          # HTTP/gRPC endpoints
│   └── ...
└── tests/
    └── integration.rs
```

### Documentation

- Add doc comments to public APIs (`///` style)
- Update README.md for user-facing changes
- Keep ARCHITECTURE.md in sync with major changes
- Document trade-offs in ADRs when applicable

## Security & Secrets

**IMPORTANT:** Never commit secrets, API keys, or personal information.

- Use environment variables for sensitive data
- Don't commit `.env` files
- Don't hardcode passwords or tokens
- Use `CAVE_JWT_SECRET` for auth configuration
- Enable pre-commit hooks to catch secrets:
  ```bash
  git config core.hooksPath .githooks
  ```

## Performance & Benchmarks

For performance-sensitive changes:

1. Include before/after benchmarks
2. Run benchmarks multiple times for stability
3. Document why changes improve performance
4. Avoid premature optimization

```bash
cargo bench --package cave-core
```

## Documentation

- All public APIs must have doc comments
- Examples should be runnable
- Keep docs in sync with code
- Use markdown for guides and tutorials

## Reporting Issues

When reporting bugs:

1. Use the GitHub issue template
2. Describe the expected vs actual behavior
3. Provide steps to reproduce
4. Include relevant logs or error messages
5. Specify environment (OS, Rust version, etc.)
6. Do NOT include credentials or sensitive data

## Release Process

Maintainers follow semantic versioning:
- `MAJOR.MINOR.PATCH` (e.g., `1.2.3`)
- `MAJOR`: Breaking changes
- `MINOR`: New features (backward compatible)
- `PATCH`: Bug fixes

Release notes summarize all changes grouped by type.

## Questions?

- Check the [README.md](README.md) and existing docs
- Review [ARCHITECTURE-ELASTIC-SCALE.md](ARCHITECTURE-ELASTIC-SCALE.md)
- Ask in GitHub issues or discussions
- Check existing PRs/issues for similar topics

## License

By contributing, you agree that your contributions will be licensed under the Apache License, Version 2.0.

---

**Thank you for contributing to CAVE Platform Runtime!**
