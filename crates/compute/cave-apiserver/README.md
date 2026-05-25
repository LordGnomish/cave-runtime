# cave-apiserver

Kubernetes-compatible API server for resource CRUD, watch, admission, and RBAC.

## Status

This crate is currently in a pre-open-source-launch phase. Feature parity with upstream Kubernetes API specifications is actively tracked and implemented incrementally.

## Upstream

- [kubernetes-api](https://github.com/kubernetes/api)

## Surface ported

- Standard RESTful HTTP endpoints for all core Kubernetes resource types.
- Support for list, get, create, update, patch, and delete operations.
- Real-time watch streams using long-polling and WebSocket protocols.
- Admission webhook integration for validating and mutating incoming requests.
- Role-Based Access Control (RBAC) enforcement at the API level.
- Resource versioning and optimistic concurrency control mechanisms.
- Pagination support for large list responses via continue tokens.
- Schema validation against OpenAPI specifications for all resource types.
- Namespace scoping and resource quota enforcement logic.
- Event broadcasting for significant resource state changes.

## Public API

- `ApiServer::new` initializes the server with configuration and storage backend.
- `ApiServer::serve` starts the HTTP listener on the configured port.
- `ResourceHandler` trait defines the interface for custom resource implementations.
- `AdmissionController` trait allows injection of custom validation logic.
- `RBACPolicy` struct represents the access control rules applied to requests.
- `WatchStream` provides an async iterator for real-time resource updates.

## Tests

Comprehensive unit tests cover individual handler logic and RBAC evaluation. Integration tests verify end-to-end CRUD operations against a mock storage backend.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-storage](../cave-storage)
- [../cave-rbac](../cave-rbac)
