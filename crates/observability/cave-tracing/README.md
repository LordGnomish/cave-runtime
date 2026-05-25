# cave-tracing

Sovereign OpenTelemetry-compatible tracing SDK for cave-runtime.

## Status

This crate is currently in a pre-OSS-launch phase. Feature parity with the upstream OpenTelemetry Rust SDK is actively tracked and implemented incrementally.

## Upstream

- [OpenTelemetry Rust](https://github.com/open-telemetry/opentelemetry-rust)

## Surface ported

- Sovereign span SDK implementation compatible with OTel standards.
- High-performance batching mechanism for efficient span export.
- Configurable sampling strategies including trace_id and parent-based sampling.
- Native OTLP exporter supporting both gRPC and HTTP protocols.
- W3C Trace Context propagation for distributed tracing interoperability.
- Multi-tenant isolation ensuring trace data separation across contexts.
- Zero-allocation hot paths for critical section instrumentation.
- Configurable resource attributes for cloud-native metadata injection.
- Graceful shutdown handling for span flush and exporter termination.
- Thread-safe context propagation for async Rust environments.

## Public API

- `TracerProvider`: Top-level entry point for creating tracers and managing spans.
- `Span`: Represents an active trace span with context and attribute management.
- `Tracer`: Interface for starting spans and recording events.
- `Exporter`: Trait for custom OTLP or stdout exporters.
- `Config`: Builder for configuring sampling, batching, and resource attributes.

## Tests

Comprehensive unit and integration tests cover sampling logic, batching delays, and OTLP serialization. Propagation correctness is verified against W3C spec examples.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-config](../cave-config)
- [../cave-metrics](../cave-metrics)
