# ADR-142: Passwordless Authentication Strategy

**Status:** Accepted

**Scope:** Universal

**Category:** Identity / Security

**Related ADRs:** 006 (Keycloak/Hetzner), 007 (Okta+Entra/Azure), 014 (Zero Trust), 064 (Identity Split), 104 (Identity Lifecycle), 129 (BYOID Federation)

## Context

Passwords are the leading attack vector for enterprise breaches. Phishing, credential stuffing, password reuse, and brute force attacks account for over 80% of identity-related incidents (Verizon DBIR 2024-2025). The industry is converging on passwordless authentication via FIDO2/WebAuthn/passkeys as the replacement.

CAVE operates across two identity stacks:
- **Hetzner:** Keycloak (self-hosted, ADR-006) with WebAuthn support
- **Azure:** Okta Workforce (ADR-007) with FastPass + Entra ID with Microsoft passkeys

A unified passwordless strategy must work across both profiles, support tenant BYOID federation (ADR-129), and provide a migration path for existing password-based users.

### Industry Trajectory (2025-2028)

- **FIDO Alliance** passkey adoption accelerating: Apple (iCloud Keychain sync), Google (Chrome + Android), Microsoft (Windows Hello + Entra) all support passkeys natively.
- **NIST SP 800-63-4 (draft):** Phishing-resistant MFA (FIDO2) as AAL3. Passwords alone no longer meet AAL2.
- **EU NIS2 Directive (Art.21):** Strong authentication required for critical infrastructure operators. Passwordless satisfies this requirement.
- **Enterprise adoption:** 2025 surveys show ~25% enterprise passkey adoption; projected ~60% by 2027 (Gartner).
- **Remaining gaps:** Cross-device passkey portability, enterprise attestation, account recovery without passwords.

---

## Candidates

| Approach | Passkey-First (chosen) | MFA-Only (no passwordless) | Password Elimination (aggressive) | Certificate-Based (mTLS client certs) |
|---|---|---|---|---|
| **User experience** | Biometric/PIN on device → seamless login | Password + TOTP/push → friction | No passwords anywhere → some users locked out | Certificate enrollment → complex onboarding |
| **Phishing resistance** | ✅ FIDO2 is origin-bound — cannot be phished | ⚠️ TOTP/SMS can be phished (SIM swap, real-time proxy) | ✅ Same as passkey-first | ✅ Certificate is bound to device |
| **Adoption curve** | Gradual — password still available as fallback during transition | No change needed | ❌ Disruptive — users without FIDO2 devices cannot authenticate | ❌ Steep — requires PKI infrastructure, device enrollment |
| **Cross-platform** | ✅ Passkeys synced via iCloud/Google/MS | ✅ TOTP works everywhere | ✅ If all platforms support passkeys | ⚠️ Cert enrollment per-device, per-OS |
| **Tenant compatibility** | ✅ BYOID tenants can bring their own passkey policy | ✅ Any MFA works | ⚠️ Forces tenants to adopt passkeys | ⚠️ Tenants must enroll in our PKI |
| **Recovery** | Password fallback during transition; recovery codes for passkey-only phase | Password reset flow | ❌ Recovery without passwords is unsolved for all edge cases | Certificate re-enrollment |
| **Compliance** | ✅ FIDO2 satisfies NIST AAL3, NIS2, SOC2 | ⚠️ TOTP = AAL2 only | ✅ Exceeds all requirements | ✅ Exceeds all requirements |

---

## Decision

**Passkey-first, phased migration** across all profiles. Passwords remain available during transition but are actively deprecated.

### Phase 1: MFA Mandatory (Current → Q3 2026)
- All platform users: password + MFA (TOTP, push, or FIDO2)
- Encourage FIDO2 registration at every login
- Track passkey adoption metrics per tenant
- **Hetzner:** Keycloak WebAuthn authentication policy (required action: configure FIDO2)
- **Azure:** Okta Verify push + FIDO2 enrollment prompt, Entra Conditional Access (require MFA for Azure Portal)

### Phase 2: Passkey-Preferred (Q4 2026 → Q2 2027)
- Default login flow: passkey first, password as fallback
- New user onboarding: passkey registration mandatory, password optional
- Admin accounts: passkey-only (no password fallback) — enforced via policy
- **Hetzner:** Keycloak authentication flow configured as WebAuthn-preferred
- **Azure:** Okta FastPass as primary, Entra passwordless phone sign-in

### Phase 3: Password-Optional (Q3 2027 → 2028)
- Users with registered passkeys can delete their passwords
- New tenants: passwordless-by-default option in tenant onboarding
- Password retention only for: break-glass admin accounts, legacy system integrations, BYOID tenants whose IdP requires passwords
- **Hetzner:** Keycloak credential management allows password removal for users with 2+ FIDO2 keys
- **Azure:** Okta passwordless policy, Entra TAP (Temporary Access Pass) for recovery

### Implementation Details

**FIDO2 Authenticator Types:**
| Type | Use Case | Sync | Platform |
|---|---|---|---|
| Platform authenticator (biometric) | Daily login — Touch ID, Face ID, Windows Hello, Android biometric | Synced via iCloud/Google/MS passkey | All platforms |
| Roaming authenticator (security key) | Admin accounts, high-security operations, break-glass | Not synced (physical device) | YubiKey 5, Titan, SoloKey |

**Recommendation:** Platform engineers get 2x YubiKey 5 NFC (primary + backup). Tenant admins encouraged to register platform passkey + 1 security key.

**BYOID Tenant Integration (ADR-129):**
- Tenants using their own IdP via federation: passwordless policy is tenant's responsibility
- CAVE enforces: federated assertion must include `amr` claim with `hwk` (hardware key) or `swk` (software key) for admin-level access
- Tenants without passkey support: CAVE accepts TOTP MFA during Phase 1-2, blocks password-only after Phase 3

**Account Recovery (passwordless users):**
1. Primary: Second registered FIDO2 key (backup security key)
2. Secondary: Recovery codes (8x one-time codes, generated at passkey registration, stored offline)
3. Emergency: Admin-initiated Temporary Access Pass (TAP) — 24hr one-time code, logged in Sovereign Ledger

---

## Rejected Options

### MFA-Only (No Passwordless Roadmap) — Rejected

**Primary:** TOTP/SMS MFA is phishable. Real-time phishing proxies (e.g., Evilginx) intercept TOTP codes in transit. Push notification fatigue attacks (MFA bombing) succeeded against Uber (2022) and Microsoft (2023). FIDO2/passkeys are origin-bound and cryptographically resistant to all known phishing vectors. Staying on TOTP-only leaves CAVE vulnerable to the attack class that caused the most high-profile breaches of 2022-2024.

**Secondary:** Regulatory trajectory. NIST SP 800-63-4 is moving AAL3 to require phishing-resistant MFA. NIS2 Art.21 requires "strong authentication." By 2027, TOTP-only may not satisfy compliance requirements for critical infrastructure platforms.

### Aggressive Password Elimination — Rejected

**Primary:** Account recovery. If a user loses their passkey device and has no backup security key, recovery without a password is extremely difficult. Industry solutions (social recovery, identity verification services) are not yet mature or standardized for enterprise use. Forcing passwordless before recovery is solved creates lockout risk.

**Secondary:** BYOID compatibility. Enterprise tenants federating via SAML/OIDC may not support passkeys on their IdP side. Forcing passwordless on CAVE's side would block tenants whose corporate IdP still requires passwords.

### Certificate-Based Authentication (mTLS) — Rejected

**Primary:** Onboarding friction. Client certificate enrollment requires PKI infrastructure, per-device certificate provisioning, and certificate lifecycle management (renewal, revocation). For a platform serving diverse tenants, this complexity is prohibitive. Passkeys provide equivalent phishing resistance with consumer-grade UX (biometric touch).

**Secondary:** FIDO2/passkeys are the industry convergence point. Apple, Google, Microsoft, and the FIDO Alliance are investing billions in passkey infrastructure. mTLS for user authentication is moving in the opposite direction (toward machine-to-machine only, where SPIFFE/mTLS via Istio, ADR-004, handles it).

---

## Consequences

### Positive

- Phishing-resistant authentication eliminates the #1 breach vector
- NIST AAL3 + NIS2 Art.21 compliance from Phase 2 onward
- Improved UX: biometric login is faster than typing password + TOTP
- Reduced help desk load: no password resets (70% of identity-related tickets, Gartner)
- Platform authenticator sync (iCloud/Google/MS) means passkeys survive device loss for most users
- BYOID tenants benefit when their IdP also supports passkeys (network effect)

### Negative

- Phase 1→2 requires active user migration effort (enrollment campaigns, training)
- YubiKey cost for platform team (~€50-70 per key, 2 keys per engineer)
- Account recovery complexity increases without passwords
- Some edge cases remain unsolved: shared workstations, legacy kiosk systems, automation service accounts
- Cross-platform passkey portability still maturing (Apple↔Google sync not available as of 2025)

### Risks

| Risk | Probability | Impact | Mitigation |
|---|---|---|---|
| Low passkey adoption by users | Medium | Medium | Enrollment nudges at login. Track adoption metrics. Phase 2 makes passkey the default flow (opt-out to password, not opt-in to passkey). |
| Passkey vendor lock-in (Apple/Google/MS silos) | Medium | Low | FIDO2 is an open standard. Security keys (YubiKey) are vendor-neutral. Platform authenticators synced within ecosystem but FIDO2 roaming keys work everywhere. |
| Cross-device portability gaps | Medium (2025-2026) | Low | **Watch:** FIDO Alliance "multi-device credentials" spec evolving. Apple-Google-MS passkey import/export expected by 2027. Monitor and adjust Phase 3 timeline accordingly. |
| Account recovery failure (lockout) | Low | High | Mandatory backup security key for admins. Recovery codes at registration. TAP emergency flow. Break-glass admin account with password retained (single account, PAM-protected, ADR-130). |
| BYOID tenant IdP doesn't support FIDO2 | Medium | Medium | Phase 1-2: accept TOTP MFA from federated tenants. Phase 3: require `amr` claim audit — tenants not meeting phishing-resistant MFA standard get migration support. |
| Keycloak WebAuthn bugs / limitations | Low | Medium | Keycloak WebAuthn is GA since v21. Pin to tested version. Staging validates all auth flows before prod upgrade. |

---

## Compliance Mapping

**NIST SP 800-63-4 (AAL3):** Phishing-resistant authenticator required — FIDO2 passkeys satisfy this. Passwords + TOTP = AAL2 only.
**SOC2 CC6.1:** Logical access security — passkey-first reduces credential theft risk.
**SOC2 CC6.6:** System hardening — eliminating passwords removes the most attacked credential type.
**ISO A.5.17:** Authentication information — FIDO2 keys are cryptographic, not knowledge-based (stronger than passwords).
**ISO A.8.5:** Secure authentication — biometric + cryptographic key satisfies multi-factor requirement in a single gesture.
**NIS2 Art.21:** Strong authentication for critical infrastructure — passkeys exceed the minimum requirement.
**GDPR Art.32:** Security of processing — reducing phishing risk protects personal data processed by the platform.
**DORA Art.9 (financial sector):** Strong authentication — passkey-first aligns with digital operational resilience requirements.
