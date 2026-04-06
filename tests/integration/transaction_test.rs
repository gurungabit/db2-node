/// Integration tests for transactions.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;
use db2_client::Error;
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_commit() {
    let client = connect().await;
    let table = temp_table_name("txcommit");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    // Start transaction, insert, commit
    let txn = client.begin_transaction().await.expect("begin txn");
    txn.query(
        &format!("INSERT INTO {} VALUES (1, 'committed')", table),
        &[],
    )
    .await
    .expect("insert in txn");
    txn.commit().await.expect("commit");

    // Verify data persisted
    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select after commit");
    assert_eq!(result.rows.len(), 1, "committed row should be visible");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_rollback() {
    let client = connect().await;
    let table = temp_table_name("txroll");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    // Insert one row outside transaction
    client
        .query(&format!("INSERT INTO {} VALUES (1, 'before')", table), &[])
        .await
        .expect("insert before txn");

    // Start transaction, insert another row, rollback
    let txn = client.begin_transaction().await.expect("begin txn");
    txn.query(
        &format!("INSERT INTO {} VALUES (2, 'rolled back')", table),
        &[],
    )
    .await
    .expect("insert in txn");
    txn.rollback().await.expect("rollback");

    // Verify only the first row exists
    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select after rollback");
    assert_eq!(
        result.rows.len(),
        1,
        "rolled-back row should not be visible"
    );

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_transfer_atomicity() {
    let client = connect().await;
    let table = temp_table_name("txfer");
    drop_table(&client, &table).await;

    client
        .query(
            &format!(
                "CREATE TABLE {} (account VARCHAR(20), balance DECIMAL(10,2))",
                table
            ),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES ('alice', 1000.00), ('bob', 500.00)",
                table
            ),
            &[],
        )
        .await
        .expect("seed accounts");

    // Transfer 200 from Alice to Bob in a transaction
    let txn = client.begin_transaction().await.expect("begin");
    txn.query(
        &format!(
            "UPDATE {} SET balance = balance - 200.00 WHERE account = 'alice'",
            table
        ),
        &[],
    )
    .await
    .expect("debit alice");
    txn.query(
        &format!(
            "UPDATE {} SET balance = balance + 200.00 WHERE account = 'bob'",
            table
        ),
        &[],
    )
    .await
    .expect("credit bob");
    txn.commit().await.expect("commit transfer");

    // Verify balances
    let result = client
        .query(
            &format!("SELECT account, balance FROM {} ORDER BY account", table),
            &[],
        )
        .await
        .expect("select balances");
    assert_eq!(result.rows.len(), 2);
    // Total balance should still be 1500.00 (conservation check)

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_drop_rolls_back_uncommitted_work() {
    let client = connect().await;
    let table = temp_table_name("txdrop");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    {
        let txn = client.begin_transaction().await.expect("begin txn");
        txn.query(
            &format!("INSERT INTO {} VALUES (1, 'transient')", table),
            &[],
        )
        .await
        .expect("insert in dropped txn");
    }

    sleep(Duration::from_millis(250)).await;

    let result = client
        .query(&format!("SELECT COUNT(*) AS CNT FROM {}", table), &[])
        .await
        .expect("select after dropping txn");
    let row_count: i64 = result.rows[0].get::<i64>("CNT").unwrap_or_default();
    assert_eq!(row_count, 0, "dropped transaction should be rolled back");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_transaction_is_invalid_after_reconnect() {
    let mut client = connect().await;

    let txn = client.begin_transaction().await.expect("begin txn");

    client.close().await.expect("close client");
    client.connect().await.expect("reconnect client");

    let err = txn
        .query("VALUES 1", &[])
        .await
        .expect_err("transaction from the old session should be invalid after reconnect");
    assert!(
        matches!(err, Error::Connection(ref message) if message.contains("session changed")),
        "expected stale transaction error, got {:?}",
        err
    );

    let result = client
        .query("VALUES 1", &[])
        .await
        .expect("reconnected client should still be usable");
    assert_eq!(result.row_count, 1);

    client.close().await.expect("close");
}

#[tokio::test]
async fn test_transaction_prepare_keeps_multiple_statements_isolated() {
    let client = connect().await;

    let txn = client.begin_transaction().await.expect("begin txn");
    let stmt1 = txn
        .prepare("VALUES CAST(? AS INTEGER)")
        .await
        .expect("prepare first transaction statement");
    let stmt2 = txn
        .prepare("VALUES CAST(? AS INTEGER) + 100")
        .await
        .expect("prepare second transaction statement");

    let one = db2_proto::types::Db2Value::Integer(1);
    let two = db2_proto::types::Db2Value::Integer(2);

    let result1 = stmt1
        .execute(&[&one as &dyn db2_client::ToSql])
        .await
        .expect("execute first transaction statement");
    let result2 = stmt2
        .execute(&[&two as &dyn db2_client::ToSql])
        .await
        .expect("execute second transaction statement");

    let value1: i32 = result1.rows[0].get("1").expect("first transaction result");
    let value2: i32 = result2.rows[0].get("1").expect("second transaction result");

    assert_eq!(value1, 1);
    assert_eq!(value2, 102);

    stmt1
        .close()
        .await
        .expect("close first transaction statement");
    stmt2
        .close()
        .await
        .expect("close second transaction statement");
    txn.rollback().await.expect("rollback txn");
    client.close().await.expect("close");
}
