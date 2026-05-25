# cave-vault

Helm chart for **cave-vault** — port of OpenBao v2.5.4.

Workload kind: `statefulset`. Default service port: `8200`.

## Install

```bash
helm install my-vault ./helm/charts/cave-vault \
  --namespace cave-system --create-namespace
```

## Values

| Key | Default | Notes |
| --- | --- | --- |
| `image.repository` | `ghcr.io/lordgnomish/cave-vault` | OCI image (placeholder until release pipeline lands) |
| `image.tag` | `0.1.0` | matches chart `appVersion` |
| `replicaCount` | `1` | ignored for DaemonSet |
| `service.port` | `8200` | upstream-aligned default |
| `serviceAccount.create` | `true` | named via `cave.serviceAccountName` |
| `podSecurityContext.runAsNonRoot` | `true` | hardened by default |
| `securityContext.readOnlyRootFilesystem` | `true` | tmp + config mounted as emptyDir/configMap |
| `autoscaling.enabled` | `false` | HPA scaffold present, opt-in |
| `persistence.enabled` | `true` (StatefulSet only) | `10Gi` default |
| `podDisruptionBudget.enabled` | `false` | enable for HA |
| `networkPolicy.enabled` | `false` | default-deny with allowlist skeleton |

See [`values.yaml`](./values.yaml) for full schema.

## Upgrade

```bash
helm upgrade my-vault ./helm/charts/cave-vault \
  --namespace cave-system --reuse-values
```

## Test

```bash
helm test my-vault --namespace cave-system
```

## Notes

This chart is a scaffold — `image.repository` points to a placeholder OCI ref
that will be populated once the cave-runtime release pipeline publishes container
images. Update via `--set image.repository=...` for local clusters.

For real secret material, prefer External Secrets Operator (`cave-secrets`) or
Sealed Secrets over the `secret.yaml` placeholder shipped here.
