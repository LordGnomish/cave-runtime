# cave-controller-manager

Kubernetes controller-manager parity scaffold for sovereign Cloud OS operations.

## Status

This crate is currently in a pre-OSS-launch phase, focusing on establishing foundational parity with standard Kubernetes controller behaviors. Feature completeness is tracked against the upstream Kubernetes controller-manager specification, with active development prioritizing core workload controllers and service discovery mechanisms.

## Upstream

- [kubernetes/controller-manager](https://github.com/kubernetes/kubernetes/tree/master/pkg/controller)

## Surface ported

- Deployment controller for managing replica sets and rollout strategies.
- ReplicaSet controller ensuring desired state for pod groups.
- StatefulSet controller handling ordered deployment and scaling.
- DaemonSet controller ensuring pod execution on all or selected nodes.
- Job controller for executing finite tasks to completion.
- CronJob controller for scheduling periodic Job executions.
- HorizontalPodAutoscaler controller for dynamic resource scaling.
- PodDisruptionBudget controller enforcing availability constraints.
- EndpointSlice controller for efficient service endpoint discovery.
- Service controller managing network policies and load balancing.

## Public API

- `ControllerManager`: The main entry point for initializing and running the controller loop.
- `NewControllerManager`: Constructor function for setting up controller dependencies.
- `Run`: Method to start the controller manager and block until shutdown.
- `ControllerConfig`: Configuration struct for tuning controller behavior.
- `EventBroadcaster`: Interface for emitting and broadcasting Kubernetes events.
- `SharedInformerFactory`: Factory for creating shared informers for resource caching.

## Tests

Unit tests cover individual controller logic and reconciliation loops extensively. Integration tests validate end-to-end workflows against a simulated Kubernetes API server, ensuring correct state transitions and error handling.

## License

Apache-2.0

## See also

- [../cave-runtime](../cave-runtime)
- [../cave-api](../cave-api)
- [../cave-scheduler](../cave-scheduler)
