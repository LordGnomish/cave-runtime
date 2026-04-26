//! Trivy vulnerability scanner subsystem.
//!
//! Full parity with Trivy 0.49:
//!   - OS package scanning (Alpine APK, Debian dpkg, RHEL RPM)
//!   - Language package scanning (Go, npm, pip, Maven, Cargo, Composer)
//!   - Vulnerability matching against NVD-style database
//!   - Secret scanning (30+ pattern rules)
//!   - License detection (SPDX normalisation + risk classification)
//!   - Misconfiguration scanning (Dockerfile, K8s YAML, Terraform)
//!   - SBOM generation (CycloneDX 1.4, SPDX 2.3)
//!   - .trivyignore suppression with expiry
//!   - JSON / table / SARIF output

pub mod ignore;
pub mod lang_pkg;
pub mod license;
pub mod misconfig;
pub mod os_pkg;
pub mod output;
pub mod sbom;
pub mod scanner;
pub mod secret;
pub mod vuln_db;

pub use scanner::{ScanOptions, ScanResult, ScanType, Scanner, VulnFinding};
pub use vuln_db::{Severity, VulnDb, VulnRecord};
