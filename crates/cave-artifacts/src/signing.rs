//! Signing service — GPG-sign repository metadata.

use crate::error::ArtifactsError;
use crate::models::SigningService;
use tracing::{info, warn};

/// Sign `data` using the provided signing service's script.
///
/// In production this would invoke an external GPG signing script.
/// Currently returns a placeholder detached signature.
pub async fn sign_metadata(
    service: &SigningService,
    data: &[u8],
) -> Result<Vec<u8>, ArtifactsError> {
    info!(
        service = %service.name,
        fingerprint = %service.pubkey_fingerprint,
        bytes = data.len(),
        "signing metadata (stub)"
    );

    if service.script.is_empty() {
        return Err(ArtifactsError::SigningError(
            "signing service has no script configured".into(),
        ));
    }

    // Placeholder: return a fake ASCII-armoured signature.
    let sig = format!(
        "-----BEGIN PGP SIGNATURE-----\n\
         Version: cave-artifacts (stub)\n\
         \n\
         stub-signature-for-{}\n\
         -----END PGP SIGNATURE-----\n",
        service.pubkey_fingerprint
    );

    warn!("sign_metadata is a stub — configure a real GPG script for production");
    Ok(sig.into_bytes())
}
