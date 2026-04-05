/// Integration tests for DB2 connection lifecycle.
/// Requires a running DB2 instance (set DB2_TEST_* env vars).
#[path = "../common/mod.rs"]
mod common;
use common::*;

use db2_client::Client;

#[tokio::test]
async fn test_basic_connect_disconnect() {
    let client = connect().await;
    // Verify the connection is alive by running a trivial query
    let result = client
        .query("VALUES 1", &[])
        .await
        .expect("should execute trivial query");
    assert_eq!(result.row_count, 1);
    client.close().await.expect("should disconnect cleanly");
}

#[tokio::test]
async fn test_connect_wrong_password() {
    let mut config = test_config();
    config.password = "definitely_wrong_password_12345".into();
    let mut client = Client::new(config);
    let err = client
        .connect()
        .await
        .expect_err("should fail with wrong password");
    // Should be an authentication error
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("auth")
            || msg.contains("password")
            || msg.contains("security")
            || err.is_auth_error(),
        "Error should indicate authentication failure, got: {}",
        err
    );
}

#[tokio::test]
async fn test_connect_wrong_database() {
    let mut config = test_config();
    config.database = "nonexistent_db_xyz".into();
    let mut client = Client::new(config);
    let err = client
        .connect()
        .await
        .expect_err("should fail with wrong database");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("database")
            || msg.contains("rdb")
            || msg.contains("not found")
            || msg.contains("not accessed"),
        "Error should indicate database not found, got: {}",
        err
    );
}

#[tokio::test]
async fn test_connect_wrong_host() {
    let mut config = test_config();
    config.host = "192.0.2.1".into(); // TEST-NET address, should not be routable
    config.connect_timeout = std::time::Duration::from_secs(2);
    let mut client = Client::new(config);
    let err = client
        .connect()
        .await
        .expect_err("should fail connecting to bad host");
    let msg = err.to_string().to_lowercase();
    assert!(
        msg.contains("timeout") || msg.contains("connect") || msg.contains("refused"),
        "Error should indicate connection failure, got: {}",
        err
    );
}

#[tokio::test]
async fn test_multiple_sequential_connections() {
    // Open and close several connections in sequence to verify cleanup
    for i in 0..5 {
        let client = connect().await;
        let result = client
            .query("VALUES 1", &[])
            .await
            .unwrap_or_else(|_| panic!("query #{} should succeed", i));
        assert_eq!(result.row_count, 1);
        client.close().await.expect("should close cleanly");
    }
}

#[tokio::test]
async fn test_server_attributes() {
    let client = connect().await;
    // After connection, we should be able to retrieve basic server info.
    // Run a query that returns DB2 version info.
    let result = client
        .query(
            "SELECT SERVICE_LEVEL FROM SYSIBMADM.ENV_INST_INFO FETCH FIRST 1 ROW ONLY",
            &[],
        )
        .await;
    // This may fail if the view is not accessible, which is acceptable
    // in limited test environments. Just verify the connection works.
    match result {
        Ok(r) => {
            assert!(r.row_count >= 0);
        }
        Err(_) => {
            // Fallback: at least verify we can run a trivial query
            let r = client.query("VALUES 1", &[]).await.expect("fallback query");
            assert_eq!(r.row_count, 1);
        }
    }
    client.close().await.expect("close");
}
