# cave-portal-api

HTTP API layer for the Cave Portal, handling attribution, telemetry, and dashboard endpoints.

## Status

This crate is currently in a pre-open-source-launch phase. Feature parity with the proprietary upstream implementation is actively tracked and prioritized for the upcoming release cycle.

## Upstream

- (internal — no external upstream)

## Surface ported

- RESTful HTTP endpoint definitions for portal interactions.
- Attribution tracking middleware for request origin identification.
- Telemetry data ingestion and validation pipelines.
- Dashboard data aggregation and serialization logic.
- Authentication and authorization middleware integration.
- Request validation using serde and custom validators.
- Error handling with standardized HTTP error responses.
- JSON response formatting and content negotiation.
- Health check endpoints for service monitoring.
- Rate limiting configuration for API access control.

## Public API

- `PortalApi`: The main struct exposing all HTTP route handlers.
- `AttributionRequest`: Struct representing incoming attribution payloads.
- `TelemetryEvent`: Struct for structured telemetry data submission.
- `DashboardResponse`: Struct for aggregated dashboard data output.
- `PortalError`: Enum defining all possible API error states.
- `setup_routes`: Function to register all API routes with a router.

## Tests

Unit tests cover serialization, deserialization, and validation logic for all public structs. Integration tests verify endpoint behavior against a mock server, ensuring correct status codes and response bodies.

## License

Apache-2.0

## See also

- [cave-runtime](../cave-runtime)
- [cave-portal-core](../cave-portal-core)
- [cave-telemetry](../cave-telemetry)
