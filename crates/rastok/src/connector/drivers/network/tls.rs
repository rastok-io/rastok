use std::error::Error;
use std::fmt;
use std::io::Cursor;

#[cfg(feature = "hosted-http3")]
use super::pump::EngineLimits;

const HTTP3_ALPN: &[&[u8]] = &[b"h3"];
const MAX_QUIC_DATAGRAM_BYTES: usize = 1_350;
const MAX_QUIC_IDLE_TIMEOUT_MILLIS: u64 = 30_000;
const REQUIRED_H3_UNIDIRECTIONAL_STREAMS: usize = 3;

#[derive(Clone, Copy, Debug)]
pub(super) struct TlsIdentity<'a> {
    pub(super) certificate_chain: &'a [u8],
    pub(super) private_key: &'a [u8],
}

#[derive(Debug)]
pub(super) struct TlsConfigError(String);

impl TlsConfigError {
    fn at(boundary: &str, error: impl fmt::Display) -> Self {
        Self(format!("{boundary}: {error}"))
    }

    fn message(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for TlsConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for TlsConfigError {}

fn pem_payload(bytes: &[u8]) -> Option<&[u8]> {
    let start = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    bytes[start..]
        .starts_with(b"-----BEGIN")
        .then_some(&bytes[start..])
}

fn certificate_der(bytes: &[u8]) -> Result<Vec<Vec<u8>>, TlsConfigError> {
    if let Some(pem) = pem_payload(bytes) {
        let mut cursor = Cursor::new(pem);
        let certificates = rustls_pemfile::certs(&mut cursor)
            .map(|certificate| certificate.map(|value| value.as_ref().to_vec()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| TlsConfigError::at("the PEM certificate bundle is invalid", error))?;
        if certificates.is_empty() {
            return Err(TlsConfigError::message(
                "the PEM certificate bundle contains no certificate",
            ));
        }
        Ok(certificates)
    } else if bytes.is_empty() {
        Err(TlsConfigError::message("the DER certificate is empty"))
    } else {
        Ok(vec![bytes.to_vec()])
    }
}

fn trust_roots(custom_ca: Option<&[u8]>) -> Result<Vec<Vec<u8>>, TlsConfigError> {
    match custom_ca {
        Some(bytes) => certificate_der(bytes),
        None => Ok(webpki_root_certs::TLS_SERVER_ROOT_CERTS
            .iter()
            .map(|certificate| certificate.as_ref().to_vec())
            .collect()),
    }
}

#[cfg(feature = "hosted-http12")]
fn rustls_private_key(
    bytes: &[u8],
) -> Result<rustls::pki_types::PrivateKeyDer<'static>, TlsConfigError> {
    if let Some(pem) = pem_payload(bytes) {
        rustls_pemfile::private_key(&mut Cursor::new(pem))
            .map_err(|error| TlsConfigError::at("the PEM private key is invalid", error))?
            .ok_or_else(|| TlsConfigError::message("the PEM bundle contains no private key"))
    } else {
        rustls::pki_types::PrivateKeyDer::try_from(bytes.to_vec())
            .map_err(|error| TlsConfigError::at("the DER private key is invalid", error))
    }
}

#[cfg(feature = "hosted-http12")]
fn rustls_roots(custom_ca: Option<&[u8]>) -> Result<rustls::RootCertStore, TlsConfigError> {
    let mut roots = rustls::RootCertStore::empty();
    for certificate in trust_roots(custom_ca)? {
        roots
            .add(rustls::pki_types::CertificateDer::from(certificate))
            .map_err(|error| TlsConfigError::at("a TLS trust root is invalid", error))?;
    }
    Ok(roots)
}

#[cfg(feature = "hosted-http12")]
fn rustls_builder()
-> Result<rustls::ConfigBuilder<rustls::ClientConfig, rustls::WantsVerifier>, TlsConfigError> {
    rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
    .map_err(|error| TlsConfigError::at("the Rustls protocol set is unavailable", error))
}

#[cfg(feature = "hosted-http12")]
pub(super) fn rustls_client(
    custom_ca: Option<&[u8]>,
    identity: Option<TlsIdentity<'_>>,
) -> Result<rustls::ClientConfig, TlsConfigError> {
    let builder = rustls_builder()?.with_root_certificates(rustls_roots(custom_ca)?);
    let mut config = match identity {
        Some(identity) => builder
            .with_client_auth_cert(
                certificate_der(identity.certificate_chain)?
                    .into_iter()
                    .map(rustls::pki_types::CertificateDer::from)
                    .collect(),
                rustls_private_key(identity.private_key)?,
            )
            .map_err(|error| TlsConfigError::at("the TLS client identity is invalid", error))?,
        None => builder.with_no_client_auth(),
    };
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.enable_early_data = false;
    Ok(config)
}

#[cfg(feature = "hosted-http12")]
pub(super) fn rustls_server(
    identity: TlsIdentity<'_>,
) -> Result<rustls::ServerConfig, TlsConfigError> {
    let builder = rustls::ServerConfig::builder_with_provider(std::sync::Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13, &rustls::version::TLS12])
    .map_err(|error| TlsConfigError::at("the Rustls protocol set is unavailable", error))?;
    let mut config = builder
        .with_no_client_auth()
        .with_single_cert(
            certificate_der(identity.certificate_chain)?
                .into_iter()
                .map(rustls::pki_types::CertificateDer::from)
                .collect(),
            rustls_private_key(identity.private_key)?,
        )
        .map_err(|error| TlsConfigError::at("the TLS server identity is invalid", error))?;
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    config.max_early_data_size = 0;
    Ok(config)
}

#[cfg(feature = "hosted-http3")]
fn boring_certificates(bytes: &[u8]) -> Result<Vec<boring::x509::X509>, TlsConfigError> {
    if let Some(pem) = pem_payload(bytes) {
        boring::x509::X509::stack_from_pem(pem)
            .map_err(|error| TlsConfigError::at("the PEM certificate bundle is invalid", error))
    } else {
        boring::x509::X509::from_der(bytes)
            .map(|certificate| vec![certificate])
            .map_err(|error| TlsConfigError::at("the DER certificate is invalid", error))
    }
}

#[cfg(feature = "hosted-http3")]
fn boring_identity(
    builder: &mut boring::ssl::SslContextBuilder,
    identity: TlsIdentity<'_>,
) -> Result<(), TlsConfigError> {
    let mut certificates = boring_certificates(identity.certificate_chain)?.into_iter();
    let leaf = certificates
        .next()
        .ok_or_else(|| TlsConfigError::message("the certificate chain contains no leaf"))?;
    builder
        .set_certificate(&leaf)
        .map_err(|error| TlsConfigError::at("the BoringSSL leaf certificate is invalid", error))?;
    for intermediate in certificates {
        builder
            .add_extra_chain_cert(intermediate)
            .map_err(|error| {
                TlsConfigError::at("a BoringSSL intermediate certificate is invalid", error)
            })?;
    }
    let key = if let Some(pem) = pem_payload(identity.private_key) {
        boring::pkey::PKey::private_key_from_pem(pem)
    } else {
        boring::pkey::PKey::private_key_from_der(identity.private_key)
    }
    .map_err(|error| TlsConfigError::at("the BoringSSL private key is invalid", error))?;
    builder
        .set_private_key(&key)
        .map_err(|error| TlsConfigError::at("BoringSSL rejected the private key", error))?;
    builder
        .check_private_key()
        .map_err(|error| TlsConfigError::at("the certificate and private key do not match", error))
}

#[cfg(feature = "hosted-http3")]
fn boring_client_context(
    custom_ca: Option<&[u8]>,
    identity: Option<TlsIdentity<'_>>,
) -> Result<boring::ssl::SslContextBuilder, TlsConfigError> {
    let mut builder = boring::ssl::SslContextBuilder::new(boring::ssl::SslMethod::tls())
        .map_err(|error| TlsConfigError::at("BoringSSL context construction failed", error))?;
    builder.set_verify(boring::ssl::SslVerifyMode::PEER);
    for certificate in trust_roots(custom_ca)? {
        let certificate = boring::x509::X509::from_der(&certificate)
            .map_err(|error| TlsConfigError::at("a BoringSSL trust root is invalid", error))?;
        builder
            .cert_store_mut()
            .add_cert(certificate)
            .map_err(|error| TlsConfigError::at("BoringSSL refused a trust root", error))?;
    }
    if let Some(identity) = identity {
        boring_identity(&mut builder, identity)?;
    }
    Ok(builder)
}

#[cfg(feature = "hosted-http3")]
fn boring_server_context(
    identity: TlsIdentity<'_>,
) -> Result<boring::ssl::SslContextBuilder, TlsConfigError> {
    let mut builder = boring::ssl::SslContextBuilder::new(boring::ssl::SslMethod::tls())
        .map_err(|error| TlsConfigError::at("BoringSSL context construction failed", error))?;
    builder.set_verify(boring::ssl::SslVerifyMode::NONE);
    boring_identity(&mut builder, identity)?;
    Ok(builder)
}

#[cfg(feature = "hosted-http3")]
fn quiche_transport(
    context: boring::ssl::SslContextBuilder,
    limits: EngineLimits,
) -> Result<quiche::Config, TlsConfigError> {
    let mut config = quiche::Config::with_boring_ssl_ctx_builder(quiche::PROTOCOL_VERSION, context)
        .map_err(|error| TlsConfigError::at("quiche rejected the TLS context", error))?;
    config
        .set_application_protos(HTTP3_ALPN)
        .map_err(|error| TlsConfigError::at("quiche rejected the h3 ALPN", error))?;
    config.set_max_recv_udp_payload_size(MAX_QUIC_DATAGRAM_BYTES);
    config.set_max_send_udp_payload_size(MAX_QUIC_DATAGRAM_BYTES);
    config.set_max_idle_timeout(MAX_QUIC_IDLE_TIMEOUT_MILLIS);
    config.set_initial_max_data(limits.buffered_bytes() as u64);
    config.set_initial_max_stream_data_bidi_local(limits.buffered_bytes() as u64);
    config.set_initial_max_stream_data_bidi_remote(limits.buffered_bytes() as u64);
    config.set_initial_max_stream_data_uni(limits.buffered_bytes() as u64);
    config.set_initial_max_streams_bidi(limits.work_capacity() as u64);
    config.set_initial_max_streams_uni(
        limits
            .control_capacity()
            .max(REQUIRED_H3_UNIDIRECTIONAL_STREAMS) as u64,
    );
    config.set_disable_active_migration(true);
    // The only enabling API is intentionally never called. H2 freezes 0-RTT
    // as disabled because replay policy has not been specified.
    Ok(config)
}

#[cfg(feature = "hosted-http3")]
pub(super) fn quiche_client(
    custom_ca: Option<&[u8]>,
    identity: Option<TlsIdentity<'_>>,
    limits: EngineLimits,
) -> Result<quiche::Config, TlsConfigError> {
    let mut config = quiche_transport(boring_client_context(custom_ca, identity)?, limits)?;
    config.verify_peer(true);
    Ok(config)
}

#[cfg(feature = "hosted-http3")]
pub(super) fn quiche_server(
    identity: TlsIdentity<'_>,
    limits: EngineLimits,
) -> Result<quiche::Config, TlsConfigError> {
    quiche_transport(boring_server_context(identity)?, limits)
}

#[cfg(all(test, feature = "hosted-http12", feature = "hosted-http3"))]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    const CA: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spikes/h3-quinn-http3/certs/ca.crt"
    ));
    const CERTIFICATE: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spikes/h3-quinn-http3/certs/server.crt"
    ));
    const PRIVATE_KEY: &[u8] = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../spikes/h3-quinn-http3/certs/server.key"
    ));

    fn identity() -> TlsIdentity<'static> {
        TlsIdentity {
            certificate_chain: CERTIFICATE,
            private_key: PRIVATE_KEY,
        }
    }

    fn limits() -> EngineLimits {
        EngineLimits::new(8, 2, 8, 64 * 1024).expect("the TLS test limits are valid")
    }

    #[test]
    fn both_tls_stacks_accept_the_same_roots_and_in_memory_identity() {
        let rustls_client = rustls_client(Some(CA), None).expect("Rustls accepts the fixture CA");
        let rustls_server = rustls_server(identity()).expect("Rustls accepts the fixture identity");
        let _quiche_client =
            quiche_client(Some(CA), None, limits()).expect("quiche accepts the fixture CA");
        let _quiche_server =
            quiche_server(identity(), limits()).expect("quiche accepts the fixture identity");
        assert_eq!(
            rustls_client.alpn_protocols,
            [b"h2".to_vec(), b"http/1.1".to_vec()]
        );
        assert_eq!(
            rustls_server.alpn_protocols,
            [b"h2".to_vec(), b"http/1.1".to_vec()]
        );
    }

    #[test]
    fn public_roots_are_one_shared_exact_corpus() {
        let roots = trust_roots(None).expect("the compiled root corpus is valid");
        assert_eq!(roots.len(), webpki_root_certs::TLS_SERVER_ROOT_CERTS.len());
        assert!(rustls_client(None, None).is_ok());
        assert!(quiche_client(None, None, limits()).is_ok());
    }

    #[test]
    fn malformed_tls_material_refuses_before_a_socket_exists() {
        let malformed = TlsIdentity {
            certificate_chain: b"not a certificate",
            private_key: b"not a key",
        };
        assert!(rustls_server(malformed).is_err());
        assert!(quiche_server(malformed, limits()).is_err());
        assert!(rustls_client(Some(b"not a CA"), None).is_err());
        assert!(quiche_client(Some(b"not a CA"), None, limits()).is_err());
    }
}
