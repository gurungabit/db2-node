//! Live regressions for GitHub issues #19 and #20 (Db2 LUW 11.5 and 12.1).
#[path = "../common/mod.rs"]
mod common;
use common::*;
use db2_proto::types::Db2Value;

fn unique_table(issue: u32) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("LOB_I{issue}_{}_{nanos}", std::process::id())
}

#[tokio::test]
async fn clob_before_blob_preserves_external_data_order() {
    let client = connect().await;
    let mut failures = Vec::new();
    for size in ["1K", "1M"] {
        let table = unique_table(19);
        client
            .query(
                &format!(
                    "CREATE TABLE {table} (ID INTEGER NOT NULL, CL CLOB({size}), BL BLOB({size}))"
                ),
                &[],
            )
            .await
            .unwrap();
        let check = async {
            for (id, text) in [(1, "clob-one"), (2, "clob text xxxxxxxxxxxxxxxxxxxx")] {
                client
                    .query(
                        &format!("INSERT INTO {table} VALUES ({id}, '{text}', BLOB(X'0001FEFF'))"),
                        &[],
                    )
                    .await?;
            }
            client
                .query(&format!("INSERT INTO {table} (ID) VALUES (3)"), &[])
                .await?;
            for projection in ["CL, BL", "BL, CL", "ID, CL, BL", "ID, BL"] {
                let sql = format!("SELECT {projection} FROM {table} ORDER BY ID");
                let result = client.query(&sql, &[]).await?;
                if result.rows.len() != 3 {
                    failures.push(format!(
                        "{size} {projection}: expected 3 rows, got {:?}",
                        result.rows
                    ));
                    continue;
                }
                for (index, text) in ["clob-one", "clob text xxxxxxxxxxxxxxxxxxxx"]
                    .iter()
                    .enumerate()
                {
                    let row = &result.rows[index];
                    if row.get::<Vec<u8>>("BL") != Some(vec![0, 1, 254, 255]) {
                        failures.push(format!(
                            "{size} {projection} row {index}: wrong BLOB {:?}",
                            row.values()
                        ));
                    }
                    if projection.contains("CL") && row.get::<String>("CL").as_deref() != Some(text)
                    {
                        failures.push(format!(
                            "{size} {projection} row {index}: wrong CLOB {:?}",
                            row.values()
                        ));
                    }
                }
                if !result.rows[2].is_null("BL")
                    || (projection.contains("CL") && !result.rows[2].is_null("CL"))
                {
                    failures.push(format!(
                        "{size} {projection}: wrong NULL row {:?}",
                        result.rows[2].values()
                    ));
                }
            }
            Ok::<_, db2_client::Error>(())
        }
        .await;
        let cleanup = client.query(&format!("DROP TABLE {table}"), &[]).await;
        check.unwrap();
        cleanup.unwrap();
    }
    client.close().await.unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[tokio::test]
async fn large_clob_with_fixed_columns_preserves_null_rows() {
    let client = connect().await;
    let mut failures = Vec::new();
    for size in ["1K", "1M"] {
        let table = unique_table(20);
        client.query(&format!("CREATE TABLE {table} (ID INTEGER NOT NULL, D DOUBLE, G GRAPHIC(3), CL CLOB({size}))"), &[]).await.unwrap();
        let check = async {
            client.query(&format!("INSERT INTO {table} VALUES (1, 1.5, 'abc', 'clob text xxxxxxxxxxxxxxxxxxxx'), (2, 2.5, 'def', NULL), (3, 3.5, 'ghi', NULL)"), &[]).await?;
            for projection in ["G, CL", "D, CL", "ID, CL", "CL, D, G", "*"] {
                let result = client.query(&format!("SELECT {projection} FROM {table} ORDER BY ID"), &[]).await?;
                if result.rows.len() != 3 {
                    failures.push(format!("{size} {projection}: expected 3 rows, got {:?}", result.rows));
                    continue;
                }
                for (index, row) in result.rows.iter().enumerate() {
                    let expected_clob = if index == 0 { Some("clob text xxxxxxxxxxxxxxxxxxxx") } else { None };
                    if row.get::<String>("CL").as_deref() != expected_clob {
                        failures.push(format!("{size} {projection} row {index}: wrong CLOB {:?}", row.values()));
                    }
                    if row.columns().iter().any(|name| name == "D") && row.get::<f64>("D") != Some(index as f64 + 1.5) {
                        failures.push(format!("{size} {projection} row {index}: wrong DOUBLE {:?}", row.values()));
                    }
                    if row.columns().iter().any(|name| name == "G") && row.get::<String>("G").as_deref() != Some(["abc", "def", "ghi"][index]) {
                        failures.push(format!("{size} {projection} row {index}: wrong GRAPHIC {:?}", row.values()));
                    }
                    if index > 0 && !row.values().contains(&Db2Value::Null) {
                        failures.push(format!("{size} {projection} row {index}: missing NULL"));
                    }
                }
            }
            Ok::<_, db2_client::Error>(())
        }.await;
        let cleanup = client.query(&format!("DROP TABLE {table}"), &[]).await;
        check.unwrap();
        cleanup.unwrap();
    }
    client.close().await.unwrap();
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
