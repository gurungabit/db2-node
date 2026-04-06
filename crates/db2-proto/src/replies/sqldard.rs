//! Parse SQLDARD (SQL Descriptor Area Reply Data).
use crate::codepage::ebcdic037_to_utf8;
use crate::codepoints::SQLDARD;
use crate::ddm::DdmObject;
use crate::replies::sqlcard::{parse_sqlcard_data, SqlCard};
use crate::types::Db2Type;
use crate::{ProtoError, Result};

/// Metadata for a single column.
#[derive(Debug, Clone)]
pub struct ColumnMetadata {
    /// Column index (0-based).
    pub index: usize,
    /// Column name.
    pub name: String,
    /// DB2 SQL type.
    pub db2_type: Db2Type,
    /// Approximate DRDA type code for downstream decoders.
    pub drda_type: u8,
    /// Data length.
    pub length: u16,
    /// Whether the column is nullable.
    pub nullable: bool,
    /// Decimal precision (for DECIMAL/NUMERIC).
    pub precision: u8,
    /// Decimal scale (for DECIMAL/NUMERIC).
    pub scale: u8,
    /// CCSID of the column data.
    pub ccsid: u16,
    /// Base table name (if available).
    pub base_table_name: String,
    /// Schema name (if available).
    pub schema_name: String,
    /// Label (if available).
    pub label: String,
}

/// Parsed SQLDARD result.
#[derive(Debug, Clone)]
pub struct SqlDard {
    /// Embedded SQLCARD / SQLCA group.
    pub sqlcard: SqlCard,
    /// Number of columns.
    pub num_columns: u16,
    /// Column metadata.
    pub columns: Vec<ColumnMetadata>,
}

/// Parse an SQLDARD DDM object.
pub fn parse_sqldard(obj: &DdmObject) -> Result<SqlDard> {
    if obj.code_point != SQLDARD {
        return Err(ProtoError::UnexpectedReply {
            expected: SQLDARD,
            actual: obj.code_point,
        });
    }

    parse_sqldard_data(&obj.data)
}

/// Parse SQLDARD from raw data bytes.
///
/// DB2 LUW with `QTDSQLX86` returns a mixed-endian SQLDA layout:
/// control-group lengths remain network byte order, while the SQLTYPE
/// identifiers and numeric payload fields are little-endian.
pub fn parse_sqldard_data(data: &[u8]) -> Result<SqlDard> {
    if let Some(compact) = parse_compact_sqldard_data(data)? {
        return Ok(compact);
    }

    if data.is_empty() {
        return Err(ProtoError::Other("empty SQLDARD data".into()));
    }

    let (sqlcard, mut offset) = consume_sqlca_group(data)?;

    if offset >= data.len() {
        return Ok(SqlDard {
            sqlcard,
            num_columns: 0,
            columns: Vec::new(),
        });
    }

    // SQLDHGRP flag. The current LUW replies usually omit it (`0xFF`),
    // but some result sets include the optional group.
    let dh_flag = data[offset];
    offset += 1;
    if dh_flag != 0xFF {
        offset = skip_sqldhgrp(data, offset)?;
    }

    if offset + 2 > data.len() {
        return Ok(SqlDard {
            sqlcard,
            num_columns: 0,
            columns: Vec::new(),
        });
    }

    let mut declared_columns = u16::from_le_bytes([data[offset], data[offset + 1]]);
    if declared_columns > 256 && offset + 3 <= data.len() && data[offset] == 0xFF {
        offset += 1;
        declared_columns = u16::from_le_bytes([data[offset], data[offset + 1]]);
    }
    offset += 2;

    let mut columns = Vec::new();
    for index in 0..declared_columns as usize {
        if offset >= data.len() {
            break;
        }

        let end = descriptor_end(data, offset).unwrap_or(data.len());
        let descriptor = &data[offset..end];
        if descriptor.is_empty() {
            break;
        }

        match parse_column_descriptor(descriptor, index) {
            Ok(column) => columns.push(column),
            Err(_) => break,
        }

        offset = end;
    }

    Ok(SqlDard {
        sqlcard,
        num_columns: columns.len() as u16,
        columns,
    })
}

fn parse_compact_sqldard_data(data: &[u8]) -> Result<Option<SqlDard>> {
    let Some((declared_columns, mut offset)) = find_compact_descriptor_table(data) else {
        return Ok(None);
    };

    let mut columns = Vec::with_capacity(declared_columns);

    for index in 0..declared_columns {
        if offset + 16 > data.len() {
            break;
        }

        let end = next_compact_descriptor_offset(data, offset, index + 1 < declared_columns)
            .unwrap_or(data.len());
        if end > data.len() || end <= offset {
            break;
        }

        match parse_column_descriptor(&data[offset..end], index) {
            Ok(column) => columns.push(column),
            Err(_) => break,
        }

        offset = end;
    }

    if columns.is_empty() {
        return Ok(None);
    }

    Ok(Some(SqlDard {
        sqlcard: parse_sqlcard_data(data).unwrap_or_else(|_| SqlCard::success()),
        num_columns: columns.len() as u16,
        columns,
    }))
}

fn find_compact_descriptor_table(data: &[u8]) -> Option<(usize, usize)> {
    if data.len() < 18 {
        return None;
    }

    for pos in 0..=(data.len() - 18) {
        let count = u16::from_le_bytes([data[pos], data[pos + 1]]) as usize;
        if count == 0 || count > 512 {
            continue;
        }

        let start = pos + 2;
        if !looks_like_compact_descriptor_start(&data[start..]) {
            continue;
        }

        if count == 1 {
            return Some((count, start));
        }

        if let Some(next) = next_compact_descriptor_offset(data, start, true) {
            if next > start && looks_like_compact_descriptor_start(&data[next..]) {
                return Some((count, start));
            }
        }
    }

    None
}

fn consume_sqlca_group(data: &[u8]) -> Result<(SqlCard, usize)> {
    // SQLDARD embeds the SQLCARD/SQLCA group directly. The compact LUW layout is:
    //   sqlca_flag (1)
    //   sqlcode (4, LE)
    //   sqlstate (5)
    //   sqlerrproc (8)
    //   sqlcaxgrp_flag (1)
    //   if present:
    //     rowsfetched (8, LE)
    //     rowsupdated (4, LE)
    //     sqlerrd (12)
    //     sqlwarn (11)
    //     rdbname (2-byte BE len + bytes)
    //     errmsgm (2-byte BE len + bytes)
    //     errmsgs (2-byte BE len + bytes)
    let mut offset = 1 + 4 + 5 + 8;
    if data.len() < offset + 1 {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 1,
            actual: data.len(),
        });
    }

    let cax_flag = data[offset];
    offset += 1;
    if cax_flag != 0xFF {
        offset += 8 + 4 + 12 + 11;
        offset = skip_len_prefixed_string(data, offset)?;
        offset = skip_len_prefixed_string(data, offset)?;
        offset = skip_len_prefixed_string(data, offset)?;
    }

    let sqlcard = parse_sqlcard_data(&data[..offset]).unwrap_or_else(|_| SqlCard::success());
    Ok((sqlcard, offset))
}

fn skip_sqldhgrp(data: &[u8], mut offset: usize) -> Result<usize> {
    let fixed = 6 * 2;
    if offset + fixed > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + fixed,
            actual: data.len(),
        });
    }
    offset += fixed;
    offset = skip_len_prefixed_string(data, offset)?;
    offset = skip_len_prefixed_string(data, offset)?;
    offset = skip_len_prefixed_string(data, offset)?;
    Ok(offset)
}

fn skip_len_prefixed_string(data: &[u8], offset: usize) -> Result<usize> {
    if offset + 2 > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 2,
            actual: data.len(),
        });
    }
    let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
    let end = offset + 2 + len;
    if end > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: end,
            actual: data.len(),
        });
    }
    Ok(end)
}

fn descriptor_end(data: &[u8], start: usize) -> Option<usize> {
    data[start..]
        .windows(3)
        .position(|window| window == [0xFF, 0xFF, 0xFF])
        .map(|rel| start + rel + 3)
}

fn next_compact_descriptor_offset(data: &[u8], start: usize, expect_more: bool) -> Option<usize> {
    if !expect_more {
        return Some(data.len());
    }

    for pos in (start + 16)..data.len() {
        if data[pos] != 0xFF {
            continue;
        }

        let next = pos + 1;
        if next + 16 > data.len() {
            continue;
        }

        if looks_like_compact_descriptor_start(&data[next..]) {
            return Some(next);
        }
    }

    None
}

fn looks_like_compact_descriptor_start(data: &[u8]) -> bool {
    if data.len() < 16 {
        return false;
    }

    let precision = u16::from_le_bytes([data[0], data[1]]) as u8;
    let scale = u16::from_le_bytes([data[2], data[3]]) as u8;
    let raw_length = u64::from_le_bytes([
        data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11],
    ]);
    let sql_type = u16::from_le_bytes([data[12], data[13]]);
    let base_sql_type = sql_type & !1;

    if !matches!(
        base_sql_type,
        384 | 388
            | 392
            | 404
            | 408
            | 412
            | 448
            | 452
            | 456
            | 464
            | 468
            | 472
            | 480
            | 484
            | 488
            | 492
            | 496
            | 500
            | 908
            | 912
            | 988
    ) {
        return false;
    }

    if precision > 63 || scale > 63 {
        return false;
    }

    raw_length <= 0x0001_0000
}

fn parse_column_descriptor(data: &[u8], index: usize) -> Result<ColumnMetadata> {
    if data.len() < 16 {
        return Err(ProtoError::BufferTooShort {
            expected: 16,
            actual: data.len(),
        });
    }

    let precision = u16::from_le_bytes([data[0], data[1]]) as u8;
    let scale = u16::from_le_bytes([data[2], data[3]]) as u8;
    let raw_length = u64::from_le_bytes([
        data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11],
    ]);
    let sql_type = u16::from_le_bytes([data[12], data[13]]);
    let ccsid = decode_ccsid([data[14], data[15]]);

    let nullable = (sql_type & 0x0001) != 0;
    let db2_type = db2_type_from_sqlda(sql_type, raw_length, precision, scale);
    let length = normalized_length(sql_type, raw_length, precision, &db2_type);
    let drda_type = drda_type_for(&db2_type, nullable);
    let name = extract_column_name(data).unwrap_or_else(|| format!("COL{}", index + 1));

    Ok(ColumnMetadata {
        index,
        name,
        db2_type,
        drda_type,
        length,
        nullable,
        precision,
        scale,
        ccsid,
        base_table_name: String::new(),
        schema_name: String::new(),
        label: String::new(),
    })
}

fn decode_ccsid(bytes: [u8; 2]) -> u16 {
    let be = u16::from_be_bytes(bytes);
    let le = u16::from_le_bytes(bytes);

    if be == 0 || le == 0 {
        return be.max(le);
    }

    let be_small = be <= 10000;
    let le_small = le <= 10000;
    match (be_small, le_small) {
        (true, false) => be,
        (false, true) => le,
        _ => be,
    }
}

fn extract_column_name(data: &[u8]) -> Option<String> {
    for start in 16..data.len() {
        let len = data[start] as usize;
        if len == 0 || len > 64 || start + 1 + len > data.len() {
            continue;
        }

        let bytes = &data[start + 1..start + 1 + len];
        if !is_probably_name(bytes) {
            continue;
        }

        return Some(decode_text(bytes));
    }

    None
}

fn is_probably_name(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }

    bytes
        .iter()
        .all(|b| b.is_ascii_alphanumeric() || matches!(*b, b'_' | b'$' | b'#' | b'@'))
}

fn decode_text(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        ebcdic037_to_utf8(bytes).trim().to_string()
    }
}

fn db2_type_from_sqlda(sql_type: u16, raw_length: u64, precision: u8, scale: u8) -> Db2Type {
    match sql_type & !1 {
        384 => Db2Type::Date,
        388 => Db2Type::Time,
        392 => Db2Type::Timestamp,
        404 => Db2Type::Blob,
        408 => Db2Type::Clob,
        412 => Db2Type::DbClob,
        448 | 456 => Db2Type::VarChar(raw_length.min(u16::MAX as u64) as u16),
        452 => Db2Type::Char(raw_length.min(u16::MAX as u64) as u16),
        464 | 472 => Db2Type::VarGraphic(raw_length.min(u16::MAX as u64) as u16),
        468 => Db2Type::Graphic(raw_length.min(u16::MAX as u64) as u16),
        480 => {
            if raw_length >= 8 {
                Db2Type::Double
            } else {
                Db2Type::Real
            }
        }
        484 | 488 => Db2Type::Decimal { precision, scale },
        996 => Db2Type::DecFloat(if raw_length > 8 { 34 } else { 16 }),
        492 => Db2Type::BigInt,
        496 => Db2Type::Integer,
        500 => Db2Type::SmallInt,
        908 => Db2Type::VarBinary(raw_length.min(u16::MAX as u64) as u16),
        912 => Db2Type::Binary(raw_length.min(u16::MAX as u64) as u16),
        988 => Db2Type::Xml,
        _ => Db2Type::VarChar(raw_length.min(u16::MAX as u64) as u16),
    }
}

fn normalized_length(sql_type: u16, raw_length: u64, precision: u8, db2_type: &Db2Type) -> u16 {
    match sql_type & !1 {
        384 => 10,
        388 => 8,
        392 => 26,
        404 | 408 | 412 | 448 | 452 | 456 | 464 | 468 | 472 | 908 | 912 | 988 => {
            raw_length.min(u16::MAX as u64) as u16
        }
        480 => match db2_type {
            Db2Type::Double => 8,
            _ => 4,
        },
        484 | 488 => ((precision as usize + 2) / 2) as u16,
        996 => raw_length.min(u16::MAX as u64) as u16,
        492 => 8,
        496 => 4,
        500 => 2,
        _ => raw_length.min(u16::MAX as u64) as u16,
    }
}

fn drda_type_for(db2_type: &Db2Type, nullable: bool) -> u8 {
    use crate::types::*;

    let base = match db2_type {
        Db2Type::SmallInt => DRDA_TYPE_SMALLINT,
        Db2Type::Integer => DRDA_TYPE_INTEGER,
        Db2Type::BigInt => DRDA_TYPE_BIGINT,
        Db2Type::Real => DRDA_TYPE_FLOAT4,
        Db2Type::Double => DRDA_TYPE_FLOAT8,
        Db2Type::Decimal { .. } => DRDA_TYPE_DECIMAL,
        Db2Type::DecFloat(_) => DRDA_TYPE_DECFLOAT,
        Db2Type::Char(_) => DRDA_TYPE_CHAR,
        Db2Type::VarChar(_) | Db2Type::LongVarChar => DRDA_TYPE_VARCHAR,
        Db2Type::Clob => DRDA_TYPE_CLOB,
        Db2Type::Binary(_) => DRDA_TYPE_BINARY,
        Db2Type::VarBinary(_) => DRDA_TYPE_VARBINARY,
        Db2Type::Blob => DRDA_TYPE_BLOB,
        Db2Type::Date => DRDA_TYPE_DATE,
        Db2Type::Time => DRDA_TYPE_TIME,
        Db2Type::Timestamp => DRDA_TYPE_TIMESTAMP,
        Db2Type::Graphic(_) => DRDA_TYPE_GRAPHIC,
        Db2Type::VarGraphic(_) => DRDA_TYPE_VARGRAPH,
        Db2Type::DbClob => DRDA_TYPE_DBCLOB,
        Db2Type::Boolean => DRDA_TYPE_BOOLEAN,
        Db2Type::Xml => DRDA_TYPE_XML,
        Db2Type::Null => DRDA_TYPE_NVARCHAR,
    };

    if nullable {
        base | 0x01
    } else {
        base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_minimal_sqldard() {
        let mut data = Vec::new();
        // SQLCA group: flag(1) + sqlcode(4) + sqlstate(5) + sqlerrproc(8) + sqlcaxgrp_flag(1)
        data.push(0x00); // SQLCA flag (not null)
        data.extend_from_slice(&0i32.to_le_bytes()); // sqlcode = 0
        data.extend_from_slice(b"00000"); // sqlstate
        data.extend_from_slice(b"SQLPROC1"); // sqlerrproc
        data.push(0xFF); // SQLCAXGRP absent
                         // SQLDHGRP flag
        data.push(0xFF); // SQLDHGRP absent
                         // num_columns (LE u16)
        data.extend_from_slice(&0u16.to_le_bytes());

        let mut builder = crate::ddm::DdmBuilder::new(SQLDARD);
        builder.add_raw(&data);
        let bytes = builder.build();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();

        let dard = parse_sqldard(&obj).unwrap();
        assert_eq!(dard.num_columns, 0);
        assert!(dard.columns.is_empty());
    }

    #[test]
    fn test_extract_column_name() {
        let descriptor = [
            0, 0, 0, 0, 50, 0, 0, 0, 0, 0, 0, 0, 0xC1, 0x01, 0x04, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 8,
            0, 0, 0, 0, 0, 4, b'N', b'A', b'M', b'E', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF,
            0xFF,
        ];
        assert_eq!(extract_column_name(&descriptor), Some("NAME".to_string()));
    }
}
