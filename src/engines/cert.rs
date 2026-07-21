//! Decode PEM / DER X.509 certificates into a structured JSON value that the
//! TreeEngine renders — subject, issuer, validity, serial, SANs, key info.

use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use x509_parser::prelude::*;

/// Decode a certificate file into a JSON value for the tree view.
pub fn decode(path: &std::path::Path) -> Result<Value> {
    let bytes = super::read_text_file_bytes(path)?;

    // Try PEM first (may contain multiple certs), then raw DER.
    let mut certs = Vec::new();
    if bytes.starts_with(b"-----BEGIN") {
        for pem in Pem::iter_from_buffer(&bytes).flatten() {
            if let Ok(cert) = pem.parse_x509() {
                certs.push(cert_to_json(&cert));
            }
        }
    }
    if certs.is_empty() {
        // Fall back to DER (fail loudly if it isn't a certificate).
        let (_, cert) =
            X509Certificate::from_der(&bytes).map_err(|e| anyhow!("not a valid certificate: {}", e))?;
        certs.push(cert_to_json(&cert));
    }

    if certs.len() == 1 {
        Ok(certs.into_iter().next().unwrap())
    } else {
        Ok(json!({ "certificates": certs }))
    }
}

fn cert_to_json(cert: &X509Certificate) -> Value {
    let validity = cert.validity();
    let sans: Vec<String> = cert
        .subject_alternative_name()
        .ok()
        .flatten()
        .map(|ext| {
            ext.value
                .general_names
                .iter()
                .map(|gn| format!("{}", gn))
                .collect()
        })
        .unwrap_or_default();

    json!({
        "subject": cert.subject().to_string(),
        "issuer": cert.issuer().to_string(),
        "serial": cert.raw_serial_as_string(),
        "version": cert.version().0 + 1,
        "not_before": validity.not_before.to_string(),
        "not_after": validity.not_after.to_string(),
        "is_ca": cert.is_ca(),
        "signature_algorithm": format!("{}", cert.signature_algorithm.algorithm),
        "public_key_algorithm": format!("{}", cert.public_key().algorithm.algorithm),
        "subject_alt_names": sans,
    })
}
