use std::time::Duration;

/// Configuration for connecting to a DB2 database.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub security_mechanism: SecurityMechanism,
    pub encryption_algorithm: EncryptionAlgorithm,
    pub credential_encoding: CredentialEncoding,
    pub encrypted_password_encoding: EncryptedPasswordEncoding,
    pub encrypted_password_token_encoding: EncryptedPasswordEncoding,
    pub ssl: bool,
    pub ssl_config: Option<SslConfig>,
    pub connect_timeout: Duration,
    pub query_timeout: Duration,
    pub frame_drain_timeout: Duration,
    pub fetch_size: u32,
    pub current_schema: Option<String>,
    pub type_definition_name: Option<String>,
}

/// DRDA security mechanism to request during authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SecurityMechanism {
    /// Encrypted user ID and password (DRDA SECMEC 9).
    EncryptedUserPassword,
    /// Clear user ID and encrypted password (DRDA SECMEC 7).
    EncryptedPassword,
    /// User ID and password (DRDA SECMEC 3).
    UserPassword,
    /// User ID only (DRDA SECMEC 4).
    UserOnly,
}

/// DRDA encryption algorithm for encrypted authentication credentials.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptionAlgorithm {
    /// Data Encryption Standard (classic DRDA default).
    Des,
    /// Advanced Encryption Standard with a 256-bit key.
    Aes,
}

/// Encoding to use for DRDA credential string bytes in SECCHK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialEncoding {
    /// Follow the server's negotiated Unicode manager support.
    Auto,
    /// EBCDIC code page 037.
    Ebcdic037,
    /// UTF-8.
    Utf8,
}

/// Encoding override for the password bytes used by SECMEC 7.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EncryptedPasswordEncoding {
    /// Use the same effective value as `credential_encoding`.
    SameAsCredential,
    /// EBCDIC code page 037.
    Ebcdic037,
    /// UTF-8.
    Utf8,
}

/// TLS/SSL configuration options.
#[derive(Debug, Clone)]
pub struct SslConfig {
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub reject_unauthorized: bool,
    /// Verify the connected hostname against the server certificate SAN/CN.
    ///
    /// IBM CLI/ODBC exposes this as SSLClientHostnameValidation.  Set this to
    /// false to keep certificate-chain verification while accepting DB2 server
    /// certificates that do not contain a matching hostname.
    pub validate_server_name: bool,
}

impl Default for SslConfig {
    fn default() -> Self {
        SslConfig {
            ca_cert: None,
            client_cert: None,
            client_key: None,
            reject_unauthorized: true,
            validate_server_name: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            host: "localhost".into(),
            port: 50000,
            database: String::new(),
            user: String::new(),
            password: String::new(),
            security_mechanism: SecurityMechanism::EncryptedUserPassword,
            encryption_algorithm: EncryptionAlgorithm::Des,
            credential_encoding: CredentialEncoding::Auto,
            encrypted_password_encoding: EncryptedPasswordEncoding::SameAsCredential,
            encrypted_password_token_encoding: EncryptedPasswordEncoding::SameAsCredential,
            ssl: false,
            ssl_config: None,
            connect_timeout: Duration::from_secs(30),
            query_timeout: Duration::from_secs(0),
            frame_drain_timeout: Duration::from_millis(25),
            fetch_size: 100,
            current_schema: None,
            type_definition_name: None,
        }
    }
}

impl Config {
    /// Create a new Config with required fields.
    pub fn new(host: &str, port: u16, database: &str, user: &str, password: &str) -> Self {
        Config {
            host: host.to_string(),
            port,
            database: database.to_string(),
            user: user.to_string(),
            password: password.to_string(),
            ..Default::default()
        }
    }

    /// Returns the socket address string "host:port".
    pub fn addr(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Set SSL configuration.
    pub fn with_ssl(mut self, ssl_config: SslConfig) -> Self {
        self.ssl = true;
        self.ssl_config = Some(ssl_config);
        self
    }

    /// Set the query timeout.
    pub fn with_query_timeout(mut self, timeout: Duration) -> Self {
        self.query_timeout = timeout;
        self
    }

    /// Set the timeout used when opportunistically draining follow-up reply frames.
    pub fn with_frame_drain_timeout(mut self, timeout: Duration) -> Self {
        self.frame_drain_timeout = timeout;
        self
    }

    /// Set the current schema.
    pub fn with_schema(mut self, schema: &str) -> Self {
        self.current_schema = Some(schema.to_string());
        self
    }
}
