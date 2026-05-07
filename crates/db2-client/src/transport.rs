use bytes::BytesMut;
use std::sync::{Arc, OnceLock};
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{debug, trace, warn};

use crate::config::{Config, SslConfig};
use crate::error::Error;

const READ_RESERVE: usize = 64 * 1024;
static DEFAULT_TLS_CLIENT_CONFIG: OnceLock<Result<Arc<rustls::ClientConfig>, String>> =
    OnceLock::new();
static INSECURE_TLS_CLIENT_CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();

/// Transport layer abstraction over TCP and TLS connections.
pub enum Transport {
    Tcp(TcpStream),
    Tls(Box<tokio_rustls::client::TlsStream<TcpStream>>),
}

impl Transport {
    /// Connect to the DB2 server, optionally upgrading to TLS.
    ///
    /// The `connect_timeout` bounds the entire process: TCP connect + TLS handshake.
    pub async fn connect(config: &Config) -> Result<Self, Error> {
        Self::connect_with_diagnostics(config, None).await
    }

    pub(crate) async fn connect_with_diagnostics(
        config: &Config,
        diagnostics: Option<&mut Vec<String>>,
    ) -> Result<Self, Error> {
        let addr = config.addr();
        debug!("Connecting to DB2 server at {}", addr);

        timeout(
            config.connect_timeout,
            Self::connect_inner(config, &addr, diagnostics),
        )
        .await
        .map_err(|_| {
            Error::Timeout(format!(
                "Connection to {} timed out after {:?}",
                addr, config.connect_timeout
            ))
        })?
    }

    /// Inner connection logic (TCP + optional TLS), called under timeout.
    async fn connect_inner(
        config: &Config,
        addr: &str,
        mut diagnostics: Option<&mut Vec<String>>,
    ) -> Result<Self, Error> {
        let tcp_started = diagnostics.as_ref().map(|_| Instant::now());
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Error::Connection(format!("Failed to connect to {}: {}", addr, e)))?;
        push_transport_elapsed(&mut diagnostics, "db2_connect_tcp_ms", tcp_started);

        // Set TCP nodelay for low-latency protocol exchange
        stream
            .set_nodelay(true)
            .map_err(|e| Error::Connection(format!("Failed to set TCP_NODELAY: {}", e)))?;

        debug!("TCP connection established to {}", addr);

        if config.ssl {
            debug!("Upgrading connection to TLS");
            let tls_stream = Self::upgrade_tls(stream, config, diagnostics).await?;
            Ok(Transport::Tls(Box::new(tls_stream)))
        } else {
            Ok(Transport::Tcp(stream))
        }
    }

    /// Upgrade a TCP connection to TLS.
    async fn upgrade_tls(
        stream: TcpStream,
        config: &Config,
        mut diagnostics: Option<&mut Vec<String>>,
    ) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Error> {
        let tls_config_started = diagnostics.as_ref().map(|_| Instant::now());
        let tls_config = Self::build_tls_config(config.ssl_config.as_ref())?;
        push_transport_elapsed(
            &mut diagnostics,
            "db2_connect_tls_config_ms",
            tls_config_started,
        );
        let connector = tokio_rustls::TlsConnector::from(tls_config);

        let server_name = rustls::pki_types::ServerName::try_from(config.host.as_str())
            .map_err(|e| Error::Tls(format!("Invalid server name '{}': {}", config.host, e)))?
            .to_owned();

        let tls_handshake_started = diagnostics.as_ref().map(|_| Instant::now());
        let tls_stream = connector
            .connect(server_name, stream)
            .await
            .map_err(|e| Error::Tls(format!("TLS handshake failed: {}", e)))?;
        push_transport_elapsed(
            &mut diagnostics,
            "db2_connect_tls_handshake_ms",
            tls_handshake_started,
        );

        debug!("TLS connection established");
        Ok(tls_stream)
    }

    /// Warm reusable TLS configuration without opening a socket.
    pub(crate) fn warm_tls_config(config: &Config) {
        if config.ssl {
            let _ = Self::build_tls_config(config.ssl_config.as_ref());
        }
    }

    /// Build the rustls ClientConfig from our SslConfig.
    fn build_tls_config(
        ssl_config: Option<&SslConfig>,
    ) -> Result<Arc<rustls::ClientConfig>, Error> {
        // Ensure the ring crypto provider is installed (idempotent)
        let _ = rustls::crypto::ring::default_provider().install_default();

        if let Some(ssl) = ssl_config {
            if !ssl.reject_unauthorized {
                return Ok(Arc::clone(INSECURE_TLS_CLIENT_CONFIG.get_or_init(|| {
                    Arc::new(
                        rustls::ClientConfig::builder()
                            .dangerous()
                            .with_custom_certificate_verifier(Arc::new(NoVerifier))
                            .with_no_client_auth(),
                    )
                })));
            }
            if ssl.ca_cert.is_some() || !ssl.validate_server_name {
                return build_verified_tls_config(ssl.ca_cert.as_deref(), ssl.validate_server_name)
                    .map(Arc::new)
                    .map_err(Error::Tls);
            }
        }

        match DEFAULT_TLS_CLIENT_CONFIG
            .get_or_init(|| build_verified_tls_config(None, true).map(Arc::new))
        {
            Ok(config) => Ok(Arc::clone(config)),
            Err(message) => Err(Error::Tls(message.clone())),
        }
    }
}

fn push_transport_elapsed(
    diagnostics: &mut Option<&mut Vec<String>>,
    name: &str,
    started: Option<Instant>,
) {
    if let (Some(diagnostics), Some(started)) = (diagnostics.as_deref_mut(), started) {
        diagnostics.push(format!(
            "{}={:.3}",
            name,
            started.elapsed().as_secs_f64() * 1000.0
        ));
    }
}

fn build_verified_tls_config(
    ca_cert_path: Option<&str>,
    validate_server_name: bool,
) -> Result<rustls::ClientConfig, String> {
    let root_store = build_root_store(ca_cert_path)?;

    if !validate_server_name {
        return Ok(rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(NoHostnameVerifier::new(root_store)))
            .with_no_client_auth());
    }

    Ok(rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth())
}

fn build_root_store(ca_cert_path: Option<&str>) -> Result<rustls::RootCertStore, String> {
    let mut root_store = rustls::RootCertStore::empty();
    let native_certs = rustls_native_certs::load_native_certs();
    if !native_certs.errors.is_empty() {
        warn!(
            "Encountered {} error(s) while loading native certificates",
            native_certs.errors.len()
        );
    }
    for cert in native_certs.certs {
        root_store
            .add(cert)
            .map_err(|e| format!("Failed to add native CA cert: {}", e))?;
    }

    if let Some(ca_cert_path) = ca_cert_path {
        let ca_data = std::fs::read(ca_cert_path)
            .map_err(|e| format!("Failed to read CA cert {}: {}", ca_cert_path, e))?;
        let mut cursor = std::io::Cursor::new(ca_data);
        let certs = rustls_pemfile::certs(&mut cursor)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("Failed to parse CA cert: {}", e))?;
        for cert in certs {
            root_store
                .add(cert)
                .map_err(|e| format!("Failed to add CA cert: {}", e))?;
        }
    }

    if root_store.is_empty() {
        return Err("TLS verification is enabled but no root certificates are available".into());
    }

    Ok(root_store)
}

impl Transport {
    /// Read bytes from the transport into the provided buffer.
    /// Returns the number of bytes read (0 means EOF).
    pub async fn read_bytes(&mut self, buf: &mut BytesMut) -> Result<usize, Error> {
        // Ensure we have space to read into
        if buf.capacity() - buf.len() < READ_RESERVE {
            buf.reserve(READ_RESERVE);
        }

        let n = match self {
            Transport::Tcp(stream) => stream.read_buf(buf).await?,
            Transport::Tls(stream) => stream.read_buf(buf).await?,
        };

        trace!("Read {} bytes from transport", n);

        if n == 0 {
            return Err(Error::Connection("Connection closed by server".to_string()));
        }

        Ok(n)
    }

    /// Read at least `min_bytes` into the buffer.
    pub async fn read_at_least(
        &mut self,
        buf: &mut BytesMut,
        min_bytes: usize,
    ) -> Result<(), Error> {
        while buf.len() < min_bytes {
            self.read_bytes(buf).await?;
        }
        Ok(())
    }

    /// Write all bytes to the transport.
    pub async fn write_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        trace!("Writing {} bytes to transport", data.len());
        match self {
            Transport::Tcp(stream) => {
                stream.write_all(data).await?;
                stream.flush().await?;
            }
            Transport::Tls(stream) => {
                stream.write_all(data).await?;
                stream.flush().await?;
            }
        }
        Ok(())
    }

    /// Close the transport connection.
    pub async fn close(&mut self) -> Result<(), Error> {
        debug!("Closing transport connection");
        match self {
            Transport::Tcp(stream) => {
                stream.shutdown().await?;
            }
            Transport::Tls(stream) => {
                stream.shutdown().await?;
            }
        }
        Ok(())
    }
}

/// A TLS certificate verifier that accepts any certificate (for reject_unauthorized=false).
#[derive(Debug)]
struct NoVerifier;

impl rustls::client::danger::ServerCertVerifier for NoVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![
            rustls::SignatureScheme::RSA_PKCS1_SHA256,
            rustls::SignatureScheme::RSA_PKCS1_SHA384,
            rustls::SignatureScheme::RSA_PKCS1_SHA512,
            rustls::SignatureScheme::ECDSA_NISTP256_SHA256,
            rustls::SignatureScheme::ECDSA_NISTP384_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA256,
            rustls::SignatureScheme::RSA_PSS_SHA384,
            rustls::SignatureScheme::RSA_PSS_SHA512,
            rustls::SignatureScheme::ED25519,
        ]
    }
}

/// A TLS verifier that validates the certificate chain but skips hostname/SAN checks.
///
/// This matches IBM CLI/ODBC's explicit `SSLClientHostnameValidation=OFF` mode:
/// trust must still chain to system roots or `SSLServerCertificate`/`caCert`, but
/// the certificate does not need a subjectAltName for the connected host.
#[derive(Debug)]
struct NoHostnameVerifier {
    roots: rustls::RootCertStore,
    supported: rustls::crypto::WebPkiSupportedAlgorithms,
}

impl NoHostnameVerifier {
    fn new(roots: rustls::RootCertStore) -> Self {
        Self {
            roots,
            supported: rustls::crypto::ring::default_provider().signature_verification_algorithms,
        }
    }
}

impl rustls::client::danger::ServerCertVerifier for NoHostnameVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        ocsp_response: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let cert = rustls::server::ParsedCertificate::try_from(end_entity)?;
        rustls::client::verify_server_cert_signed_by_trust_anchor(
            &cert,
            &self.roots,
            intermediates,
            now,
            self.supported.all,
        )?;

        if !ocsp_response.is_empty() {
            trace!("Unvalidated OCSP response: {:?}", ocsp_response.to_vec());
        }

        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.supported)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.supported)
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.supported.supported_schemes()
    }
}
