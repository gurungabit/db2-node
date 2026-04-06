/// Integration tests for TLS/SSL connections to DB2.
/// Requires a running DB2 instance with SSL enabled.
///
/// These tests are gated on `DB2_TEST_SSL_PORT` being set in the environment.
/// To run locally:
///   cd docker/tls && bash generate-certs.sh && bash setup-db2-ssl.sh
///   DB2_TEST_SSL_PORT=50001 cargo test -p db2-integration-tests --test tls_test -- --nocapture
#[path = "../common/mod.rs"]
mod common;

use db2_client::{Client, Config, SslConfig};
use std::env;

/// Returns the SSL port if configured, or skips the test.
fn ssl_port() -> Option<u16> {
    env::var("DB2_TEST_SSL_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
}

/// Build a Config targeting the SSL port with the given SslConfig.
fn ssl_config(ssl_cfg: SslConfig) -> Config {
    let base = common::test_config();
    Config {
        port: ssl_port().unwrap(),
        ssl: true,
        ssl_config: Some(ssl_cfg),
        ..base
    }
}

/// Path to the test CA certificate used to sign the server cert.
fn ca_cert_path() -> String {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    format!("{}/../../docker/tls/ca.pem", manifest_dir)
}

#[tokio::test]
async fn test_tls_connect_reject_unauthorized_false() {
    if ssl_port().is_none() {
        eprintln!("SKIP: DB2_TEST_SSL_PORT not set");
        return;
    }

    let config = ssl_config(SslConfig {
        reject_unauthorized: false,
        ..Default::default()
    });

    let mut client = Client::new(config);
    client
        .connect()
        .await
        .expect("TLS connect with reject_unauthorized=false should succeed");

    let result = client.query("VALUES 1", &[]).await.expect("query over TLS");
    assert_eq!(result.row_count, 1);

    client.close().await.expect("close TLS connection");
}

#[tokio::test]
async fn test_tls_connect_with_ca_cert() {
    if ssl_port().is_none() {
        eprintln!("SKIP: DB2_TEST_SSL_PORT not set");
        return;
    }

    let ca_path = ca_cert_path();
    if !std::path::Path::new(&ca_path).exists() {
        eprintln!("SKIP: CA cert not found at {}", ca_path);
        return;
    }

    let config = ssl_config(SslConfig {
        ca_cert: Some(ca_path),
        reject_unauthorized: true,
        ..Default::default()
    });

    let mut client = Client::new(config);
    client
        .connect()
        .await
        .expect("TLS connect with custom CA cert should succeed");

    let result = client
        .query("VALUES 'tls-ok'", &[])
        .await
        .expect("query over verified TLS");
    assert_eq!(result.row_count, 1);

    client.close().await.expect("close verified TLS connection");
}

#[tokio::test]
async fn test_tls_query_and_prepared_statements() {
    if ssl_port().is_none() {
        eprintln!("SKIP: DB2_TEST_SSL_PORT not set");
        return;
    }

    let config = ssl_config(SslConfig {
        reject_unauthorized: false,
        ..Default::default()
    });

    let mut client = Client::new(config);
    client.connect().await.expect("connect");

    // Test prepared statements over TLS
    let stmt = client
        .prepare("VALUES CAST(? AS INTEGER) + 10")
        .await
        .expect("prepare over TLS");

    let param = db2_proto::types::Db2Value::Integer(5);
    let result = stmt
        .execute(&[&param as &dyn db2_client::ToSql])
        .await
        .expect("execute prepared over TLS");

    let val: i32 = result.rows[0].get("1").expect("result column");
    assert_eq!(val, 15, "prepared statement should work over TLS");

    stmt.close().await.expect("close stmt");
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_tls_connection_to_non_ssl_port_fails() {
    if ssl_port().is_none() {
        eprintln!("SKIP: DB2_TEST_SSL_PORT not set");
        return;
    }

    // TLS-connect to the plain TCP port. connect_timeout now covers the
    // entire TCP + TLS handshake, so the driver will time out on its own
    // without needing an external wrapper.
    let base = common::test_config();
    let config = Config {
        ssl: true,
        ssl_config: Some(SslConfig {
            reject_unauthorized: false,
            ..Default::default()
        }),
        connect_timeout: std::time::Duration::from_secs(3),
        ..base
    };

    let mut client = Client::new(config);
    let err = client
        .connect()
        .await
        .expect_err("TLS handshake to non-SSL port should fail");

    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("timeout") || msg.contains("tls") || msg.contains("handshake"),
        "Expected timeout or TLS error, got: {}",
        err
    );
}

#[tokio::test]
async fn test_tls_reject_unauthorized_true_without_ca_fails() {
    if ssl_port().is_none() {
        eprintln!("SKIP: DB2_TEST_SSL_PORT not set");
        return;
    }

    // Connect with reject_unauthorized=true but no custom CA.
    // Self-signed cert won't be in system store, so this should fail.
    let config = ssl_config(SslConfig {
        reject_unauthorized: true,
        ca_cert: None,
        ..Default::default()
    });

    let mut client = Client::new(config);
    let result = client.connect().await;
    assert!(
        result.is_err(),
        "TLS with reject_unauthorized=true and self-signed cert should fail without custom CA"
    );
}

#[tokio::test]
async fn test_tls_server_info_available() {
    if ssl_port().is_none() {
        eprintln!("SKIP: DB2_TEST_SSL_PORT not set");
        return;
    }

    let config = ssl_config(SslConfig {
        reject_unauthorized: false,
        ..Default::default()
    });

    let mut client = Client::new(config);
    client.connect().await.expect("connect");

    let info = client.server_info().await.expect("should have server info");
    assert!(
        !info.product_name.is_empty(),
        "server info should be populated over TLS"
    );

    client.close().await.expect("close");
}
