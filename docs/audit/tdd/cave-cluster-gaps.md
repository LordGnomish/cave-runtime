# TDD Coverage Audit — cave-cluster vs kubernetes-sigs/cluster-api @ v1.6.0

- **Cave crate**: `crates/compute/cave-cluster`
- **Upstream**: https://github.com/kubernetes-sigs/cluster-api @ `v1.6.0`
- **Upstream test files**: 314, **upstream `func Test*` symbols**: 911
- **Cave behavioral test fns** (src `#[test]` / `#[tokio::test]`): 74 (+4 generic proptest smoke)
- **Audit date**: 2026-05-30 · src NOT modified, nothing committed

## Scope framing

Upstream cluster-api is the full CAPI project: kubeadm bootstrap providers, controller
reconcilers (KubeadmConfig, KubeadmControlPlane, MachineSet, MachineDeployment, MachineHealthCheck),
API-type fuzzy conversions across `v1alpha3/4`/`v1beta1`, admission webhooks, clusterctl
repository/upgrade/move clients, and topology/ClusterClass machinery. ~870 of the 911 test
symbols exercise CRD-plumbing, controller-runtime reconcile loops, multi-apiVersion conversion,
webhook defaulting/validation, and clusterctl provider tooling — **none of which cave-cluster
implements**. cave-cluster is a Rancher-style cluster *lifecycle manager*: an in-memory
store + pure planning functions + an Axum API, deliberately not a CRD controller.

The honest portable surface is therefore small: a handful of upstream behaviors (version
upgrade-step validation, scale clamping/bounds, health roll-up, IP/CIDR config) have
**direct behavioral analogues** in cave's pure functions. The vast majority is scope-cut.

## Behavior table

| behavior | upstream test | cave impl? | cave test? | gap type | suggested test |
|---|---|---|---|---|---|
| Single-minor-step upgrade validation | `Test_shouldUpgrade`, `Test_getContractsForUpgrade` (clusterctl upgrade) | yes — `version::validate_upgrade` (reject downgrade / same / skip-minor) | yes — `valid_upgrade_path`, `skip_minor_version_fails`, `downgrade_fails`, `same_version_fails` | covered | — |
| Supported / EOL version gate | `TestVersionChecker` family | yes — `version::validate_k8s_version` | yes — `supported_version_ok`, `unsupported_version_fails` | covered | — |
| Scale-down node selection / bounds | `TestSelectMachineForScaleDown`, `scaleDownControlPlane` | yes — `nodepool::NodePool::scale` (clamp to autoscale range, reject <0) + `provisioner::scale_node_pool` (clamp to min/max) | partial — `NodePool::scale` tested (`autoscaling_range_enforced`); **`provisioner::scale_node_pool` clamp untested** | **portable-coverage** | `provisioner::scale_node_pool` — assert desired clamped into `[min,max]` and bounds-override via req |
| Cluster create / dedup / delete lifecycle | controller reconcile create/delete paths | yes — `ClusterStore::{create,get,delete}` + pure `provisioner::{provision_cluster,delete_cluster}` | partial — store tested; **pure `provision_cluster` (status=Provisioning, defaults) + `delete_cluster` event untested** | **portable-coverage** | `provisioner::provision_cluster` — assert status `Provisioning`, network/upgrade defaults applied; `provisioner::delete_cluster` — assert `Deleted` event for id |
| Rolling-upgrade plan / max-unavailable batching | `RolloutStrategy_ScaleUp/ScaleDown` | yes — `upgrade::UpgradePlan::{next_batch,progress_percent}` + `provisioner::upgrade_cluster` | partial — UpgradePlan tested; **pure `upgrade_cluster` (status=Upgrading, version bump) untested** | **portable-coverage** | `provisioner::upgrade_cluster` — assert version set to target + status `Upgrading` |
| Credential / kubeconfig rotation | `adoptKubeconfigSecret`, `reconcileKubeconfig` | yes — `provisioner::rotate_credentials` (vault path, encrypted, timestamp) | no | **portable-coverage** | `provisioner::rotate_credentials` — assert `vault_path == clusters/{id}/kubeconfig`, `encrypted == true` |
| Add-on default-version resolution | n/a (CAPI uses CNI/addon providers) | yes — `provisioner::install_addons` (default version per addon type, status=Installing) | partial — `AddonStore::install` tested; **pure `install_addons` default-version branch untested** | **portable-coverage** | `provisioner::install_addons` — assert default version chosen when req omits it + status `Installing` |
| Cluster health roll-up by phase | `TestClusterCacheHealthCheck`, `TestHasUnhealthyMachine` | yes — `health::check_cluster_health` (Running→Healthy, Upgrading/Scaling→Degraded, Failed→Unhealthy) | yes — `healthy_cluster_check`, `failed_cluster_is_unhealthy` | covered (one gap: Degraded path) | optional: assert Upgrading→`Degraded` roll-up |
| etcd member / backup mgmt | `TestEtcdMembers_*`, `TestCanSafelyRemoveEtcdMember` | yes — `EtcdBackupStore` (CAPI does live etcd quorum; cave does backup/restore) | yes — backup/restore/version tests | covered (different scope) | — |
| Cluster IP family / CIDR config | `TestClusterIPFamily` (v1alpha4 + v1beta1) | partial — `NetworkConfig{pod_cidr,service_cidr}` fields exist; no IP-family detection fn | missing-impl | scope-cut (no dual-stack family resolver) | — |
| Index machine/pool/node by providerID/nodeName | `TestIndexMachineBy*`, `TestIndexNodeByProviderID` | no | missing-impl | scope-cut (controller index, no CRD cache) | — |
| Fuzzy API conversion roundtrip | `TestFuzzyConversion` (×6) | no | n/a | scope-cut (no multi-apiVersion CRD) | — |
| Kubeadm bootstrap token / cloudinit / ignition | `bootstraptokenstring`, `cloudinit`, `ignition` suites | no | n/a | scope-cut (no kubeadm bootstrap provider) | — |
| Webhook defaulting / validation | `KubeadmConfigDefault/Validate`, etc. | no | n/a | scope-cut (no admission webhooks) | — |
| clusterctl repo / move / upgrade clients | `gitHubRepository`, `providerUpgrader`, `topologyClient` | no | n/a | scope-cut (no provider tooling) | — |
| Control-plane init mutex / locking | `TestControlPlaneInitMutex_*` | no | n/a | scope-cut (no distributed CP bootstrap) | — |

## Recommended TDD fills (portable-coverage first)

All gaps are concentrated in `src/provisioner.rs` — the pure planning functions are the only
fully-untested behavioral module, yet they each have a direct upstream behavioral analogue.
Suggested fills, each naming the exact public cave fn exercised:

1. **`provisioner::scale_node_pool`** — desired count is clamped into `[min,max]`; `req.min_nodes`/`req.max_nodes` override pool bounds. (analogue: `TestSelectMachineForScaleDown` / scale-down bounds)
2. **`provisioner::provision_cluster`** — returns `status == Provisioning` with network + upgrade-policy defaults filled from `unwrap_or_default()`. (analogue: reconcile create path)
3. **`provisioner::upgrade_cluster`** — returns clone with `version == target_version` and `status == Upgrading`. (analogue: rollout-strategy upgrade)
4. **`provisioner::rotate_credentials`** — returns `KubeconfigRef` with `vault_path == "clusters/{id}/kubeconfig"`, `encrypted == true`, fresh `last_rotated`. (analogue: kubeconfig adopt/reconcile)
5. **`provisioner::install_addons`** — when `req.version` is `None`, default version is resolved per `ClusterAddonType`; status is `Installing`. (analogue: addon provider defaulting)
6. **`provisioner::delete_cluster`** — emits a `ClusterEvent` with `event_type == Deleted` for the given id. (analogue: reconcile delete path)

Optional hardening (already-tested modules, one branch each):
- `health::check_cluster_health` — assert `Upgrading`/`Scaling` cluster status rolls up to `Degraded` (currently only Running/Failed branches are asserted).

**Honest assessment**: 6 portable-coverage gaps, all in `provisioner.rs`. Everything else
upstream is genuine scope-cut (CRD controllers / conversions / webhooks / kubeadm / clusterctl)
that cave-cluster intentionally does not implement. No padding — the rest of the crate
(cluster, nodepool, version, kubeconfig, health, network, rbac, addons, etcd, upgrade, node,
tenant, tenant_ns, multi_cluster, k8s_distro) already has behavioral tests on its public surface.
