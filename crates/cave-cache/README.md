# cave-cache

A high-performance, Redis-compatible cache layer built in Rust for the cave-runtime ecosystem.

## Status

This crate is currently in the pre-open-source-launch phase. Full Redis protocol parity is actively tracked and implemented incrementally.

## Upstream

- [Redis Protocol Specification](https://redis.io/docs/reference/protocol-spec/)

## Surface ported

- Full RESP3 protocol support for bidirectional communication.
- Support for all core Redis data types including strings, lists, sets, sorted sets, and hashes.
- Pub/Sub messaging system with pattern matching capabilities.
- Lua scripting engine integration for atomic server-side execution.
- Transactional support via MULTI, EXEC, and DISCARD commands.
- Persistence mechanisms including RDB snapshots and AOF logs.
- Client-side connection pooling and keep-alive management.
- Pub/Sub sharding for distributed cache scenarios.
- Memory usage optimization with eviction policies.
- Command batching for reduced network latency.

## Public API

- `Cache::new`: Initializes a new cache instance with default configuration.
- `Cache::get`: Retrieves a value by key with optional expiration handling.
- `Cache::set`: Sets a key-value pair with optional TTL and flags.
- `Cache::subscribe`: Subscribes to a channel for pub/sub messaging.
- `Cache::eval`: Executes a Lua script within the cache context.
- `Cache::transaction`: Begins a multi-command transaction block.

## Tests

Comprehensive test suites cover protocol parsing, data structure integrity, and concurrency safety. Integration tests verify RESP3 compliance against reference implementations.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-net](../cave-net)
- [../cave-store](../cave-store)
