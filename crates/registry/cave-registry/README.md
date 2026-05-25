# cave-registry

Container registry implementation compatible with Harbor API standards.

## Status

This crate is currently in a pre-OSS-launch phase, with feature parity actively tracked against the upstream Harbor specification. Development focuses on stabilizing the core artifact storage and retrieval mechanisms before public release.

## Upstream

- [Harbor API Specification](https://github.com/goharbor/harbor)

## Surface ported

- Harbor-compatible REST API endpoints for image management.
- Secure authentication and token-based access control.
- Blob storage integration with local and remote backends.
- Image manifest parsing and validation logic.
- Tag management and lifecycle hooks for container images.
- Metadata indexing for efficient search and retrieval.
- Support for multi-architecture image manifests.
- Audit logging for all registry operations.
- Configuration-driven storage backend selection.
- Error handling consistent with OCI distribution spec.

## Public API

- `Registry::new` initializes the registry instance with configuration.
- `Registry::serve` starts the HTTP server on the specified port.
- `ArtifactStore::put_blob` uploads binary data to the registry.
- `ArtifactStore::get_manifest` retrieves image manifest by reference.
- `Registry::authenticate` handles user authentication flows.
- `Registry::list_tags` returns available tags for a given repository.

## Tests

Comprehensive unit tests cover all public API endpoints and edge cases in manifest parsing. Integration tests verify end-to-end workflows with mock storage backends to ensure data integrity.

## License

Apache-2.0

## See also

- [../cave-artifacts](../cave-artifacts)
- [../cave-runtime](../cave-runtime)
- [../cave-config](../cave-config)
