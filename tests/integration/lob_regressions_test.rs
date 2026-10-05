//! Live regressions for GitHub issues #19, #20 and #31 (Db2 LUW 11.5 and 12.1).
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

#[tokio::test]
async fn unicode_lobs_and_graphic_values_survive_single_row_direct_and_prepared_fetches() {
    use db2_proto::types::Db2Value::*;
    let mut config = test_config();
    config.fetch_size = 1;
    let mut client = db2_client::Client::new(config);
    client.connect().await.unwrap();
    let cases = [
        (
            "VALUES (CLOB(''), BLOB(X'')), (CLOB('clob text xxxxxxxxxxxxxxxxxxxx'),BLOB(X'01020304')), (CAST(NULL AS CLOB(1M)),CAST(NULL AS BLOB(1M))), (CLOB('LOB locator 0x12345678'),BLOB(X'ABCD1234'))",
            vec![
                vec![Clob("".into()), Blob(vec![])],
                vec![Clob("clob text xxxxxxxxxxxxxxxxxxxx".into()), Blob(vec![1, 2, 3, 4])],
                vec![Null, Null],
                vec![Clob("LOB locator 0x12345678".into()), Blob(vec![0xAB, 0xCD, 0x12, 0x34])],
            ],
        ),
        (
            "VALUES (CAST(1.25 AS REAL), CAST('é漢' AS VARGRAPHIC(8)), CLOB('clob text xxxxxxxxxxxxxxxxxxxx')), (CAST(2.25 AS REAL), CAST('abc' AS VARGRAPHIC(8)), CAST(NULL AS CLOB(1M)))",
            vec![
                vec![Real(1.25), VarChar("é漢".into()), Clob("clob text xxxxxxxxxxxxxxxxxxxx".into())],
                vec![Real(2.25), VarChar("abc".into()), Null],
            ],
        ),
        (
            "VALUES (CAST(1.25 AS DOUBLE), CAST('é漢' AS LONG VARGRAPHIC), CLOB('clob text xxxxxxxxxxxxxxxxxxxx')), (CAST(2.25 AS DOUBLE), CAST('abc' AS LONG VARGRAPHIC), CAST(NULL AS CLOB(1M)))",
            vec![
                vec![Double(1.25), VarChar("é漢".into()), Clob("clob text xxxxxxxxxxxxxxxxxxxx".into())],
                vec![Double(2.25), VarChar("abc".into()), Null],
            ],
        ),
        (
            "VALUES (CLOB('a'), DBCLOB('é漢'),BLOB(X'FF')), (CAST(NULL AS CLOB(1M)),DBCLOB(''),BLOB(X'')), (CLOB('x'),DBCLOB('abc'),BLOB(X'00010203'))",
            vec![
                vec![Clob("a".into()), Clob("é漢".into()), Blob(vec![255])],
                vec![Null, Clob("".into()), Blob(vec![])],
                vec![Clob("x".into()), Clob("abc".into()), Blob(vec![0, 1, 2, 3])],
            ],
        ),
        (
            "SELECT * FROM (VALUES (1,REPEAT(CLOB('α'),70000),REPEAT(DBCLOB('漢'),35000),BLOB(X'12345678')), (2,CAST(NULL AS CLOB(1M)),CAST(NULL AS DBCLOB(1M)),CAST(NULL AS BLOB(1M))), (3,CLOB(''),DBCLOB(''),BLOB(X'')), (4,CLOB('tail'),DBCLOB('é'),BLOB(X'ABCD1234'))) V(ID,CL,DC,BL) ORDER BY ID",
            vec![
                vec![Integer(1), Clob("α".repeat(70000)), Clob("漢".repeat(35000)), Blob(vec![0x12, 0x34, 0x56, 0x78])],
                vec![Integer(2), Null, Null, Null],
                vec![Integer(3), Clob("".into()), Clob("".into()), Blob(vec![])],
                vec![Integer(4), Clob("tail".into()), Clob("é".into()), Blob(vec![0xAB, 0xCD, 0x12, 0x34])],
            ],
        ),
    ];
    for (index, (sql, expected)) in cases.iter().enumerate() {
        let direct = client.query(sql, &[]).await.unwrap();
        let statement = client.prepare(sql).await.unwrap();
        let prepared = statement.execute(&[]).await.unwrap();
        statement.close().await.unwrap();
        for (mode, result) in [("direct", direct), ("prepared", prepared)] {
            let actual: Vec<Vec<Db2Value>> = result
                .rows
                .iter()
                .map(|row| row.values().to_vec())
                .collect();
            assert_eq!(&actual, expected, "case={index} mode={mode}");
        }
    }
    let followup = client.query("VALUES 42", &[]).await.unwrap();
    assert_eq!(followup.rows[0].get_by_index::<i32>(0), Some(42));
    client.close().await.unwrap();
}

#[tokio::test]
async fn lob_parameters_beyond_varying_descriptor_limit_are_written() {
    use db2_proto::types::Db2Value::*;
    let mut client = db2_client::Client::new(test_config());
    client.connect().await.unwrap();
    let table = unique_table(31);
    client
        .query(
            &format!(
                "CREATE TABLE {table} (ID INTEGER NOT NULL PRIMARY KEY, SMALL CLOB(32767), \
                 BIG CLOB(32768), BB BLOB(1M), DB DBCLOB(32768))"
            ),
            &[],
        )
        .await
        .unwrap();
    let check = async {
        client
            .query(
                &format!("INSERT INTO {table} (ID, SMALL, BIG) VALUES (1, 'old', 'old')"),
                &[],
            )
            .await?;
        // Exactly the declared 32768 bytes.
        let big = "β".repeat(16_384);
        let blob = (0..70_000).map(|index| index as u8).collect::<Vec<_>>();
        let graphic = "漢\u{1D11E}".repeat(5_000);
        let id = 1i32;
        let update = format!("UPDATE {table} SET SMALL = ?, BIG = ?, BB = ?, DB = ? WHERE ID = ?");
        let direct = client
            .query(&update, &[&"small", &big, &blob, &graphic, &id])
            .await?;
        assert_eq!(direct.row_count, 1);
        let select = format!("SELECT SMALL, BIG, BB, DB FROM {table} WHERE ID = 1");
        let row = client.query(&select, &[]).await?.rows[0].values().to_vec();
        assert_eq!(
            row,
            [
                Clob("small".into()),
                Clob(big),
                Blob(blob),
                Clob(graphic.clone())
            ]
        );

        let overflow = "β".repeat(16_385);
        let err = client
            .query(
                &format!("UPDATE {table} SET BIG = ? WHERE ID = ?"),
                &[&overflow, &id],
            )
            .await
            .unwrap_err();
        assert!(
            matches!(&err, db2_client::Error::Sql { sqlstate, .. } if sqlstate == "22001"),
            "{err:?}"
        );

        let statement = client.prepare(&update).await?;
        let empty: Vec<u8> = Vec::new();
        let null_text: Option<&str> = None;
        let prepared = statement
            .execute(&[&"", &"", &empty, &null_text, &id])
            .await?;
        statement.close().await?;
        assert_eq!(prepared.row_count, 1);
        let row = client.query(&select, &[]).await?.rows[0].values().to_vec();
        assert_eq!(row, [Clob("".into()), Clob("".into()), Blob(vec![]), Null]);

        // Empty LOBs before a nonempty LOB send no EXTDTA of their own.
        let statement = client.prepare(&update).await?;
        let prepared = statement
            .execute(&[&"s", &"", &empty, &graphic, &id])
            .await?;
        statement.close().await?;
        assert_eq!(prepared.row_count, 1);
        let row = client.query(&select, &[]).await?.rows[0].values().to_vec();
        assert_eq!(
            row,
            [
                Clob("s".into()),
                Clob("".into()),
                Blob(vec![]),
                Clob(graphic)
            ]
        );
        Ok::<_, db2_client::Error>(())
    }
    .await;
    client
        .query(&format!("DROP TABLE {table}"), &[])
        .await
        .unwrap();
    check.unwrap();
    client.close().await.unwrap();
}
