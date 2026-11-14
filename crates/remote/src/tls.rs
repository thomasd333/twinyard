//! TLS: the Windows certificate store decides first (corporate CAs work);
//! a certificate the user explicitly trusted (pinned by fingerprint, also
//! imported from TC) is accepted even if self-signed or issued for another
//! name — the typical shared-hosting case (`ftps://<ip>:990`).

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, Error, SignatureScheme};
use std::sync::{Arc, Mutex};

/// Why a certificate was rejected — shown to the user, who may trust it.
#[derive(Debug, Clone)]
pub struct Rejected {
    pub sha256: String,
    pub sha1: String,
    pub reason: String,
}

#[derive(Debug)]
pub struct PinVerifier {
    inner: rustls_platform_verifier::Verifier,
    provider: Arc<CryptoProvider>,
    pins: Vec<String>,
    pub rejected: Mutex<Option<Rejected>>,
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

/// "AABB…" -> "AA:BB:…" for display.
pub fn pretty(h: &str) -> String {
    h.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap_or("")).collect::<Vec<_>>().join(":")
}

fn digest(alg: &'static ring::digest::Algorithm, data: &[u8]) -> String {
    hex(ring::digest::digest(alg, data).as_ref())
}

impl PinVerifier {
    pub fn config(pins: Vec<String>) -> Result<(Arc<ClientConfig>, Arc<PinVerifier>), String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let inner = rustls_platform_verifier::Verifier::new(provider.clone()).map_err(|e| e.to_string())?;
        let v = Arc::new(PinVerifier { inner, provider: provider.clone(), pins, rejected: Mutex::new(None) });
        let cfg = ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| e.to_string())?
            .dangerous()
            .with_custom_certificate_verifier(v.clone())
            .with_no_client_auth();
        Ok((Arc::new(cfg), v))
    }
}

impl ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        let sha256 = digest(&ring::digest::SHA256, end_entity);
        let sha1 = digest(&ring::digest::SHA1_FOR_LEGACY_USE_ONLY, end_entity);
        if self.pins.iter().any(|p| *p == sha256 || *p == sha1) {
            return Ok(ServerCertVerified::assertion());
        }
        self.inner.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now).inspect_err(|e| {
            *self.rejected.lock().unwrap() = Some(Rejected { sha256, sha1, reason: e.to_string() });
        })
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}
