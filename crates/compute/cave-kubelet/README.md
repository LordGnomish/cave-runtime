# cave-kubelet

Node agent for cave-runtime that watches the API server for pod assignments and manages container lifecycle via cave-cri.

## Status

This crate is currently in pre-open-source-launch phase. Feature parity with standard Kubernetes kubelet is tracked via internal issue trackers and is not yet complete.

## Upstream

- [kubernetes/kubelet](https://github.com/kubernetes/kubernetes/tree/master/pkg/kubelet)

## Surface ported

- Watches the API server for pod assignment events using long-polling or WebSocket streams.
- Parses standard Kubernetes PodSpec definitions to determine required container configurations.
- Delegates container creation, start, stop, and deletion to the cave-cri interface.
- Monitors container health and reports status back to the API server.
- Manages volume mounts and environment variable injection as specified in pod specs.
- Handles node status updates including resource utilization and network connectivity.
- Supports basic taints and tolerations to determine schedulability on the node.
- Implements graceful shutdown procedures to ensure clean container termination.
- Logs detailed lifecycle events for debugging and auditing purposes.
- Integrates with the cave-runtime network plugin for pod networking setup.

## Public API

- `Kubelet::new` initializes the agent with configuration and CRI client.
- `Kubelet::run` starts the main event loop for watching and managing pods.
- `Kubelet::sync_node_status` updates the node object in the API server.
- `Kubelet::handle_pod_add` processes new pod assignment events.
- `Kubelet::handle_pod_delete` processes pod deletion events.
- `Kubelet::get_container_status` retrieves current status of managed containers.

## Tests

Unit tests cover configuration parsing and basic lifecycle state transitions. Integration tests require a running cave-cri instance and API server mock to verify end-to-end pod management.

## License

Apache-2.0

## See also

- [../cave-cri](../cave-cri)
- [../cave-api](../cave-api)
- [../cave-network](../cave-network)
