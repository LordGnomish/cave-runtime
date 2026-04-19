# Security Policy

## Supported Versions

| Version | Supported          |
| ------- | ------------------ |
| 0.x     | :white_check_mark: |

## Reporting a Vulnerability

If you discover a security vulnerability in CAVE Runtime, please report it responsibly.

**Do NOT open a public GitHub issue for security vulnerabilities.**

Instead, please email: **security@cave-runtime.dev**

You will receive a response within 48 hours acknowledging receipt.

### Scope

CAVE Runtime is a platform runtime with security-critical components:

- **Authentication bypass**: JWT validation flaws, middleware ordering issues
- **Authorization escalation**: RBAC policy bypass, cross-tenant data access
- **Secret exposure**: Vault secrets in logs, API responses, or error messages
- **Wire protocol injection**: MongoDB/PostgreSQL wire protocol parsing vulnerabilities
- **Container registry poisoning**: Malicious image acceptance via proxy pipeline
- **Supply chain**: Compromised dependencies, YARA rule bypass

### Out of Scope

- Vulnerabilities in upstream OSS tools that CAVE reimplements (report to upstream)
- Configuration errors in deployment (covered by operational runbooks)
