# ADR-061: OpenTofu for Day 0 Infrastructure Provisioning

**Status:** Accepted

**Scope:** Universal (Hetzner + Azure)

**Category:** Infrastructure / IaC

**Related ADRs:** 067, 108, 119

## Context

CAVE requires Infrastructure-as-Code for Day 0 provisioning (one-time, not reconciled):

- **Hetzner:** VPC setup, Talos Linux cluster creation, DNS configuration
- **Azure:** Virtual Network, AKS cluster creation, resource groups, managed identity setup
- **Common:** S3/Blob storage buckets, SSL certificates (route53/Azure DNS), firewall rules

Requirements:
- **Cloud-agnostic:** Same HCL code with provider-specific variables (Hetzner vs. Azure)
- **State management:** Secure state storage + remote locks (prevent concurrent applies)
- **Plan/Apply model:** Review changes before apply. Git-driven approval workflows.
- **Diff visibility:** Easy to review infrastructure changes in PR (similar to application code reviews)

Day 0 IaC differs from Day 1+ (Crossplane, ADR-067). Day 0 = one-shot cluster setup. Day 1+ = continuous reconciliation of application infrastructure.

## Candidates

| Criteria | OpenTofu | Terraform | Pulumi | Crossplane |
|---|---|---|---|---|
| **Cloud-agnostic** | ✅ HCL + multi-provider | ✅ HCL + multi-provider | ✅ Multi-language + multi-provider | ✅ XRDs abstract providers |
| **License** | ✅ MPL 2.0 (OpenTofu post-split) | ⚠️ Proprietary (Terraform Cloud SaaS features) | Proprietary | Apache 2.0 |
| **Hetzner support** | ✅ Community provider (active) | ✅ Community provider | ⚠️ Limited | ✅ XRD provider |
| **Azure support** | ✅ Official AzureRM provider | ✅ Official AzureRM provider | ✅ | ✅ XRD provider |
| **State management** | ✅ Remote state (S3 + locks) | ✅ State backend abstraction | ✅ Pulumi backend | ❌ No state file (K8s CRD is state) |
| **Plan/Apply workflow** | ✅ Plan → Review → Apply (human-in-loop) | ✅ Same | ✅ Preview/up workflow | ❌ Continuous reconciliation |
| **Community** | Growing (Linux Foundation post-HashiCorp BSL split) | Very large (de facto standard) | Large (Pulumi Inc) | CNCF (Crossplane) |
| **Day 0 vs Day 1** | ✅ Designed for one-shot provisioning | ✅ Designed for one-shot provisioning | ✅ Designed for one-shot | ❌ Designed for continuous reconciliation |

## Decision

**OpenTofu** (MPL 2.0) for Day 0 infrastructure provisioning on both Hetzner and Azure profiles. Configuration:

- **State storage:** S3 (Hetzner) / Azure Blob with encryption + remote locks (prevent concurrent applies)
- **Modules:** Reusable Hetzner + Azure modules (compute, networking, storage, DNS)
- **Approval:** TerraformPlan artifact in PR. Code review of changes before merge. Merge == apply authority.
- **Git flow:** cave-infra-config Git repo. Changes via PR. ARgoCD is NOT used for Terraform (separate tool: cave-ctl terraform apply)
- **Separation:** Day 0 (OpenTofu) = cluster setup. Day 1+ (Crossplane, ADR-067) = application infrastructure (databases, caches, queues).

## Implementation Reference

**Implementation Status:** Production

- **cave-infra** crate: OpenTofu modules for Hetzner + Azure, state management, lock handling
- **Modules:** hetzner-cluster, azure-cluster, shared-networking, dns-setup, s3-storage, etc.
- **State:** Remote backend with locks. Backup to encrypted storage. Disaster recovery: state reconstruction from IaC.

## Rejected Options

### Terraform — Licensing Concern (similar to Vault BSL)

**Reasons:**
1. **Proprietary cloud features:** HashiCorp moved Terraform Cloud and advanced features to proprietary licensing (Terraform Cloud → paid SaaS). Core CLI remains open source but ecosystem is increasingly proprietary.
2. **OpenTofu alternative:** OpenTofu is a direct fork of Terraform 1.6 (pre-licensing shift). Community-maintained. All Terraform skills transfer directly.
3. **Stability:** OpenTofu backed by Linux Foundation. Ensures long-term community control.

### Pulumi — Language Fragmentation

**Reasons:**
1. **Multi-language but heterogeneous:** Pulumi supports Python, Go, TypeScript, C#. CAVE standardizes on 5 languages (ADR-044). Introducing 6th language (for IaC) adds complexity.
2. **HCL is standard:** Infrastructure teams know HCL (Terraform, Ansible, other tools). HCL skill transfer is lower cost.
3. **Community size:** Terraform/OpenTofu community larger. More blog posts, modules, troubleshooting guides.

### Crossplane for Day 0 — Wrong Tool for Job

**Reasons:**
1. **Continuous reconciliation model:** Crossplane is designed for Day 1+ (continuous drift detection). One-shot cluster creation doesn't fit reconciliation model.
2. **Stateless by design:** Crossplane uses K8s CRDs as state (no separate state file). Loses plan/review/apply human-in-loop workflow. Cluster creation requires review.
3. **Day 0 + Day 1 split is correct:** OpenTofu for one-shot Day 0. Crossplane for continuous Day 1+.

## Consequences

### Positive

- **Cloud-agnostic:** Same HCL for Hetzner + Azure. Modules abstract provider differences.
- **Familiar workflow:** Plan/apply is industry standard. Team skills transferable from other Terraform projects.
- **State management:** Remote state + locks prevent concurrent modifications. Disasters recoverable from IaC + state backup.
- **Code review integration:** Terraform plan artifact in PR. Reviewers see exact infrastructure changes before apply.
- **MPL 2.0 license:** No proprietary restrictions. Community-maintained fork ensures stability.

### Negative

- **Learning curve:** HCL syntax + Terraform concepts (state, modules, providers, locks) require training.
- **State management overhead:** Remote state backend must be maintained (S3 bucket, Azure storage). State backups required.
- **Drift detection limited:** OpenTofu shows drift on plan. Continuous monitoring requires separate tools (Infracost for cost drift, Checkov for compliance drift).

### Risks & Mitigations

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| State file corruption/loss | Low | High | Remote state with versioning (S3 versioning). Weekly backups. Disaster recovery: state reconstruction from IaC + cloud API. |
| Breaking change in provider | Low | High | Provider version pinning in .terraform-lock.hcl (ADR-108). Staging validates before prod. |
| Concurrent apply causes state conflict | Low | Medium | Remote locks prevent concurrent applies. Runbook for lock recovery (rare). |
| Hetzner provider abandoned | Low | Medium | Community provider maintained by Hetzner community. If abandoned, fallback: API-based cluster creation scripts + import into IaC. |

## License

**OpenTofu:** Mozilla Public License 2.0 (https://github.com/opentofu/opentofu/blob/main/LICENSE)

## Compliance Mapping

**SOC2 CC8.1:** Infrastructure configuration management — IaC ensures reproducible, auditable cluster setup.
**SOC2 CC8.2:** Monitoring and control — Terraform plan enforces review gate.
**ISO/IEC 27001 A.5.30:** Access control — Approve infrastructure changes via Git (tied to user identity).
**ISO/IEC 27001 A.8.9:** Configuration management — infrastructure as versioned code in Git repository.
**NIS2 Directive Article 21:** Configuration management — Infrastructure changes reviewed and version controlled.
