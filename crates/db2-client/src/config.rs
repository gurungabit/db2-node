use std::time::Duration;

/// Configuration for connecting to a DB2 database.
#[derive(Debug, Clone)]
pub struct Config {
    pub host: String,
    pub port: u16,
    pub database: String,
    pub user: String,
    pub password: String,
    pub ssl: bool,
    pub ssl_config: Option<SslConfig>,
    pub connect_timeout: Duration,
    pub query_timeout: Duration,
    pub frame_drain_timeout: Duration,
    pub fetch_size: u32,
    pub current_schema: Option<String>,
}

/// TLS/SSL configuration options.
#[derive(Debug, Clone)]
pub struct SslConfig {
    pub ca_cert: Option<String>,
    pub client_cert: Option<String>,
    pub client_key: Option<String>,
    pub reject_unauthorized: bool,
}

impl Default for SslConfig {
    fn default() -> Self {
        SslConfig {
            ca_cert: None,
            client_cert: None,
            client_key: None,
            reject_unauthorized: true,
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
            ssl: false,
            ssl_config: None,
            connect_timeout: Duration::from_secs(30),
            query_timeout: Duration::from_secs(0),
            frame_drain_timeout: Duration::from_millis(500),
            fetch_size: 100,
            current_schema: None,
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
