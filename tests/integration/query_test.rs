/// Integration tests for SQL query execution.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;
use db2_client::{Client, Error};
use std::time::{Duration, Instant};

#[tokio::test]
async fn test_select_dummy() {
    let client = connect().await;
    let result = client
        .query("VALUES 1", &[])
        .await
        .expect("should execute VALUES query");
    assert_eq!(result.row_count, 1);
    assert_eq!(result.rows.len(), 1);
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_select_multiple_columns() {
    let client = connect().await;
    let result = client
        .query("VALUES (1, 'hello', 3.14)", &[])
        .await
        .expect("should execute multi-column query");
    assert_eq!(result.rows.len(), 1);
    assert!(result.columns.len() >= 3, "should have at least 3 columns");
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_select_empty_result() {
    let client = connect().await;
    let table = temp_table_name("empty");
    drop_table(&client, &table).await;

    client
        .query(
            &format!(
                "CREATE TABLE {} (id INTEGER NOT NULL, name VARCHAR(50))",
                table
            ),
            &[],
        )
        .await
        .expect("create table");

    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select from empty table");
    assert_eq!(result.rows.len(), 0);
    assert_eq!(result.row_count, 0);
    assert!(
        result.columns.len() >= 2,
        "should describe columns even for empty result"
    );

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_select_large_result_set() {
    let client = connect().await;
    let table = temp_table_name("large");
    drop_table(&client, &table).await;

    client
        .query(
            &format!(
                "CREATE TABLE {} (id INTEGER NOT NULL, val VARCHAR(20))",
                table
            ),
            &[],
        )
        .await
        .expect("create table");

    // Insert 10,000 rows using a recursive CTE
    let insert_sql = format!(
        "INSERT INTO {} (id, val) \
         WITH t(n) AS (VALUES 1 UNION ALL SELECT n+1 FROM t WHERE n < 10000) \
         SELECT n, 'row_' || CHAR(n) FROM t",
        table
    );
    client
        .query(&insert_sql, &[])
        .await
        .expect("insert 10K rows");

    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select 10K rows");
    assert_eq!(result.rows.len(), 10_000);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_insert_and_select() {
    let client = connect().await;
    let table = temp_table_name("inssel");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, name VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!("INSERT INTO {} VALUES (1, 'Alice'), (2, 'Bob')", table),
            &[],
        )
        .await
        .expect("insert rows");

    let result = client
        .query(&format!("SELECT id, name FROM {} ORDER BY id", table), &[])
        .await
        .expect("select rows");
    assert_eq!(result.rows.len(), 2);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_update_returns_row_count() {
    let client = connect().await;
    let table = temp_table_name("upd");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val INTEGER)", table),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!("INSERT INTO {} VALUES (1, 10), (2, 20), (3, 30)", table),
            &[],
        )
        .await
        .expect("insert");

    let result = client
        .query(
            &format!("UPDATE {} SET val = val + 1 WHERE id <= 2", table),
            &[],
        )
        .await
        .expect("update");
    assert_eq!(result.row_count, 2, "should report 2 rows updated");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_delete_returns_row_count() {
    let client = connect().await;
    let table = temp_table_name("del");
    drop_table(&client, &table).await;

    client
        .query(&format!("CREATE TABLE {} (id INTEGER)", table), &[])
        .await
        .expect("create table");

    client
        .query(
            &format!("INSERT INTO {} VALUES (1), (2), (3), (4), (5)", table),
            &[],
        )
        .await
        .expect("insert");

    let result = client
        .query(&format!("DELETE FROM {} WHERE id > 2", table), &[])
        .await
        .expect("delete");
    assert_eq!(result.row_count, 3, "should report 3 rows deleted");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_syntax_error() {
    let client = connect().await;
    let err = client
        .query("SELCT * FORM nowhere", &[])
        .await
        .expect_err("malformed SQL should fail");
    let msg = err.to_string();
    // DB2 should return some kind of SQL error
    assert!(
        err.sqlstate().is_some()
            || msg.to_lowercase().contains("sql")
            || msg.to_lowercase().contains("syntax"),
        "Error should indicate SQL syntax problem, got: {}",
        msg
    );
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_runtime_query_error() {
    let client = connect().await;
    let err = client
        .query("SELECT 1/0 AS X FROM SYSIBM.SYSDUMMY1", &[])
        .await
        .expect_err("division by zero should fail");

    match err {
        Error::Sql {
            sqlstate,
            sqlcode: _,
            message,
        } => {
            assert!(!sqlstate.is_empty());
            assert!(!message.is_empty());
        }
        other => panic!("expected SQL error, got {:?}", other),
    }

    client.close().await.expect("close");
}

#[tokio::test]
async fn test_query_timeout_interrupts_blocked_statement() {
    let locker = connect().await;
    let table = temp_table_name("timeout");
    drop_table(&locker, &table).await;

    locker
        .query(
            &format!("CREATE TABLE {} (id INTEGER NOT NULL, val INTEGER)", table),
            &[],
        )
        .await
        .expect("create table");
    locker
        .query(&format!("INSERT INTO {} VALUES (1, 10)", table), &[])
        .await
        .expect("seed row");

    let txn = locker.begin_transaction().await.expect("begin lock holder");
    txn.query(&format!("UPDATE {} SET val = 20 WHERE id = 1", table), &[])
        .await
        .expect("lock row");

    let mut timeout_config = test_config();
    timeout_config.query_timeout = Duration::from_millis(250);
    timeout_config.frame_drain_timeout = Duration::from_millis(50);
    let blocked_client = Client::connect_with(timeout_config)
        .await
        .expect("connect blocked client");

    let started = Instant::now();
    let err = blocked_client
        .query(&format!("UPDATE {} SET val = 30 WHERE id = 1", table), &[])
        .await
        .expect_err("blocked update should time out");

    assert!(
        matches!(err, Error::Timeout(_)),
        "expected timeout error, got {:?}",
        err
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "timed-out query should fail promptly"
    );
    assert!(
        !blocked_client.is_connected().await,
        "timed-out client should be disconnected to avoid protocol desync"
    );

    txn.rollback().await.expect("unlock row");

    let reconnect_result = blocked_client
        .query("VALUES 1", &[])
        .await
        .expect("next client operation should reconnect after a timeout-induced disconnect");
    assert_eq!(reconnect_result.row_count, 1);
    assert!(
        blocked_client.is_connected().await,
        "client should be connected again after the follow-up query"
    );

    drop_table(&locker, &table).await;
    let _ = blocked_client.close().await;
    locker.close().await.expect("close");
}

#[tokio::test]
async fn test_column_metadata() {
    let client = connect().await;
    let table = temp_table_name("colmeta");
    drop_table(&client, &table).await;

    client
        .query(
            &format!(
                "CREATE TABLE {} (\
                    id INTEGER NOT NULL, \
                    name VARCHAR(100), \
                    amount DECIMAL(10,2), \
                    active SMALLINT \
                )",
                table
            ),
            &[],
        )
        .await
        .expect("create table");

    let result = client
        .query(
            &format!("SELECT * FROM {} FETCH FIRST 0 ROWS ONLY", table),
            &[],
        )
        .await
        .expect("select for metadata");

    assert_eq!(result.columns.len(), 4);

    // Verify column names (DB2 uppercases them)
    let names: Vec<&str> = result.columns.iter().map(|c| c.name.as_str()).collect();
    assert!(names.contains(&"ID") || names.contains(&"id"));
    assert!(names.contains(&"NAME") || names.contains(&"name"));
    assert!(names.contains(&"AMOUNT") || names.contains(&"amount"));
    assert!(names.contains(&"ACTIVE") || names.contains(&"active"));

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}
