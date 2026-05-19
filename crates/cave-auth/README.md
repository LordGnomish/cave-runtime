# cave-auth

Comprehensive authentication and authorization subsystem for the cave-runtime sovereign Cloud OS.

## Status

This crate is currently in a pre-open-source-launch phase, with feature parity tracked against the internal reference implementation. It provides the foundational identity layer for multi-tenant operations within the cave-runtime ecosystem.

## Upstream

- (internal — no external upstream)

## Surface ported

- Full OIDC (OpenID Connect) compliance for standard authentication flows.
- Role-Based Access Control (RBAC) engine for hierarchical permission management.
- Attribute-Based Access Control (ABAC) engine for fine-grained policy evaluation.
- SCIM 2.0 integration for automated user provisioning and directory synchronization.
- Personal Access Token (PAT) generation, validation, and revocation mechanisms.
- Secure session management with configurable expiration and renewal policies.
- Multi-tenancy support ensuring strict isolation between tenant identities.
- Integration with Okta for enterprise-grade identity provider connectivity.
- Secure storage of credentials and secrets within the cave-runtime stack.
- Extensible middleware for intercepting and validating incoming authentication requests.

## Public API

- `AuthContext`: The primary struct holding tenant, user, and session state for request processing.
- `authenticate()`: Top-level function to validate credentials and issue tokens or sessions.
- `authorize()`: Function to check if a specific action is permitted under current RBAC/ABAC rules.
- `ScimClient`: Struct for interacting with external SCIM 2.0 compliant directories.
- `TokenManager`: Struct handling the lifecycle of access and refresh tokens.
- `TenantResolver`: Utility for identifying and isolating tenant contexts from incoming requests.

## Tests

The crate includes comprehensive unit tests covering all authentication flows and authorization policies. Integration tests verify SCIM synchronization and OIDC token validation against mock providers.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-network](../cave-network)
- [../cave-storage](../cave-storage)
