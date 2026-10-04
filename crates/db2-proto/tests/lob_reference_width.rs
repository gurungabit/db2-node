use db2_proto::fdoca::{decode_rows_with_extdta, parse_qrydsc};
use db2_proto::types::Db2Value;

#[test]
fn mixed_clob_declared_reference_width_survives_every_row_split() {
    for width in [4u8, 8, 9] {
        let columns = parse_qrydsc(&[6, 0x76, 0xD0, 0xCF, 0x80, width]).unwrap();
        let mut reference = vec![0; usize::from(width)];
        if width == 9 {
            reference[0] = 2;
        }
        *reference.last_mut().unwrap() = 3;
        let mut data = vec![0xFF, 0, 0];
        data.extend_from_slice(&reference);
        for split in 0..=data.len() {
            let mut tail = Vec::new();
            let mut rows = decode_rows_with_extdta(&data[..split], &columns, &mut tail).unwrap();
            rows.extend(decode_rows_with_extdta(&data[split..], &columns, &mut tail).unwrap());
            assert!(tail.is_empty(), "width={width} split={split} tail={tail:?}");
            assert_eq!(rows.len(), 1, "width={width} split={split}");
            assert_eq!(rows[0].extdta_columns, [0], "width={width} split={split}");
            assert!(
                matches!(&rows[0].values[0], Db2Value::Clob(s) if s.starts_with("LOB locator 0x"))
            );
        }
    }
}

#[test]
fn zero_length_declared_references_and_nulls_do_not_need_payloads() {
    for width in [4u8, 8, 9] {
        let columns = parse_qrydsc(&[6, 0x76, 0xD0, 0xCF, 0x80, width]).unwrap();
        let mut data = vec![0xFF, 0, 0];
        let mut reference = vec![0; usize::from(width)];
        if width == 9 {
            reference[0] = 2;
        }
        data.extend(reference);
        data.extend([0xFF, 0, 0xFF]);
        let mut tail = Vec::new();
        let rows = decode_rows_with_extdta(&data, &columns, &mut tail).unwrap();
        assert!(tail.is_empty(), "width={width}");
        assert_eq!(rows.len(), 2, "width={width}");
        assert_eq!(rows[0].values, [Db2Value::Clob(String::new())]);
        assert_eq!(rows[1].values, [Db2Value::Null]);
        assert!(rows.iter().all(|row| row.extdta_columns.is_empty()));
    }
}

#[test]
fn sql_column_size_does_not_declare_a_wire_reference_width() {
    let mut columns = parse_qrydsc(&[9, 0x76, 0xD0, 0xC9, 4, 0, 0xCF, 0x80, 8]).unwrap();
    // SQLDARD sizes can have the high bit set too. Only QRYDSC declares the
    // reference width; a SQL column size of 32772 must retain inline decoding.
    columns[0].length = 0x8004;
    assert_eq!(columns[0].extdta_reference_length, None);
    assert_eq!(columns[1].extdta_reference_length, Some(8));
    let mut data = vec![0xFF, 0, 0, 0, 0, 0, 4, 0xAA, 0xBB, 0xCC, 0xDD, 0];
    data.extend(3u64.to_be_bytes());
    let mut tail = Vec::new();
    let rows = decode_rows_with_extdta(&data, &columns, &mut tail).unwrap();
    assert!(tail.is_empty());
    assert_eq!(
        rows[0].values[0],
        Db2Value::Blob(vec![0xAA, 0xBB, 0xCC, 0xDD])
    );
    assert_eq!(rows[0].extdta_columns, [1]);
}
