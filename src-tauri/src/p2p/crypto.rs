use rcgen::{generate_simple_self_signed, CertifiedKey};
use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};

use super::{P2pError, Result};

pub struct EphemeralIdentity {
    pub certificate: CertificateDer<'static>,
    pub private_key: PrivatePkcs8KeyDer<'static>,
}

impl EphemeralIdentity {
    pub fn generate() -> Result<Self> {
        let CertifiedKey { cert, key_pair } =
            generate_simple_self_signed(vec!["catcl.local".into()])
                .map_err(|error| P2pError::CryptoError(error.to_string()))?;
        Ok(Self {
            certificate: cert.der().clone(),
            private_key: PrivatePkcs8KeyDer::from(key_pair.serialize_der()),
        })
    }
}
