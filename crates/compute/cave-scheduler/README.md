# cave-scheduler

Pod scheduler — assigns pods to nodes based on resources, affinity, and constraints

## Status

This crate is currently in the pre-open-source-launch phase. Feature parity with the target Cloud OS specifications is actively tracked via internal issue trackers. The scheduler logic is stable but subject to breaking changes as the underlying node abstraction evolves.

## Upstream

- (internal — no external upstream)

## Surface ported

- Core scheduling algorithm implementation for pod-to-node assignment
- Resource capacity tracking for CPU, memory, and ephemeral storage
- Node affinity and anti-affinity rule evaluation engine
- Taint and toleration processing for node selection filtering
- Priority queue management for scheduling pod queues
- Preemption logic for handling resource contention scenarios
- Node health status integration for unschedulable node exclusion
- Resource request validation against node allocatable limits
- Topology spread constraint evaluation for distribution
- Scheduling cycle orchestration and event handling

## Public API

- `Scheduler::new` initializes the scheduler with node provider and configuration
- `Scheduler::schedule` attempts to assign a pending pod to a suitable node
- `Scheduler::unschedule` removes a pod from its assigned node and updates resources
- `Scheduler::get_node_status` retrieves current health and capacity of a node
- `Scheduler::update_node_resources` applies resource changes from node updates
- `Scheduler::list_pending_pods` returns the current queue of unscheduled pods

## Tests

Unit tests cover the core scheduling logic, including affinity rules and resource calculations. Integration tests verify the interaction with the node provider and the correctness of the scheduling cycle. Coverage is maintained for edge cases such as resource exhaustion and node taints.

## License

Apache-2.0

## See also

- [../cave-node](../cave-node)
- [../cave-pod](../cave-pod)
- [../cave-resource](../cave-resource)
