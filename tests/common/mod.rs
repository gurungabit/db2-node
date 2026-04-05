#![allow(dead_code)]
use db2_client::{Client, Config, Pool, PoolConfig};
use std::env;
use std::sync::Once;

static INIT: Once = Once::new();

/// Initialize test environment (logging, etc.). Called at most once.
pub fn init() {
    INIT.call_once(|| {
        // Initialize tracing or env_logger if desired.
        let _ = std::io::stderr(); // placeholder
    });
}

/// Build a Config from environment variables with sensible defaults.
pub fn test_config() -> Config {
    Config {
        host: env::var("DB2_TEST_HOST").unwrap_or_else(|_| "localhost".into()),
        port: env::var("DB2_TEST_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(50000),
        database: env::var("DB2_TEST_DATABASE").unwrap_or_else(|_| "testdb".into()),
        user: env::var("DB2_TEST_USER").unwrap_or_else(|_| "db2inst1".into()),
        password: env::var("DB2_TEST_PASSWORD").unwrap_or_else(|_| "db2wire_test_pw".into()),
        ssl: false,
        ssl_config: None,
        connect_timeout: std::time::Duration::from_secs(10),
        query_timeout: std::time::Duration::from_secs(30),
        fetch_size: 100,
        current_schema: None,
    }
}

/// Connect a Client using default test config.
pub async fn connect() -> Client {
    init();
    let config = test_config();
    let mut client = Client::new(config);
    client.connect().await.expect("Failed to connect to DB2");
    client
}

/// Create a connection pool with the given max_connections.
pub async fn create_pool(max_connections: u32) -> Pool {
    init();
    let config = test_config();
    Pool::new(PoolConfig {
        connection: config,
        min_connections: 0,
        max_connections,
        idle_timeout: std::time::Duration::from_secs(60),
        max_lifetime: std::time::Duration::from_secs(300),
    })
    .await
    .expect("Failed to create pool")
}

/// Execute SQL and ignore any errors (useful for DROP TABLE IF EXISTS).
pub async fn exec_ignore(client: &Client, sql: &str) {
    let _ = client.query(sql, &[]).await;
}

/// Generate a temporary table name with a timestamp suffix to avoid collisions.
pub fn temp_table_name(prefix: &str) -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis();
    format!("tmp_{}_{}", prefix, ts % 1_000_000)
}

/// Drop a table, ignoring errors if it does not exist.
pub async fn drop_table(client: &Client, table: &str) {
    exec_ignore(client, &format!("DROP TABLE {}", table)).await;
}
