/// Edge case integration tests.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;

#[tokio::test]
async fn test_empty_string_vs_null() {
    let client = connect().await;
    let table = temp_table_name("emptystr");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!("INSERT INTO {} VALUES (1, ''), (2, NULL)", table),
            &[],
        )
        .await
        .expect("insert empty string and null");

    let result = client
        .query(&format!("SELECT id, val FROM {} ORDER BY id", table), &[])
        .await
        .expect("select");
    assert_eq!(result.rows.len(), 2);

    // Row 1 should have empty string (not null) — or in some DB2 configs, null
    let row1_val: Option<String> = result.rows[0].get("VAL");
    // Acceptable: Some("") or None (some DB2 configs treat empty string as null)
    if let Some(ref s) = row1_val {
        assert!(
            s.is_empty() || s.trim().is_empty(),
            "should be empty string, got: '{}'",
            s
        );
    }

    // Row 2 should be null
    let row2_val: Option<String> = result.rows[1].get("VAL");
    assert!(row2_val.is_none(), "NULL row should be None");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_very_long_sql() {
    let client = connect().await;

    // Build a ~40KB SQL statement using string concatenation
    let long_string: String = "x".repeat(39_000);
    let sql = format!("VALUES ('{}')", long_string);
    assert!(sql.len() > 39_000, "SQL should be > 39KB");

    let result = client
        .query(&sql, &[])
        .await
        .expect("long SQL should execute");
    assert_eq!(result.rows.len(), 1);

    client.close().await.expect("close");
}

#[tokio::test]
async fn test_special_characters_in_data() {
    let client = connect().await;
    let table = temp_table_name("special");
    drop_table(&client, &table).await;

    client
        .query(&format!("CREATE TABLE {} (val VARCHAR(200))", table), &[])
        .await
        .expect("create table");

    // Test various special characters (escaped for SQL)
    let test_values = vec![
        "hello''world",        // single quote (escaped)
        "line1\nline2",        // newline is just data
        "tab\there",           // tab
        "back\\slash",         // backslash
        "percent%underscore_", // SQL wildcards as data
    ];

    for val in &test_values {
        client
            .query(&format!("INSERT INTO {} VALUES ('{}')", table, val), &[])
            .await
            .unwrap_or_else(|_| panic!("insert special chars: {}", val));
    }

    let result = client
        .query(&format!("SELECT COUNT(*) FROM {}", table), &[])
        .await
        .expect("count");
    assert_eq!(result.rows.len(), 1);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_rapid_prepare_close() {
    let client = connect().await;

    // Rapidly prepare and close 100 statements to test resource cleanup
    for i in 0..100 {
        let stmt = client
            .prepare(&format!("VALUES {}", i))
            .await
            .unwrap_or_else(|_| panic!("prepare #{}", i));
        stmt.close()
            .await
            .unwrap_or_else(|_| panic!("close stmt #{}", i));
    }

    // Verify the connection is still healthy after all that churn
    let result = client
        .query("VALUES 1", &[])
        .await
        .expect("query after rapid prepare/close");
    assert_eq!(result.row_count, 1);

    client.close().await.expect("close");
}

#[tokio::test]
async fn test_multiple_result_sets_sequentially() {
    let client = connect().await;

    // Execute multiple queries sequentially on the same connection
    for i in 1..=20 {
        let result = client
            .query(&format!("VALUES {}", i), &[])
            .await
            .unwrap_or_else(|_| panic!("query #{}", i));
        assert_eq!(result.row_count, 1);
    }

    client.close().await.expect("close");
}
