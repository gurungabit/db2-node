/// Integration tests for prepared statements.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;

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
