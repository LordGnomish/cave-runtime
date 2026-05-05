# cave-cli

The official command-line interface for the cave-runtime sovereign Cloud OS, providing native and compatibility surfaces for system management.

## Status

This crate is currently in a pre-open-source-launch phase. Feature parity with the broader cave-runtime ecosystem is actively tracked and verified against the ADR-RUNTIME-CLI-CONSOLIDATION-001 specification to ensure consistency across all interaction surfaces.

## Upstream

- [cave-runtime](https://github.com/cave-os/cave-runtime)

## Surface ported

- **System Initialization**: Handles the boot sequence and initialization of core runtime services, ensuring the sovereign OS environment is correctly configured before user interaction.
- **Service Management**: Provides commands to start, stop, restart, and inspect the status of individual runtime services and daemons within the Hetzner OSS stack.
- **Configuration Management**: Allows users to view, edit, and validate the system configuration files that drive the cave-runtime behavior, ensuring atomic updates and rollback capabilities.
- **Log Inspection**: Offers real-time log streaming and historical log retrieval for debugging and monitoring system health, integrating with the central logging infrastructure.
- **User and Permission Control**: Manages user accounts, roles, and access control lists, enforcing the security policies defined in the sovereign OS architecture.
- **Network Configuration**: Interfaces with the underlying Linux network stack to configure interfaces, routes, and firewall rules as required by the runtime environment.
- **Package Management**: Integrates with the local package repository to install, update, and remove software components that extend the base cave-runtime functionality.
- **Diagnostic Tools**: Includes utilities for gathering system diagnostics, including CPU, memory, and disk usage metrics, to assist in troubleshooting performance issues.

## Public API

- `run()`: The primary entry point that parses command-line arguments and dispatches to the appropriate subcommand handler.
- `Cli`: The struct defining the top-level command-line interface structure, including all subcommands and global options.
- `CommandError`: The error type returned by CLI operations, providing detailed context for failures in execution or configuration.
- `ConfigLoader`: A utility struct responsible for loading and validating the runtime configuration from standard paths.
- `ServiceManager`: An abstraction over the service lifecycle, allowing the CLI to interact with systemd or custom runtime managers.

## Tests

The crate includes comprehensive unit tests for argument parsing, configuration validation, and error handling. Integration tests verify the interaction with the runtime daemon, ensuring that CLI commands correctly trigger the expected system state changes.

## License

This project is licensed under the Apache License, Version 2.0. You may not use this file except in compliance with the License. A copy of the license is available in the root directory of the workspace.

## See also

- [cave-runtime](../cave-runtime)
- [cave-core](../cave-core)
- [cave-config](../cave-config)
