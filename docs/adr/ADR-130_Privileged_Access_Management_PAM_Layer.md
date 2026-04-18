# ADR-130: Privileged Access Management (PAM) Layer

**Status:** Proposed

## Category:

**Related ADRs:** 006, 007, 020, 053, 064, 079, 083, 092, 104, 112, 125, 128, 129

**Back to Index:** =HYPERLINK("#Index!A1","← Back to Index")

## Context

## ### 1.1 Current Identity Stack

| Layer | Hetzner (Sovereign) | Azure (Enterprise) |
|---|---|---|
| User Identity (OIDC) | Keycloak | Okta Workforce Identity |
| Resource RBAC | Keycloak roles → OPA | Entra ID (Azure RBAC only) |
| Secrets Management | OpenBao + ESO | Azure Key Vault + ESO |
| Service Identity | Istio ambient mTLS | Istio ambient mTLS |
| Machine Identity | OpenBao AppRole / K8s auth | Key Vault + Managed Identity |

### 1.2 The PAM Gap

CAVE handles authentication, authorization, and secrets. It does **not** handle Privileged Access Management — governance for how privileged sessions are initiated, monitored, recorded, and audited.

| Capability | Current State | Gap |
|---|---|---|
| Session recording (video/keystroke) | Tetragon syscall + Hubble network → WORM. Reconstructable but no native video replay. | Medium |
| Privileged session proxy | No centralized proxy. kubectl exec restricted on prod (break-glass only) but no session broker. | **High** |
| JIT privilege elevation | n8n + Keycloak/Okta temp role, 4h TTL (ADR-104). Functional but not standardized. | Medium |
| Credential checkout/checkin | OpenBao dynamic credentials (lease-based). No manual checkout for shared/emergency accounts. | Medium |
| Third-party vendor access | No mechanism for external vendors to access with scoped, recorded sessions. | **High** |
| Break-glass audit | Approval hash → Sovereign Ledger. No recording of what operator actually did. | **High** |
| Compliance evidence | SOC2 CC6.1-6.3 met via Loki + Ledger. Full PAM audit trail would strengthen NIS2 Art.21. | Medium |

### 1.3 Regulatory Drivers

| Framework | Requirement | PAM Relevance |
|---|---|---|
| SOC2 CC6.1 | Logical access controls | Session proxy + recording closes audit gap |
| SOC2 CC6.2 | Privileged access provisioning | JIT elevation + checkout workflow |
| ISO 27001 A.5.15 | Access control | PAM policy engine enforces least privilege |
| ISO 27001 A.8.2 | Privileged access rights | Dedicated PAM vs manual processes |
| NIS2 Art.21(2)(i) | Access control policies | Session governance, vendor access |
| GDPR Art.32 | Security of processing | Session recording for personal data access |

---

## Candidates

## ### 3.1 Evaluation Matrix

| Candidate | Type | License | K8s-Native | Self-Hosted |
|---|---|---|---|---|
| CyberArk Privilege Cloud | Enterprise PAM | Proprietary SaaS | No (agent) | Yes (separate product) |
| Teleport Enterprise | Infrastructure PAM | Proprietary | Yes (Helm) | Yes |
| Teleport Community | Infrastructure PAM | Modified Apache 2.0 (<100 emp / <$10M) | Yes (Helm) | Yes |
| HashiCorp Boundary | Access Broker | BSL 1.1 | Yes (Helm) | Yes |
| StrongDM | PAM Proxy | Proprietary SaaS | No | No |
| BeyondTrust PRA | Enterprise PAM | Proprietary | No | Yes |

### 3.2 Feature Comparison

| Capability | CyberArk PCloud | Teleport Community | Boundary (BSL) | StrongDM | BeyondTrust |
|---|---|---|---|---|---|
| K8s API proxy | SIA | Native `tsh kube` | Native `boundary connect kube` | Proxy | No |
| Database proxy | PSM/SIA | Native (PG, MySQL, Mongo, Redis, ES) | Via Vault injection | Native (100+ protocols) | Jump host |
| SSH proxy | PSMP/SIA | Native (cert-based, no SSH keys) | SSH target | Native | Native |
| Session recording | Video + keystroke, tamper-proof | Structured event log, web UI replay | Enterprise only | Full capture | Video |
| JIT access | CyberArk workflow | Native access requests + approval | Via Vault | Native | Native |
| Zero Standing Privileges | Ephemeral accounts (SIA) | Short-lived certificates | Dynamic Vault creds | Ephemeral creds | Checkout |
| OIDC/SAML federation | All major IdPs | Community: GitHub only. Enterprise: all | All OIDC | All | All |
| MCP server | No | **Yes** (native, 2025) | No | No | No |
| Vendor access | Vendor PAM (native) | Access requests + temp roles | Managed sessions | Workflows | Secure remote |
| K8s-native deployment | No (Windows connectors + SIA) | Yes (Helm, K8s pods) | Yes (Helm) | No (SaaS) | No |
| Sovereign self-hosting | Self-hosted is different product | Yes | Yes | No | No |

### 3.3 Cost Comparison

| Solution | Pricing Model | Estimated Annual Cost (10-20 privileged users) |
|---|---|---|
| CyberArk Privilege Cloud | Per-user annual, enterprise sales | €15,000 – €40,000 + professional services |
| Teleport Community | Free (<100 emp / <$10M revenue) | €0 (self-hosted infra cost only) |
| Teleport Enterprise | Per-resource | €10,000 – €25,000 |
| Boundary Enterprise | Per-resource, HCP usage-based | €8,000 – €20,000 |
| Boundary Community | Free (BSL limitations) | €0 (self-hosted infra cost only) |
| StrongDM | Per-user/month SaaS | €10,000 – €24,000 |
| BeyondTrust PRA | Per-user annual, enterprise sales | €20,000 – €50,000 |

### 3.4 Architecture Fit Scoring

| Criteria | Weight | CyberArk PCloud | Teleport CE | Boundary | StrongDM | BeyondTrust |
|---|---|---|---|---|---|---|
| Sovereign self-hosting | Critical | ❌ | ✅ | ✅ | ❌ | ❌ |
| K8s-native | High | ❌ | ✅ | ✅ | ❌ | ❌ |
| Talos compatible | High | ❌ (Windows connectors) | ✅ (no host agent) | ✅ | N/A | ❌ (agent) |
| OpenBao integration | High | ❌ (competes) | ⚠️ (own cert model) | ✅ (Vault-native) | ❌ | ❌ |
| Keycloak OIDC | High | ✅ (generic) | ⚠️ (CE: GitHub only) | ✅ | ✅ | ✅ |
| Okta integration | High | ✅ (native) | ✅ (Enterprise) | ✅ | ✅ | ✅ |
| License risk | Medium | ✅ (stable) | ⚠️ (size restriction) | ⚠️ (BSL = Vault risk) | ✅ | ✅ |
| MCP/APOL support | Medium | ❌ | ✅ (native MCP) | ❌ | ❌ | ❌ |
| Complexity budget | Medium | ❌ (Windows VMs) | ✅ (~1.5GB RAM) | ✅ (+Vault needed) | ❌ | ❌ |
| Enterprise compliance cred | Medium | ✅ (gold standard) | ⚠️ (newer) | ⚠️ | ⚠️ | ✅ |

**Result:** CyberArk wins for Azure (enterprise compliance credibility, Okta-native, managed SaaS). Teleport CE wins for Hetzner (K8s-native, self-hosted, MCP support, zero cost, Talos-compatible).

---

## Architecture

### 4.1 Azure: CyberArk Privilege Cloud

```
User → Okta (MFA) → CyberArk Privilege Cloud
                         ├── SIA → AKS (kubectl proxy, recorded)
                         ├── SIA → Azure PG Flexible (DB proxy, recorded)
                         ├── PSM → Azure Portal (web session, recorded)
                         ├── CPM → Credential rotation (automated)
                         └── Audit → Loki → Sovereign Ledger
```

**Components:**

| Component | Purpose | Deploy |
|---|---|---|
| Privilege Cloud (SaaS) | Central vault + policies | CyberArk-hosted |
| CPM Connector | Credential rotation | Windows VM (D2s_v5) |
| PSM Connector | RDP/Web session recording | Windows VM (D4s_v5) |
| SIA Connector | Modern K8s/DB/SSH proxy | K8s pod or lightweight agent |

**Note on Windows:** CyberArk's SIA is actively replacing PSM/PSMP — lightweight, Linux-compatible, reverse SSH tunnel. Strategy: SIA for K8s/DB/SSH, minimal PSM for web recording until SIA parity.

**Integration points:**
1. Okta → CyberArk: SAML 2.0 federation, MFA via Okta Verify
2. CyberArk → AKS: SIA K8s API proxy with session recording
3. CyberArk → Azure PG: SIA/PSM database broker, per-session credentials via CPM
4. CyberArk → Azure Portal: Secure Web Sessions (SWS), passwordless recorded access
5. CyberArk → Entra ID: JIT privileged role assignment (Global Admin, Sub Owner)
6. CyberArk → Ledger: Syslog → Loki → `Privileged Session` attestation

### 4.2 Hetzner: Teleport Community Edition

```
User → Keycloak (MFA) → [user sync] → Teleport (self-hosted K8s)
                                            ├── K8s Agent → Talos cluster (kubectl, recorded)
                                            ├── DB Agent → CloudNativePG (PG proxy, recorded)
                                            ├── DB Agent → Valkey/OpenSearch (recorded)
                                            ├── App Agent → Internal web apps (recorded)
                                            └── Audit → WORM bucket → Sovereign Ledger
```

**Components:**

| Component | Purpose | Resources |
|---|---|---|
| Auth Service | Certificate authority, user auth | 1 pod, 512Mi RAM |
| Proxy Service | TLS termination, web UI, access point | 1 pod, 512Mi RAM |
| K8s Agent | Registers K8s cluster | 1 pod, 256Mi per cluster |
| DB Agent | Database protocol proxy | 1 pod, 256Mi |
| **Total** | | ~1.5 GB RAM, 2 vCPU |

**Why Teleport CE over Boundary:**

| Factor | Teleport CE | Boundary |
|---|---|---|
| License | Modified Apache 2.0 (CAVE eligible) | BSL 1.1 — contradicts ADR-020 (Vault→OpenBao BSL migration precedent) |
| Session recording | Structured events in CE (replayable) | Enterprise only — CE has audit events only |
| OpenBao | Not needed — Teleport issues own short-lived certs, no persistent credentials | Core workflow depends on Vault, not OpenBao. Compatibility unverified. |
| MCP server | Native — enables APOL to use PAM-proxied operations | None — APOL would need custom integration |
| Complexity | Single binary, Helm chart | Controller + Worker + Vault + DB backend |
| DB proxy | Native multi-protocol (PG, MySQL, Mongo, Redis, ES) | Requires Vault credential injection |

**Keycloak federation workaround (CE OIDC limitation):**

Teleport CE restricts OIDC to GitHub auth. Workaround via `cave-ctl identity sync`:
1. Query Keycloak Admin API for users with `platform-admin` / `guardian` role
2. Create/update Teleport local users via `tctl` API
3. Map Keycloak roles → Teleport roles (`guardian` → `access,editor,auditor`)
4. Sync interval: 5 minutes
5. Keycloak removal → Teleport user lock at next sync

If upgraded to Enterprise later, sync replaced by native OIDC connector.

---

## Integration with CAVE Components

### 5.1 Identity Lifecycle Enhancement (ADR-104)

| Capability | Before PAM | After PAM (Hetzner) | After PAM (Azure) |
|---|---|---|---|
| JIT access | n8n → Keycloak temp role (4h) | n8n → Teleport Access Request → short-lived cert | n8n → CyberArk JIT → ephemeral account |
| Break-glass | Dual-approval → hash to Ledger → direct kubectl | Dual-approval → Teleport recorded session → Ledger | Dual-approval → CyberArk PSM recorded → Ledger |
| Vendor access | Not supported | Teleport temp user + mandatory recording | CyberArk Vendor PAM + scoped + recorded |
| Dormant detection | Keycloak 90d event listener | + Teleport session inactivity correlation | + CyberArk last-login correlation |

### 5.2 APOL Integration (ADR-112)

Teleport's MCP server uniquely aligns with APOL:

| APOL Role | Without PAM | With Teleport MCP |
|---|---|---|
| AI SRE | cave-ctl → direct K8s API | cave-ctl → Teleport MCP → K8s API (recorded) |
| AI Compliance | OPA + Loki analysis | + Teleport session audit (who, when, how long) |
| AI Change Manager | Renovate → ArgoCD | + Teleport Access Request for manual upgrade steps |

**Azure asymmetry:** CyberArk has no MCP server. AI operations on Azure bypass PAM. Accepted trade-off — AI governed by ADR-092 allowlist/denylist/ceiling.

### 5.3 Sovereign Ledger New Attestation (ADR-093)

```yaml
type: Privileged Session
schema:
  session_id: string          # PAM-generated
  operator: string            # cave_uid
  target: string              # e.g. "k8s:prod-hetzner", "db:billing-pg"
  access_method: string       # "teleport" | "cyberark" | "break-glass"
  justification: string       # From access request
  approval_chain: string[]    # cave_uid list of approvers
  start_time: datetime
  end_time: datetime
  recording_ref: string       # WORM bucket reference (not recording itself)
  actions_summary: string     # Redacted per ADR-128
```

### 5.4 Compliance Enhancement

| Control | Framework | Before | After |
|---|---|---|---|
| Session governance | SOC2 CC6.1-6.3 | Partial (Loki+Ledger) | Full (PAM recording + Ledger) |
| Vendor access | ISO A.5.19, NIS2 Art.21 | Gap | Closed |
| Break-glass audit | SOC2 CC7.2, ISO A.5.24 | Approval hash only | Full session recording |
| Session monitoring | NIS2 Art.21(2)(b) | Forensic reconstruction | Real-time monitoring |

---

## Decision

## **Azure (Enterprise):** CyberArk Privilege Cloud as PAM layer, integrated with Okta (SAML federation) and Entra ID (Azure RBAC target).

**Hetzner (Sovereign):** Teleport Community Edition (self-hosted on K8s) as PAM layer, integrated with Keycloak (user sync, future OIDC) and OpenBao (credential brokering).

Both produce session audit records flowing to Sovereign Ledger as new attestation type: `Privileged Session`.

---

## Rejected

## ### 6.1 HashiCorp Boundary — Rejected

**Primary:** BSL 1.1 license. ADR-020 established BSL avoidance precedent (Vault → OpenBao). Adopting another BSL product contradicts this.

**Secondary:** Session recording Enterprise-only. OpenBao compatibility unverified. Additional infra (controller+worker+DB). No MCP. Protocol gaps (SSH/RDP focus).

### 6.2 StrongDM — Rejected

**Primary:** SaaS-only. No self-hosted. Incompatible with sovereign profile.

**Secondary:** Vendor lock-in, high cost (~€70-100/user/month), no K8s-native deploy, no MCP.

### 6.3 BeyondTrust PRA — Rejected

**Primary:** Agent-based, incompatible with Talos Linux (immutable, no SSH, no packages).

**Secondary:** Windows-centric, no K8s-native, enterprise-only pricing, no MCP.

### 6.4 CyberArk Self-Hosted (for Hetzner) — Rejected

**Primary:** Requires Windows Server for core components (Digital Vault, CPM, PSM, PVWA). Contradicts K8s-native, Linux-only, immutable infra principles (ADR-098).

**Secondary:** GOT increase ~1-2 weeks, Windows licensing, massive complexity.

### 6.5 Teleport Enterprise (for Hetzner) — Deferred

CE sufficient for current team size. Enterprise upgrade path is minimal (same binary, add license key, enable OIDC). Trigger: if CAVE deployed for >100 employees or >$10M revenue entity.

### 6.6 Native Combo Only (OpenBao + n8n + Tetragon) — Rejected as standalone

Current stack provides PAM-adjacent capabilities but lacks: unified session proxy, native session recording with replay, standardized vendor access, PAM compliance reporting. Kept as defense-in-depth alongside PAM (two independent audit trails: PAM application-level + Tetragon kernel-level).

---

## Implementation

### 7.1 Phase Mapping

Teleport and CyberArk deploy in **Phase 3 (Advanced)**.

| Task | Effort | Dependencies |
|---|---|---|
| Teleport Helm deploy on Hetzner | 1 day | K8s cluster, Keycloak |
| Teleport K8s + DB agent config | 1 day | CloudNativePG, Talos |
| Keycloak → Teleport user sync | 1 day | Keycloak Admin API |
| CyberArk Privilege Cloud onboarding | 1 week | Vendor engagement, Okta |
| CyberArk SIA agent on AKS | 1 day | AKS, Okta SAML |
| cave-ctl pam commands | 2 days | Teleport + CyberArk APIs |
| Sovereign Ledger integration | 1 day | Ledger attestation pipeline |
| **Total** | ~2 weeks | |

### 7.2 cave-ctl PAM Commands

```
cave-ctl pam sessions list --profile <p>
cave-ctl pam sessions replay --id <session-id>
cave-ctl pam sessions terminate --id <session-id>
cave-ctl pam request create --target <t> --reason <r>
cave-ctl pam request approve|deny --id <req-id>
cave-ctl pam connect k8s|db|web --target <t>
cave-ctl pam audit --since <t> --user <u>
cave-ctl pam compliance report --framework soc2
```

### 7.3 Complexity Budget

| Metric | Before | After | Delta |
|---|---|---|---|
| Component count | ~70 | ~72 | +2 |
| GOT estimate | 2 weeks | 2 weeks + 0.5 day | <10% |
| K8s RAM (Hetzner) | Baseline | +1.5 GB | Minimal |
| K8s RAM (Azure) | Baseline | +256Mi (SIA pod) | Minimal |

PAM is not in resurrection critical path. GOT within 10% threshold.

---

## Consequences

## ### Positive

- Closes privileged session governance gap (SOC2 CC6.1-6.3, NIS2 Art.21)
- Vendor access capability (previously missing)
- Break-glass now fully recorded
- Teleport MCP enables PAM-proxied APOL operations with audit trail
- Two independent audit trails: PAM + Tetragon/Hubble forensics
- CyberArk satisfies enterprise compliance expectations
- Teleport maintains sovereign self-hosting

### Negative

- Two PAM products (cognitive load)
- CyberArk requires Windows connector VM(s) until SIA parity
- Teleport CE OIDC limitation requires sync workaround
- CyberArk licensing cost
- APOL asymmetry: PAM-proxied on Hetzner (Teleport MCP), not on Azure

### Risks

| Risk | Prob | Impact | Mitigation |
|---|---|---|---|
| Teleport CE license tightens | Medium | High | cave-ctl pam abstraction enables swap. Boundary/Enterprise fallback. |
| CyberArk SIA delayed vs PSM | Low | Medium | Keep PSM as bridge. SIA roadmap tracked by cave-ctl roadmap scan. |
| Keycloak sync fails | Low | Medium | Health check in cave-ctl doctor. Alert on sync delta >10min. |
| CyberArk cost escalation | Medium | Medium | Annual contract review. Negotiate based on competitive alternatives. |

## Compliance Mapping

## SOC2 CC6.1-6.3 (privileged access management, session recording, JIT access). SOC2 CC7.2 (monitoring — session recording for forensics). ISO A.5.15 (access control — PAM enforces least privilege). ISO A.8.2 (privileged access rights). ISO A.8.5 (secure authentication — MFA + session recording). NIS2 Art.21 (access control — privileged session governance). GDPR Art.32 (security of processing — controlled admin access).
