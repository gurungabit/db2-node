use super::*;
use db2_proto::ddm::DdmBuilder;
use db2_proto::dss::{DssFlags, DssHeader, DssType};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn frame(objects: &[(u16, &[u8])], chained: bool) -> DssFrame {
    let payload: Vec<u8> = objects
        .iter()
        .flat_map(|(cp, data)| DdmBuilder::new(*cp).add_raw(data).build())
        .collect();
    let mut flags = DssFlags::none();
    flags.chained = chained;
    DssFrame {
        header: DssHeader {
            length: (payload.len() + 6) as u16,
            dss_type: DssType::Reply,
            flags,
            correlation_id: 1,
        },
        payload,
    }
}

fn wire_bytes(frame: DssFrame) -> Vec<u8> {
    let mut bytes = vec![0, 0, 0xD0, 2, 0, 1];
    bytes[..2].copy_from_slice(&frame.header.length.to_be_bytes());
    if frame.header.flags.chained {
        bytes[3] |= 0x40;
    }
    bytes.extend(frame.payload);
    bytes
}

async fn socket_pair() -> (tokio::net::TcpStream, tokio::net::TcpStream) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let stream = tokio::net::TcpStream::connect(listener.local_addr().unwrap())
        .await
        .unwrap();
    let (peer, _) = listener.accept().await.unwrap();
    (stream, peer)
}

async fn read_request(peer: &mut tokio::net::TcpStream) {
    loop {
        let mut header = [0; 6];
        peer.read_exact(&mut header).await.unwrap();
        let mut payload = vec![0; usize::from(u16::from_be_bytes([header[0], header[1]])) - 6];
        peer.read_exact(&mut payload).await.unwrap();
        if header[3] & 0x40 == 0 {
            return;
        }
    }
}

fn lob_fetch(chained: bool) -> DssFrame {
    frame(
        &[
            (codepoints::QRYDTA, &[0xFF, 0, 2, 0, 0, 0, 0, 0, 0, 0, 1]),
            (codepoints::EXTDTA, b"x"),
            (codepoints::ENDQRYRM, &[]),
        ],
        chained,
    )
}

#[tokio::test]
async fn mixed_clob_declared_eight_byte_reference_materializes_query_reply() {
    let client = Client::new(Config::default());
    let mut inner = client.inner.lock().await;
    let frames = [frame(
        &[
            (codepoints::QRYDSC, &[6, 0x76, 0xD0, 0xCF, 0x80, 8]),
            (codepoints::QRYDTA, &[0xFF, 0, 0, 0, 0, 0, 0, 0, 0, 0, 3]),
            (codepoints::EXTDTA, b"\0abc"),
            (codepoints::ENDQRYRM, &[]),
        ],
        false,
    )];
    let result = inner
        .process_query_reply_public(&frames, "SELECT CL FROM T", &[], None)
        .await
        .unwrap();
    assert_eq!(result.rows.len(), 1);
    assert_eq!(
        result.rows[0].get_by_index::<String>(0).as_deref(),
        Some("abc")
    );
}

#[test]
fn declared_blob_references_preserve_zos_legacy_materialization() {
    use db2_proto::fdoca::{decode_rows_with_extdta, parse_qrydsc};
    use db2_proto::types::Db2Value;
    for width in [4u8, 8, 9] {
        let descriptors = parse_qrydsc(&[6, 0x76, 0xD0, 0xC9, 0x80, width]).unwrap();
        let mut reference = vec![0; usize::from(width)];
        if width == 9 {
            reference[0] = 2;
        }
        *reference.last_mut().unwrap() = 3;
        let mut data = vec![0xFF, 0, 0];
        data.extend(reference);
        let mut tail = Vec::new();
        let decoded = decode_rows_with_extdta(&data, &descriptors, &mut tail)
            .unwrap()
            .pop()
            .unwrap();
        assert!(tail.is_empty());
        assert_eq!(decoded.extdta_columns, [0]);
        let mut rows = [Row::from_wire(vec!["BL".into()].into(), decoded, false)];
        assert!(
            rows_need_extdta_payloads(&rows, &descriptors),
            "width={width}"
        );
        apply_extdta_payloads_to_rows(&mut rows, &descriptors, &[vec![0, 0xAB, 0xCD, 0xEF]]);
        assert_eq!(
            rows[0].values(),
            [Db2Value::Blob(vec![0xAB, 0xCD, 0xEF])],
            "width={width}"
        );
    }
}

#[tokio::test]
async fn declared_references_do_not_steal_inline_blob_payloads_across_row_splits() {
    let dsc = [9, 0x76, 0xD0, 0xC9, 4, 0, 0xCF, 0x80, 8];
    let mut data = vec![0xFF, 0, 0, 0, 0, 0, 4, 0xAA, 0xBB, 0xCC, 0xDD, 0];
    data.extend(3u64.to_be_bytes());
    data.extend([0xFF, 0, 0, 0, 0, 0, 0, 0]); // empty inline BLOB, non-null CLOB
    data.extend(0u64.to_be_bytes()); // empty external CLOB
    data.extend([0xFF, 0, 0xFF, 0xFF]); // NULLs
    data.extend([0xFF, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 4, 0]);
    data.extend(4u64.to_be_bytes());
    for split in 0..=data.len() {
        let client = Client::new(Config::default());
        let mut inner = client.inner.lock().await;
        let frames = [
            frame(
                &[
                    (codepoints::QRYDSC, &dsc),
                    (codepoints::QRYDTA, &data[..split]),
                ],
                true,
            ),
            frame(
                &[
                    (codepoints::QRYDTA, &data[split..]),
                    (codepoints::EXTDTA, b"\0abc"),
                ],
                true,
            ),
            frame(
                &[
                    (codepoints::EXTDTA, &[0, 1, 2, 3, 4]),
                    (codepoints::EXTDTA, b"\0tail"),
                    (codepoints::ENDQRYRM, &[]),
                ],
                false,
            ),
        ];
        let result = inner
            .process_query_reply_public(&frames, "SELECT BL,CL FROM T", &[], None)
            .await
            .unwrap();
        let actual: Vec<_> = result
            .rows
            .iter()
            .map(|row| row.values().to_vec())
            .collect();
        assert_eq!(
            actual,
            [
                vec![
                    db2_proto::types::Db2Value::Blob(vec![0xAA, 0xBB, 0xCC, 0xDD]),
                    db2_proto::types::Db2Value::Clob("abc".into())
                ],
                vec![
                    db2_proto::types::Db2Value::Blob(vec![]),
                    db2_proto::types::Db2Value::Clob(String::new())
                ],
                vec![
                    db2_proto::types::Db2Value::Null,
                    db2_proto::types::Db2Value::Null
                ],
                vec![
                    db2_proto::types::Db2Value::Blob(vec![1, 2, 3, 4]),
                    db2_proto::types::Db2Value::Clob("tail".into())
                ],
            ],
            "split={split}"
        );
    }
}

#[tokio::test]
async fn default_public_query_bounds_stalled_lob_chain_and_releases_mutex() {
    let (stream, mut peer) = socket_pair().await;
    let client = Client::new(Config {
        frame_drain_timeout: Duration::from_millis(1),
        ..Config::default()
    });
    {
        let mut inner = client.inner.lock().await;
        inner.transport = Some(Transport::Tcp(stream));
        inner.connected = true;
    }
    let server = tokio::spawn(async move {
        read_request(&mut peer).await; // PRPSQLSTT + SQLSTT
        peer.write_all(&wire_bytes(frame(&[], false)))
            .await
            .unwrap();
        read_request(&mut peer).await; // OPNQRY
        peer.write_all(&wire_bytes(frame(
            &[(codepoints::QRYDSC, &[6, 0x76, 0xD0, 0xCA, 4, 0])],
            false,
        )))
        .await
        .unwrap();
        read_request(&mut peer).await; // CNTQRY
        peer.write_all(&wire_bytes(lob_fetch(true))).await.unwrap();
        std::future::pending::<()>().await;
    });
    let result = timeout(
        Duration::from_secs(32),
        client.query("SELECT CL FROM T", &[]),
    )
    .await
    .expect("default query remained pending past the fetch fallback");
    assert!(
        matches!(result, Err(Error::Timeout(ref message)) if message.contains("fetch timed out after 30s")),
        "{result:?}"
    );
    let guard = timeout(Duration::from_millis(100), client.inner.lock())
        .await
        .expect("timed-out query retained the connection mutex");
    drop(guard);
    server.abort();
}

#[tokio::test]
async fn lob_chain_deadline_covers_partial_headers_and_payloads() {
    let complete = wire_bytes(frame(&[(codepoints::ENDQRYRM, &[])], false));
    for partial_len in [0, 3, 7] {
        let (stream, mut peer) = socket_pair().await;
        let client = Client::new(Config {
            query_timeout: Duration::from_millis(50),
            ..Config::default()
        });
        let mut inner = client.inner.lock().await;
        inner.transport = Some(Transport::Tcp(stream));
        let partial = complete[..partial_len].to_vec();
        let server = tokio::spawn(async move {
            read_request(&mut peer).await;
            peer.write_all(&wire_bytes(lob_fetch(true))).await.unwrap();
            peer.write_all(&partial).await.unwrap();
            std::future::pending::<()>().await;
        });
        let descriptors = db2_proto::fdoca::parse_qrydsc(&[6, 0x76, 0xD0, 0xCA, 4, 0]).unwrap();
        let mut cursor = Cursor::new(
            vec![ColumnInfo::new("CL".into(), "Clob".into(), false)],
            descriptors,
            None,
            vec![],
            1,
            false,
            false,
        );
        let result = timeout(
            Duration::from_millis(250),
            cursor.fetch_next_from(&mut inner),
        )
        .await
        .expect("continuation read has no deadline");
        server.abort();
        assert!(
            matches!(result, Err(Error::Timeout(_))),
            "partial={partial_len}: {result:?}"
        );
    }
}

#[tokio::test]
async fn lob_chain_uses_one_deadline_including_the_first_read() {
    let (stream, mut peer) = socket_pair().await;
    let client = Client::new(Config {
        query_timeout: Duration::from_millis(100),
        ..Config::default()
    });
    let mut inner = client.inner.lock().await;
    inner.transport = Some(Transport::Tcp(stream));
    let server = tokio::spawn(async move {
        read_request(&mut peer).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        peer.write_all(&wire_bytes(lob_fetch(true))).await.unwrap();
        tokio::time::sleep(Duration::from_millis(70)).await;
        peer.write_all(&wire_bytes(frame(&[], false)))
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });
    let descriptors = db2_proto::fdoca::parse_qrydsc(&[6, 0x76, 0xD0, 0xCA, 4, 0]).unwrap();
    let mut cursor = Cursor::new(
        vec![ColumnInfo::new("CL".into(), "Clob".into(), false)],
        descriptors,
        None,
        vec![],
        1,
        false,
        false,
    );
    let result = timeout(
        Duration::from_millis(250),
        cursor.fetch_next_from(&mut inner),
    )
    .await
    .expect("continuation read has no deadline");
    server.abort();
    assert!(matches!(result, Err(Error::Timeout(_))), "{result:?}");
}

#[tokio::test]
async fn luw_replies_complete_at_chain_end_without_drain_waits() {
    let (stream, mut peer) = socket_pair().await;
    // A drain wait anywhere would outlast the assertion below.
    let client = Client::new(Config {
        frame_drain_timeout: Duration::from_secs(5),
        ..Config::default()
    });
    {
        let mut inner = client.inner.lock().await;
        inner.transport = Some(Transport::Tcp(stream));
        inner.connected = true;
        inner.server_info = Some(ServerInfo {
            product_name: "QDB2/LINUXX8664".into(),
            server_release: "SQL12010".into(),
            server_class: "QDB2/LINUXX8664".into(),
            manager_levels: Vec::new(),
        });
    }
    let server = tokio::spawn(async move {
        read_request(&mut peer).await; // PRPSQLSTT + SQLSTT
        peer.write_all(&wire_bytes(frame(&[], false)))
            .await
            .unwrap();
        read_request(&mut peer).await; // OPNQRY
                                       // The reply chain spans two DSSes and two writes; only the
                                       // unchained one ends it.
        peer.write_all(&wire_bytes(frame(
            &[(codepoints::QRYDSC, &[6, 0x76, 0xD0, 0x03, 0, 4])],
            true,
        )))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        peer.write_all(&wire_bytes(frame(
            &[
                (codepoints::QRYDTA, &[0xFF, 0, 0, 7, 0, 0, 0]),
                (codepoints::ENDQRYRM, &[]),
            ],
            false,
        )))
        .await
        .unwrap();
        read_request(&mut peer).await; // EXCSQLIMM + SQLSTT + RDBCMM
        peer.write_all(&wire_bytes(frame(&[(codepoints::RDBUPDRM, &[])], true)))
            .await
            .unwrap();
        peer.write_all(&wire_bytes(frame(&[(codepoints::SQLCARD, &[0xFF])], false)))
            .await
            .unwrap();
        std::future::pending::<()>().await;
    });

    let started = Instant::now();
    let rows = timeout(Duration::from_secs(2), client.query("VALUES 7", &[]))
        .await
        .expect("query waited for a frame drain")
        .unwrap();
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0].get_by_index::<i32>(0), Some(7));
    timeout(Duration::from_secs(2), client.query("DELETE FROM T", &[]))
        .await
        .expect("execute waited for a frame drain")
        .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));
    server.abort();
}
