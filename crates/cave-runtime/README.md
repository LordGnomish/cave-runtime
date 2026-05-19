# cave-runtime

Unified runtime binary hosting all platform modules for the sovereign Cloud OS.

## Status

This crate is currently in a pre-open-source-launch phase, with feature parity tracked against the broader cave-runtime workspace. It serves as the central execution environment for the cave-runtime stack on Linux 7.1, ensuring consistent behavior across all distributed components. Development focuses on stability and modular isolation rather than public API stability.

## Upstream

- (internal — no external upstream)

## Surface ported

- Unified binary entry point for all CAVE platform modules.
- Dynamic loading of platform-specific runtime plugins.
- Configuration parsing for multi-node cluster environments.
- Signal handling and graceful shutdown mechanisms.
- Logging initialization with structured output support.
- Resource isolation via cgroup integration on Linux.
- Health check endpoint registration for orchestration tools.
- Version reporting and diagnostic information aggregation.
- Plugin lifecycle management (load, init, run, unload).
- Error handling and panic recovery strategies.

## Public API

- `main()`: The entry point that initializes the runtime environment.
- `Runtime::new()`: Constructs a new runtime instance with default settings.
- `Runtime::load_plugin()`: Dynamically loads a specified platform module.
- `Runtime::start()`: Begins execution of all loaded modules concurrently.
- `Runtime::stop()`: Initiates a graceful shutdown of all active processes.
- `Config::from_file()`: Parses configuration from a TOML or JSON file.

## Tests

The test suite covers core initialization paths, configuration parsing, and plugin loading logic. Integration tests verify the interaction between the runtime and the underlying Linux 7.1 kernel features. Coverage is prioritized for error handling and edge cases in configuration validation.

## License

Apache-2.0

## See also

- [cave-core](../cave-core)
- [cave-network](../cave-network)
- [cave-storage](../cave-storage)
