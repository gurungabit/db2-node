/// Integration tests for prepared statements.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;

use db2_client::Error;
use db2_proto::types::Db2Value;

#[tokio::test]
async fn test_prepared_select_with_params() {
    let client = connect().await;
    let table = temp_table_name("psel");
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
            &format!(
                "INSERT INTO {} VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Charlie')",
                table
            ),
            &[],
        )
        .await
        .expect("insert rows");

    let stmt = client
        .prepare(&format!("SELECT id, name FROM {} WHERE id = ?", table))
        .await
        .expect("prepare");

    let id_param = Db2Value::Integer(2);
    let result = stmt
        .execute(&[&id_param as &dyn db2_client::ToSql])
        .await
        .expect("execute prepared");
    assert_eq!(result.rows.len(), 1, "should find exactly one row for id=2");

    stmt.close().await.expect("close stmt");
    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_prepared_insert_with_params() {
    let client = connect().await;
    let table = temp_table_name("pins");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, name VARCHAR(100))", table),
            &[],
        )
        .await
        .expect("create table");

    let stmt = client
        .prepare(&format!("INSERT INTO {} VALUES (?, ?)", table))
        .await
        .expect("prepare insert");

    for i in 1..=5 {
        let id_param = Db2Value::Integer(i);
        let name_param = Db2Value::VarChar(format!("user_{}", i));
        stmt.execute(&[
            &id_param as &dyn db2_client::ToSql,
            &name_param as &dyn db2_client::ToSql,
        ])
        .await
        .unwrap_or_else(|_| panic!("execute insert #{}", i));
    }

    stmt.close().await.expect("close stmt");

    let result = client
        .query(&format!("SELECT COUNT(*) FROM {}", table), &[])
        .await
        .expect("count rows");
    assert_eq!(result.rows.len(), 1);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_prepared_null_param() {
    let client = connect().await;
    let table = temp_table_name("pnull");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    let stmt = client
        .prepare(&format!("INSERT INTO {} VALUES (?, ?)", table))
        .await
        .expect("prepare");

    let id_param = Db2Value::Integer(1);
    let null_param = Db2Value::Null;
    stmt.execute(&[
        &id_param as &dyn db2_client::ToSql,
        &null_param as &dyn db2_client::ToSql,
    ])
    .await
    .expect("insert with null param");

    stmt.close().await.expect("close stmt");

    let result = client
        .query(&format!("SELECT val FROM {} WHERE id = 1", table), &[])
        .await
        .expect("select");
    assert_eq!(result.rows.len(), 1);
    let val: Option<String> = result.rows[0].get("VAL");
    assert!(val.is_none(), "NULL param should store NULL");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_prepared_many_params() {
    let client = connect().await;
    let table = temp_table_name("pmany");
    drop_table(&client, &table).await;

    // Create table with 20 columns
    let col_defs: Vec<String> = (1..=20).map(|i| format!("c{} INTEGER", i)).collect();
    client
        .query(
            &format!("CREATE TABLE {} ({})", table, col_defs.join(", ")),
            &[],
        )
        .await
        .expect("create table with 20 columns");

    let placeholders: Vec<&str> = (0..20).map(|_| "?").collect();
    let stmt = client
        .prepare(&format!(
            "INSERT INTO {} VALUES ({})",
            table,
            placeholders.join(", ")
        ))
        .await
        .expect("prepare with 20 params");

    // Build 20 integer parameters
    let params: Vec<Db2Value> = (1..=20).map(Db2Value::Integer).collect();
    let param_refs: Vec<&dyn db2_client::ToSql> =
        params.iter().map(|p| p as &dyn db2_client::ToSql).collect();

    stmt.execute(&param_refs)
        .await
        .expect("execute with 20 params");
    stmt.close().await.expect("close stmt");

    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 20);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_prepared_statement_is_invalid_after_reconnect() {
    let mut client = connect().await;

    let stmt = client.prepare("VALUES 1").await.expect("prepare");

    client.close().await.expect("close client");
    client.connect().await.expect("reconnect client");

    let err = stmt
        .execute(&[])
        .await
        .expect_err("prepared statement from the old session should be invalid after reconnect");
    assert!(
        matches!(err, Error::Connection(ref message) if message.contains("session changed")),
        "expected stale prepared statement error, got {:?}",
        err
    );

    let result = client
        .query("VALUES 1", &[])
        .await
        .expect("reconnected client should still be usable");
    assert_eq!(result.row_count, 1);

    let _ = stmt.close().await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_multiple_prepared_statements_can_coexist_on_one_connection() {
    let client = connect().await;

    let stmt1 = client
        .prepare("VALUES CAST(? AS INTEGER)")
        .await
        .expect("prepare first statement");
    let stmt2 = client
        .prepare("VALUES CAST(? AS INTEGER) + 100")
        .await
        .expect("prepare second statement");

    let one = Db2Value::Integer(1);
    let two = Db2Value::Integer(2);

    let result1 = stmt1
        .execute(&[&one as &dyn db2_client::ToSql])
        .await
        .expect("execute first prepared statement");
    let result2 = stmt2
        .execute(&[&two as &dyn db2_client::ToSql])
        .await
        .expect("execute second prepared statement");

    let value1: i32 = result1.rows[0].get("1").expect("first statement result");
    let value2: i32 = result2.rows[0].get("1").expect("second statement result");

    assert_eq!(
        value1, 1,
        "first prepared statement should keep its own section"
    );
    assert_eq!(
        value2, 102,
        "second prepared statement should keep its own section"
    );

    stmt1.close().await.expect("close first statement");
    stmt2.close().await.expect("close second statement");
    client.close().await.expect("close");
}

/// Verify that section numbers are recycled and the server correctly re-prepares
/// on a reused section. This is the core server-side deallocation mechanism:
/// DRDA doesn't have a per-section deallocate command — re-preparing on the
/// same section overwrites the previous plan on the server.
#[tokio::test]
async fn test_section_reuse_overwrites_server_side_prepared_statement() {
    let client = connect().await;
    let table = temp_table_name("sruse");
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
            &format!("INSERT INTO {} VALUES (1, 'a'), (2, 'b'), (3, 'c')", table),
            &[],
        )
        .await
        .expect("seed data");

    // Prepare stmt1 — note its section number
    let stmt1 = client
        .prepare(&format!("SELECT id FROM {} WHERE id = ?", table))
        .await
        .expect("prepare stmt1");
    let section1 = stmt1.section_number();

    // Close stmt1 — section goes back to the free pool
    stmt1.close().await.expect("close stmt1");

    // Prepare stmt2 with DIFFERENT SQL — should reuse the same section
    let stmt2 = client
        .prepare(&format!("SELECT val FROM {} WHERE id = ?", table))
        .await
        .expect("prepare stmt2");
    let section2 = stmt2.section_number();
    assert_eq!(
        section1, section2,
        "closed section should be reused for next prepare"
    );

    // Execute stmt2 — if the server still has the old plan on this section,
    // the column type/name would be wrong. This proves the overwrite worked.
    let param = Db2Value::Integer(2);
    let result = stmt2
        .execute(&[&param as &dyn db2_client::ToSql])
        .await
        .expect("execute on reused section");

    assert_eq!(result.rows.len(), 1);
    // The column should be 'val' (from stmt2), not 'id' (from stmt1)
    let val: String = result.rows[0]
        .get("VAL")
        .expect("should have VAL column from the new prepared SQL");
    assert_eq!(val, "b", "should get the correct value from the new plan");

    stmt2.close().await.expect("close stmt2");
    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

/// Stress test: cycle through many prepare/close cycles to verify
/// section numbers don't leak and the allocator stays consistent.
#[tokio::test]
async fn test_section_allocator_does_not_leak_across_many_cycles() {
    let client = connect().await;

    let mut seen_sections = std::collections::HashSet::new();

    for i in 0..20 {
        let stmt = client
            .prepare(&format!("VALUES {}", i))
            .await
            .expect("prepare in cycle");
        let sec = stmt.section_number();
        // After the first pass, all sections should be recycled
        if i > 0 {
            assert!(
                seen_sections.contains(&sec),
                "section {} should be recycled from free pool on iteration {}",
                sec,
                i
            );
        }
        seen_sections.insert(sec);
        stmt.close().await.expect("close in cycle");
    }

    // Only section 1 should have ever been allocated (it keeps being reused)
    assert_eq!(
        seen_sections.len(),
        1,
        "sequential prepare/close should reuse the same section"
    );

    client.close().await.expect("close");
}
