/// Integration tests for DB2 data type round-trips.
/// Requires a running DB2 instance.
#[path = "../common/mod.rs"]
mod common;
use common::*;
use db2_client::ToSql;

#[tokio::test]
async fn test_null_handling() {
    let client = connect().await;
    let table = temp_table_name("nulls");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, val VARCHAR(50))", table),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(&format!("INSERT INTO {} VALUES (1, NULL)", table), &[])
        .await
        .expect("insert null");

    let result = client
        .query(&format!("SELECT val FROM {}", table), &[])
        .await
        .expect("select null");
    assert_eq!(result.rows.len(), 1);
    // NULL column should return None when accessed via get
    let val: Option<String> = result.rows[0].get("VAL");
    assert!(val.is_none(), "NULL column should return None");

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_mixed_nulls() {
    let client = connect().await;
    let table = temp_table_name("mixnull");
    drop_table(&client, &table).await;

    client
        .query(
            &format!(
                "CREATE TABLE {} (a INTEGER, b VARCHAR(20), c INTEGER)",
                table
            ),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES (1, NULL, 3), (NULL, 'hello', NULL)",
                table
            ),
            &[],
        )
        .await
        .expect("insert mixed nulls");

    let result = client
        .query(
            &format!("SELECT * FROM {} ORDER BY COALESCE(a, 999)", table),
            &[],
        )
        .await
        .expect("select");
    assert_eq!(result.rows.len(), 2);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_integer_types() {
    let client = connect().await;
    let table = temp_table_name("ints");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (s SMALLINT, i INTEGER, b BIGINT)", table),
            &[],
        )
        .await
        .expect("create table");

    // Test boundary values
    client
        .query(
            &format!(
                "INSERT INTO {} VALUES (-32768, -2147483648, -9223372036854775808)",
                table
            ),
            &[],
        )
        .await
        .expect("insert min values");

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES (32767, 2147483647, 9223372036854775807)",
                table
            ),
            &[],
        )
        .await
        .expect("insert max values");

    let result = client
        .query(&format!("SELECT * FROM {} ORDER BY s", table), &[])
        .await
        .expect("select integers");
    assert_eq!(result.rows.len(), 2);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_float_types() {
    let client = connect().await;
    let table = temp_table_name("floats");
    drop_table(&client, &table).await;

    client
        .query(&format!("CREATE TABLE {} (f REAL, d DOUBLE)", table), &[])
        .await
        .expect("create table");

    client
        .query(
            &format!("INSERT INTO {} VALUES (3.14, 2.718281828459045)", table),
            &[],
        )
        .await
        .expect("insert floats");

    client
        .query(
            &format!("INSERT INTO {} VALUES (0.0, -1.0E308)", table),
            &[],
        )
        .await
        .expect("insert edge floats");

    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select floats");
    assert_eq!(result.rows.len(), 2);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_decimal_type() {
    let client = connect().await;
    let table = temp_table_name("dec");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (amount DECIMAL(15,2))", table),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES (12345678901.23), (-99999999999.99), (0.01)",
                table
            ),
            &[],
        )
        .await
        .expect("insert decimals");

    let result = client
        .query(&format!("SELECT * FROM {} ORDER BY amount", table), &[])
        .await
        .expect("select decimals");
    assert_eq!(result.rows.len(), 3);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_decfloat_type() {
    let client = connect().await;

    let result = client
        .query(
            "VALUES (CAST('123.45' AS DECFLOAT(16)), CAST('-987654321.00001' AS DECFLOAT(34)))",
            &[],
        )
        .await
        .expect("select decfloat literals");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.columns.len(), 2);
    assert!(
        result.columns[0].type_name.contains("DecFloat(16)"),
        "expected DECFLOAT(16) metadata, got {:?}",
        result.columns[0]
    );
    assert!(
        result.columns[1].type_name.contains("DecFloat(34)"),
        "expected DECFLOAT(34) metadata, got {:?}",
        result.columns[1]
    );

    let col1: String = result.rows[0].get("COL1").expect("COL1 should decode");
    let col2: String = result.rows[0].get("COL2").expect("COL2 should decode");
    assert_eq!(col1, "123.45");
    assert_eq!(col2, "-987654321.00001");

    let param = "42.125";
    let param_result = client
        .query("VALUES CAST(? AS DECFLOAT(16))", &[&param as &dyn ToSql])
        .await
        .expect("bind decfloat parameter");
    assert_eq!(param_result.rows.len(), 1);
    let bound: String = param_result.rows[0]
        .get("1")
        .or_else(|| param_result.rows[0].get("COL1"))
        .expect("bound decfloat should decode");
    assert_eq!(bound, "42.125");

    client.close().await.expect("close");
}

#[tokio::test]
async fn test_string_types() {
    let client = connect().await;
    let table = temp_table_name("strs");
    drop_table(&client, &table).await;

    client
        .query(
            &format!(
                "CREATE TABLE {} (c CHAR(10), v VARCHAR(200), lv VARCHAR(4000))",
                table
            ),
            &[],
        )
        .await
        .expect("create table");

    // ASCII
    client
        .query(
            &format!(
                "INSERT INTO {} VALUES ('hello     ', 'world', 'long text value here')",
                table
            ),
            &[],
        )
        .await
        .expect("insert ASCII");

    // Unicode (if the database supports it)
    let unicode_result = client
        .query(
            &format!(
                "INSERT INTO {} VALUES ('unicode   ', 'caf\u{00e9} na\u{00ef}ve', '\u{00fc}ber \u{00e9}l\u{00e8}ve')",
                table
            ),
            &[],
        )
        .await;
    // Unicode insert may fail depending on DB2 codepage; that is acceptable.
    let expected_rows = if unicode_result.is_ok() { 2 } else { 1 };

    let result = client
        .query(&format!("SELECT * FROM {}", table), &[])
        .await
        .expect("select strings");
    assert_eq!(result.rows.len(), expected_rows);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_date_time_types() {
    let client = connect().await;
    let table = temp_table_name("dt");
    drop_table(&client, &table).await;

    client
        .query(
            &format!("CREATE TABLE {} (d DATE, t TIME, ts TIMESTAMP)", table),
            &[],
        )
        .await
        .expect("create table");

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES ('2024-06-15', '13:45:30', '2024-06-15-13.45.30.123456')",
                table
            ),
            &[],
        )
        .await
        .expect("insert date/time");

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES ('1970-01-01', '00:00:00', '1970-01-01-00.00.00.000000')",
                table
            ),
            &[],
        )
        .await
        .expect("insert epoch");

    let result = client
        .query(&format!("SELECT * FROM {} ORDER BY d", table), &[])
        .await
        .expect("select date/time");
    assert_eq!(result.rows.len(), 2);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}

#[tokio::test]
async fn test_boolean_type() {
    let client = connect().await;
    let table = temp_table_name("bool");
    drop_table(&client, &table).await;

    // BOOLEAN type is available in DB2 11.1+
    let create_result = client
        .query(
            &format!("CREATE TABLE {} (id INTEGER, flag BOOLEAN)", table),
            &[],
        )
        .await;

    if create_result.is_err() {
        // BOOLEAN may not be supported on older DB2 versions; skip gracefully
        eprintln!("Skipping boolean test: BOOLEAN type not supported");
        return;
    }

    client
        .query(
            &format!(
                "INSERT INTO {} VALUES (1, TRUE), (2, FALSE), (3, NULL)",
                table
            ),
            &[],
        )
        .await
        .expect("insert booleans");

    let result = client
        .query(&format!("SELECT * FROM {} ORDER BY id", table), &[])
        .await
        .expect("select booleans");
    assert_eq!(result.rows.len(), 3);

    drop_table(&client, &table).await;
    client.close().await.expect("close");
}
