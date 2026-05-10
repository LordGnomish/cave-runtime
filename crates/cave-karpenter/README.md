# cave-karpenter

CAVE Karpenter — node autoscaler reimplementation (scaffold; impl pending).

## Status

This crate is currently in the pre-open-source-launch phase, with implementation details marked as scaffolding. Full feature parity with the upstream Karpenter controller is tracked via internal issue boards and is not yet available for public consumption.

## Upstream

- [Karpenter](https://github.com/aws/karpenter) (internal — no external upstream)

## Surface ported

- Node provisioning logic scaffolding for Hetzner Cloud infrastructure.
- Integration points for the CAVE runtime's sovereign Cloud OS layer.
- Support for Linux 7.1 kernel compatibility checks.
- Basic configuration parsing for cluster scaling policies.
- Interface definitions for node pool management.
- Stub implementations for instance type filtering.
- Placeholder logic for spot vs. on-demand instance selection.
- Integration hooks for the CAVE control plane API.
- Basic logging and telemetry structures for autoscaling events.
- Error handling frameworks for provisioning failures.

## Public API

- `pub struct KarpenterController` — Main controller struct for managing node lifecycle.
- `pub fn new(config: &Config) -> Self` — Constructor for initializing the controller.
- `pub async fn run(&self, ctx: Context) -> Result<(), Error>` — Entry point for the autoscaling loop.
- `pub struct NodePool` — Configuration structure for defining node groups.
- `pub fn provision_node(&self, req: ProvisionRequest) -> impl Future<Output = Result<Node, Error>>` — Stub for node creation.
- `pub fn terminate_node(&self, node_id: &str) -> impl Future<Output = Result<(), Error>>` — Stub for node termination.

## Tests

Test coverage is currently minimal, focusing on unit tests for configuration parsing and basic struct initialization. Integration tests are pending the implementation of the core provisioning logic and will be added once the scaffold is replaced with functional code.

## License

Apache-2.0

## See also

- [cave-runtime](../cave-runtime)
- [cave-hetzner](../cave-hetzner)
- [cave-control-plane](../cave-control-plane)
