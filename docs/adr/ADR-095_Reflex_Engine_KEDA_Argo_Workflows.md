# ADR-095: Reflex Engine (KEDA + Argo Workflows)

**Status:** Accepted

**Scope:** Universal

**Category:** Operations

**Related ADRs:** 119

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## Simple remediation handled by Crossplane Operations (ADR-119). But complex multi-step, cross-service remediation (coordinated rollouts, data migration, multi-resource scaling) needs workflow orchestration.

## Candidates

## | Approach | Reflex Engine (KEDA + Argo Workflows) | Custom controllers | Ansible Tower | Manual runbooks |
|---|---|---|---|---|
| Event-driven triggers | ✅ KEDA (Prometheus, Kafka, cron) | ❌ Custom | ⚠️ | ❌ |
| Workflow DAG | ✅ Argo Workflows (steps, DAG, retries) | ❌ Custom | ✅ | ❌ |
| K8s native | ✅ CRDs | ⚠️ | ❌ | ❌ |
| Signed playbooks | ✅ cosign-signed workflow templates | ❌ | ❌ | ❌ |

## Decision

## Prometheus alerts → KEDA event-driven triggers → Argo Workflows executes pre-approved signed playbooks. `Self-Healed` attestation in Sovereign Ledger. Human approval required for actions exceeding cost threshold or touching security controls. Playbooks are Tier B constitutional artifacts.

## Rejected

## - **Custom controllers:** Build cost too high. Argo Workflows provides workflow DAG, retry logic, artifact passing, timeout handling.
- **Ansible Tower:** Not K8s-native. Separate infrastructure. Doesn't integrate with Prometheus/KEDA event model.
- **Manual runbooks only:** Too slow for automated remediation. Mean time to recovery unacceptable for 99.5% SLA.

## Consequences

## **Positive:**
- Complex remediation automated with DAG-based workflows.
- Event-driven via KEDA — no polling, no custom trigger infrastructure.
- Signed playbooks ensure only approved remediation actions execute.
- `Self-Healed` attestations provide compliance evidence.

**Negative:**
- Argo Workflows adds operational complexity (workflow templates, RBAC, artifact storage).
- KEDA trigger misconfiguration can cause remediation storms (mitigated: APOL rate limiting, conflict serializer).
- Playbook signing maintenance overhead.

## Compliance Mapping

## SOC2 CC7.2 (automated incident response). ISO A.5.26 (response to information security incidents). NIS2 Art.21 (incident response automation).
