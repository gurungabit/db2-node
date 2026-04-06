use bytes::BytesMut;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use tracing::{debug, trace, warn};

use crate::config::{Config, SslConfig};
use crate::error::Error;

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
        let addr = config.addr();
        debug!("Connecting to DB2 server at {}", addr);

        timeout(config.connect_timeout, Self::connect_inner(config, &addr))
            .await
            .map_err(|_| {
                Error::Timeout(format!(
                    "Connection to {} timed out after {:?}",
                    addr, config.connect_timeout
                ))
            })?
    }

    /// Inner connection logic (TCP + optional TLS), called under timeout.
    async fn connect_inner(config: &Config, addr: &str) -> Result<Self, Error> {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Error::Connection(format!("Failed to connect to {}: {}", addr, e)))?;

        // Set TCP nodelay for low-latency protocol exchange
        stream
            .set_nodelay(true)
            .map_err(|e| Error::Connection(format!("Failed to set TCP_NODELAY: {}", e)))?;

        debug!("TCP connection established to {}", addr);

        if config.ssl {
            debug!("Upgrading connection to TLS");
            let tls_stream = Self::upgrade_tls(stream, config).await?;
            Ok(Transport::Tls(Box::new(tls_stream)))
        } else {
            Ok(Transport::Tcp(stream))
        }
    }

    /// Upgrade a TCP connection to TLS.
    async fn upgrade_tls(
        stream: TcpStream,
        config: &Config,
    ) -> Result<tokio_rustls::client::TlsStream<TcpStream>, Error> {
        let tls_config = Self::build_tls_config(config.ssl_config.as_ref())?;
        let connector = tokio_rustls::TlsConnector::from(Arc::new(tls_config));

        let server_name = rustls::pki_types::ServerName::try_from(config.host.as_str())
            .map_err(|e| Error::Tls(format!("Invalid server name '{}': {}", config.host, e)))?
            .to_owned();

        let tls_stream = connector
            .connect(server_name, stream)
            .await
            .map_err(|e| Error::Tls(format!("TLS handshake failed: {}", e)))?;

        debug!("TLS connection established");
        Ok(tls_stream)
    }

    /// Build the rustls ClientConfig from our SslConfig.
    fn build_tls_config(ssl_config: Option<&SslConfig>) -> Result<rustls::ClientConfig, Error> {
        // Ensure the ring crypto provider is installed (idempotent)
        let _ = rustls::crypto::ring::default_provider().install_default();

        let builder = rustls::ClientConfig::builder();

        if let Some(ssl) = ssl_config {
            if !ssl.reject_unauthorized {
                // If not rejecting unauthorized, build a config that skips server verification
                let config = builder
                    .dangerous()
                    .with_custom_certificate_verifier(Arc::new(NoVerifier))
                    .with_no_client_auth();
                return Ok(config);
            }
        }

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
                .map_err(|e| Error::Tls(format!("Failed to add native CA cert: {}", e)))?;
        }

        if let Some(ssl) = ssl_config {
            if let Some(ca_cert_path) = &ssl.ca_cert {
                let ca_data = std::fs::read(ca_cert_path).map_err(|e| {
                    Error::Tls(format!("Failed to read CA cert {}: {}", ca_cert_path, e))
                })?;
                let mut cursor = std::io::Cursor::new(ca_data);
                let certs = rustls_pemfile::certs(&mut cursor)
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| Error::Tls(format!("Failed to parse CA cert: {}", e)))?;
                for cert in certs {
                    root_store
                        .add(cert)
                        .map_err(|e| Error::Tls(format!("Failed to add CA cert: {}", e)))?;
                }
            }
        }

        if root_store.is_empty() {
            return Err(Error::Tls(
                "TLS verification is enabled but no root certificates are available".into(),
            ));
        }

        let config = builder
            .with_root_certificates(root_store)
            .with_no_client_auth();

        Ok(config)
    }

    /// Read bytes from the transport into the provided buffer.
    /// Returns the number of bytes read (0 means EOF).
    pub async fn read_bytes(&mut self, buf: &mut BytesMut) -> Result<usize, Error> {
        // Ensure we have space to read into
        if buf.capacity() - buf.len() < 4096 {
            buf.reserve(4096);
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
