//! Parse SQLDARD (SQL Descriptor Area Reply Data).
use crate::codepage::ebcdic037_to_utf8;
use crate::codepoints::SQLDARD;
use crate::ddm::DdmObject;
use crate::fdoca::ByteOrder;
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
    /// Byte order used for fixed-width numeric row values.
    pub byte_order: ByteOrder,
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
    let mut fallback = None;

    if let Ok(standard) = parse_standard_sqldard_data(data, ByteOrder::BigEndian) {
        if !standard.columns.is_empty() {
            if sqldard_has_real_names(&standard) {
                return Ok(standard);
            }
            fallback = Some(standard);
        }
    }

    if let Some(compact) = parse_compact_sqldard_data(data)? {
        if sqldard_has_real_names(&compact) {
            return Ok(compact);
        }
        if fallback.is_none() {
            fallback = Some(compact);
        }
    }

    if let Ok(standard) = parse_standard_sqldard_data(data, ByteOrder::LittleEndian) {
        if !standard.columns.is_empty() {
            if sqldard_has_real_names(&standard) {
                return Ok(standard);
            }
            if fallback.is_none() {
                fallback = Some(standard);
            }
        }
    }

    if let Some(scanned) = scan_standard_sqldagroups(data) {
        if sqldard_has_real_names(&scanned) {
            return Ok(scanned);
        }
        if fallback.is_none() {
            fallback = Some(scanned);
        }
    }

    if let Some(fallback) = fallback {
        return Ok(fallback);
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

fn parse_standard_sqldard_data(data: &[u8], byte_order: ByteOrder) -> Result<SqlDard> {
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

    let declared_columns = read_u16(byte_order, [data[offset], data[offset + 1]]) as usize;
    if declared_columns > 512 {
        return Err(ProtoError::Other(format!(
            "implausible SQLDARD column count: {}",
            declared_columns
        )));
    }
    offset += 2;

    let mut columns = Vec::with_capacity(declared_columns);
    for index in 0..declared_columns {
        let (column, next_offset) =
            parse_standard_column_descriptor(data, offset, index, byte_order)?;
        columns.push(column);
        offset = next_offset;
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

fn scan_standard_sqldagroups(data: &[u8]) -> Option<SqlDard> {
    let mut best_columns = Vec::new();
    let mut best_score = 0usize;

    for byte_order in [ByteOrder::BigEndian, ByteOrder::LittleEndian] {
        for start in 0..data.len().saturating_sub(17) {
            let mut offset = start;
            let mut columns = Vec::new();

            for index in 0..512 {
                let Ok((column, next_offset)) =
                    parse_standard_column_descriptor(data, offset, index, byte_order)
                else {
                    break;
                };
                if next_offset <= offset || next_offset > data.len() {
                    break;
                }

                columns.push(column);
                offset = next_offset;
            }

            let named_count = columns
                .iter()
                .filter(|column| !is_generated_column_name(&column.name))
                .count();
            if named_count == 0 || columns.len() < 2 {
                continue;
            }

            let generated_count = columns.len().saturating_sub(named_count);
            let score = named_count * 2048 + columns.len() - generated_count * 1024;
            if score > best_score {
                best_score = score;
                best_columns = columns;
            }
        }
    }

    if best_columns.is_empty() {
        return None;
    }

    Some(SqlDard {
        sqlcard: parse_sqlcard_data(data).unwrap_or_else(|_| SqlCard::success()),
        num_columns: best_columns.len() as u16,
        columns: best_columns,
    })
}

/// Scan SQLDARD bytes for column names in SQLDOPTGRP/SQLDXGRP fields without
/// depending on the SQLTYPE values. Some z/OS replies use descriptor type values
/// that are not needed for names and can prevent the stricter descriptor parser
/// from reaching SQLNAME/SQLXNAME.
pub fn scan_column_names(data: &[u8]) -> Vec<String> {
    let qualified_names = scan_repeated_qualified_column_names(data);
    let names_before_rdb = scan_names_immediately_before_rdb(data);
    if names_before_rdb.len() > qualified_names.len() {
        return names_before_rdb;
    }
    if !qualified_names.is_empty() {
        return qualified_names;
    }

    let mut best_names = Vec::new();

    for start in 0..data.len().saturating_sub(20) {
        let mut offset = start;
        let mut names = Vec::new();

        for _ in 0..512 {
            let Some((name, next_offset)) = scan_descriptor_name(data, offset) else {
                break;
            };
            if next_offset <= offset || next_offset > data.len() {
                break;
            }
            names.push(name);
            offset = next_offset;
        }

        if names.len() > best_names.len() {
            best_names = names;
        }
    }

    best_names
}

fn scan_names_immediately_before_rdb(data: &[u8]) -> Vec<String> {
    let candidates = scan_len_prefixed_identifier_values(data, LenPrefix::BigEndianU16);
    let Some(rdb_name) = candidates.first().map(|candidate| candidate.text.as_str()) else {
        return Vec::new();
    };
    if !is_probably_identifier_text(rdb_name) {
        return Vec::new();
    }

    let mut names = Vec::new();
    for pair in candidates.windows(2) {
        let column = &pair[0].text;
        if pair[1].text == rdb_name && column != rdb_name && is_probably_identifier_text(column) {
            names.push(column.clone());
        }
    }

    names.dedup();
    if names.len() >= 2 {
        names
    } else {
        Vec::new()
    }
}

fn scan_repeated_qualified_column_names(data: &[u8]) -> Vec<String> {
    let candidates = scan_len_prefixed_identifier_values(data, LenPrefix::BigEndianU16);
    let mut names = Vec::new();
    let mut index = 0usize;

    while index + 4 < candidates.len() {
        let column = &candidates[index].text;
        if is_probably_identifier_text(column)
            && candidates[index + 4].text == *column
            && candidates[index + 1].text != *column
            && candidates[index + 2].text != *column
            && candidates[index + 3].text != *column
        {
            names.push(column.clone());
            index += 5;
        } else {
            index += 1;
        }
    }

    if names.len() >= 2 {
        names
    } else {
        Vec::new()
    }
}

pub fn diagnose_column_names(data: &[u8]) -> Vec<String> {
    let mut diagnostics = Vec::new();

    match parse_sqldard_data(data) {
        Ok(dard) => {
            let names = dard
                .columns
                .iter()
                .take(48)
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>()
                .join(",");
            diagnostics.push(format!(
                "sqldard_name_diag parsed_columns={} parsed_real_names={} parsed_names=[{}]",
                dard.columns.len(),
                dard.columns
                    .iter()
                    .filter(|column| !is_generated_column_name(&column.name))
                    .count(),
                names
            ));
        }
        Err(err) => diagnostics.push(format!("sqldard_name_diag parse_error={}", err)),
    }

    let scanned_names = scan_column_names(data);
    diagnostics.push(format!(
        "sqldard_name_diag structured_scan_count={} structured_scan_names=[{}]",
        scanned_names.len(),
        scanned_names
            .iter()
            .take(64)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(",")
    ));

    let be_candidates = scan_len_prefixed_identifier_candidates(data, LenPrefix::BigEndianU16);
    diagnostics.push(format!(
        "sqldard_name_diag be_len_candidates count={} first=[{}]",
        be_candidates.len(),
        be_candidates
            .iter()
            .take(80)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("; ")
    ));

    let one_byte_candidates = scan_len_prefixed_identifier_candidates(data, LenPrefix::OneByte);
    diagnostics.push(format!(
        "sqldard_name_diag one_byte_candidates count={} first=[{}]",
        one_byte_candidates.len(),
        one_byte_candidates
            .iter()
            .take(80)
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("; ")
    ));

    diagnostics
}

#[derive(Clone, Copy)]
enum LenPrefix {
    BigEndianU16,
    OneByte,
}

fn scan_len_prefixed_identifier_candidates(data: &[u8], prefix: LenPrefix) -> Vec<String> {
    scan_len_prefixed_identifier_values(data, prefix)
        .into_iter()
        .map(|candidate| {
            format!(
                "@{} len={} text={} ctx={}",
                candidate.offset,
                candidate.len,
                candidate.text,
                hex_context(
                    data,
                    candidate.offset,
                    candidate.offset + candidate.prefix_len + candidate.len
                )
            )
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IdentifierCandidate {
    offset: usize,
    prefix_len: usize,
    len: usize,
    text: String,
}

fn scan_len_prefixed_identifier_values(data: &[u8], prefix: LenPrefix) -> Vec<IdentifierCandidate> {
    let mut candidates = Vec::new();
    let mut last_text = String::new();
    let mut last_offset = usize::MAX;

    for offset in 0..data.len() {
        let Some((text_start, len)) = read_candidate_len(data, offset, prefix) else {
            continue;
        };
        let end = text_start + len;
        if end > data.len() || !(2..=128).contains(&len) {
            continue;
        }

        let bytes = &data[text_start..end];
        if !is_probably_name(bytes) {
            continue;
        }

        let text = decode_text(bytes);
        if !is_probably_identifier_text(&text) {
            continue;
        }

        if text == last_text && offset.saturating_sub(last_offset) < 8 {
            continue;
        }

        candidates.push(IdentifierCandidate {
            offset,
            prefix_len: text_start - offset,
            len,
            text: text.clone(),
        });
        last_text = text;
        last_offset = offset;
    }

    candidates
}

fn read_candidate_len(data: &[u8], offset: usize, prefix: LenPrefix) -> Option<(usize, usize)> {
    match prefix {
        LenPrefix::BigEndianU16 => {
            if offset + 2 > data.len() {
                return None;
            }
            Some((
                offset + 2,
                u16::from_be_bytes([data[offset], data[offset + 1]]) as usize,
            ))
        }
        LenPrefix::OneByte => {
            if offset >= data.len() {
                return None;
            }
            Some((offset + 1, data[offset] as usize))
        }
    }
}

fn hex_context(data: &[u8], start: usize, end: usize) -> String {
    let context_start = start.saturating_sub(8);
    let context_end = (end + 8).min(data.len());
    data[context_start..context_end]
        .iter()
        .map(|byte| format!("{:02X}", byte))
        .collect::<Vec<_>>()
        .join(" ")
}

fn scan_descriptor_name(data: &[u8], descriptor_offset: usize) -> Option<(String, usize)> {
    let mut offset = descriptor_offset.checked_add(16)?;
    if offset >= data.len() {
        return None;
    }

    let opt_flag = data[offset];
    offset += 1;
    if opt_flag == 0xFF {
        return None;
    }

    offset = skip_short(data, offset).ok()?;

    let (name_m, next) = read_len_prefixed_string(data, offset).ok()?;
    offset = next;
    let (name_s, next) = read_len_prefixed_string(data, offset).ok()?;
    offset = next;

    let direct_name = if is_probably_identifier_text(&name_m) {
        Some(name_m)
    } else if is_probably_identifier_text(&name_s) {
        Some(name_s)
    } else {
        None
    };

    let (_label_m, next) = read_len_prefixed_string(data, offset).ok()?;
    offset = next;
    let (_label_s, next) = read_len_prefixed_string(data, offset).ok()?;
    offset = next;
    let (_comment_m, next) = read_len_prefixed_string(data, offset).ok()?;
    offset = next;
    let (_comment_s, next) = read_len_prefixed_string(data, offset).ok()?;
    offset = next;

    offset = skip_sqludtgrp(data, offset).ok()?;
    let (_base_name, _schema_name, column_name, next) = skip_sqldxgrp(data, offset).ok()?;
    offset = next;

    if let Some(name) = direct_name {
        return Some((name, offset));
    }
    if is_probably_identifier_text(&column_name) {
        return Some((column_name, offset));
    }

    None
}

fn is_generated_column_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("COL") else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
}

fn sqldard_has_real_names(dard: &SqlDard) -> bool {
    dard.columns
        .iter()
        .any(|column| !is_generated_column_name(&column.name))
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
            | 904
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
    let db2_type = db2_type_from_sqlda(sql_type, raw_length, precision, scale, ccsid);
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
        byte_order: ByteOrder::LittleEndian,
        base_table_name: String::new(),
        schema_name: String::new(),
        label: String::new(),
    })
}

fn parse_standard_column_descriptor(
    data: &[u8],
    offset: usize,
    index: usize,
    byte_order: ByteOrder,
) -> Result<(ColumnMetadata, usize)> {
    if offset + 16 > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 16,
            actual: data.len(),
        });
    }

    let precision = read_u16(byte_order, [data[offset], data[offset + 1]]) as u8;
    let scale = read_u16(byte_order, [data[offset + 2], data[offset + 3]]) as u8;
    let raw_length = read_u64(
        byte_order,
        [
            data[offset + 4],
            data[offset + 5],
            data[offset + 6],
            data[offset + 7],
            data[offset + 8],
            data[offset + 9],
            data[offset + 10],
            data[offset + 11],
        ],
    );
    let sql_type = read_u16(byte_order, [data[offset + 12], data[offset + 13]]);
    let ccsid = read_u16(byte_order, [data[offset + 14], data[offset + 15]]);

    if !is_known_sql_type(sql_type) {
        return Err(ProtoError::Other(format!(
            "unknown SQLDARD SQLTYPE: {}",
            sql_type
        )));
    }

    let nullable = (sql_type & 0x0001) != 0;
    let db2_type = db2_type_from_sqlda(sql_type, raw_length, precision, scale, ccsid);
    let length = normalized_length(sql_type, raw_length, precision, &db2_type);
    let drda_type = drda_type_for(&db2_type, nullable);

    let mut next_offset = offset + 16;
    let mut name = None;
    let mut label = String::new();
    let mut base_table_name = String::new();
    let mut schema_name = String::new();

    if next_offset < data.len() {
        let opt_flag = data[next_offset];
        next_offset += 1;
        if opt_flag != 0xFF {
            next_offset = skip_short(data, next_offset)?;

            let (name_m, next) = read_len_prefixed_string(data, next_offset)?;
            next_offset = next;
            let (name_s, next) = read_len_prefixed_string(data, next_offset)?;
            next_offset = next;
            if !name_m.is_empty() {
                name = Some(name_m);
            } else if !name_s.is_empty() {
                name = Some(name_s);
            }

            let (label_m, next) = read_len_prefixed_string(data, next_offset)?;
            next_offset = next;
            let (label_s, next) = read_len_prefixed_string(data, next_offset)?;
            next_offset = next;
            label = if !label_m.is_empty() {
                label_m
            } else {
                label_s
            };

            let (_comment_m, next) = read_len_prefixed_string(data, next_offset)?;
            next_offset = next;
            let (_comment_s, next) = read_len_prefixed_string(data, next_offset)?;
            next_offset = next;

            next_offset = skip_sqludtgrp(data, next_offset)?;
            let (base_name, schema_name_candidate, column_name_candidate, next) =
                skip_sqldxgrp(data, next_offset)?;
            next_offset = next;
            base_table_name = base_name;
            schema_name = schema_name_candidate;
            if name.is_none() && !column_name_candidate.is_empty() {
                name = Some(column_name_candidate);
            }
        }
    }

    Ok((
        ColumnMetadata {
            index,
            name: name.unwrap_or_else(|| format!("COL{}", index + 1)),
            db2_type,
            drda_type,
            length,
            nullable,
            precision,
            scale,
            ccsid,
            byte_order,
            base_table_name,
            schema_name,
            label,
        },
        next_offset,
    ))
}

fn skip_short(data: &[u8], offset: usize) -> Result<usize> {
    if offset + 2 > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 2,
            actual: data.len(),
        });
    }
    Ok(offset + 2)
}

fn read_len_prefixed_string(data: &[u8], offset: usize) -> Result<(String, usize)> {
    if offset + 2 > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 2,
            actual: data.len(),
        });
    }

    let len = u16::from_be_bytes([data[offset], data[offset + 1]]) as usize;
    let start = offset + 2;
    let end = start + len;
    if end > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: end,
            actual: data.len(),
        });
    }

    Ok((decode_text(&data[start..end]), end))
}

fn skip_sqludtgrp(data: &[u8], mut offset: usize) -> Result<usize> {
    if offset >= data.len() {
        return Ok(offset);
    }

    let flag = data[offset];
    offset += 1;
    if flag == 0xFF {
        return Ok(offset);
    }

    if offset + 4 > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 4,
            actual: data.len(),
        });
    }
    offset += 4;

    for _ in 0..5 {
        offset = skip_len_prefixed_string(data, offset)?;
    }

    Ok(offset)
}

fn skip_sqldxgrp(data: &[u8], mut offset: usize) -> Result<(String, String, String, usize)> {
    if offset >= data.len() {
        return Ok((String::new(), String::new(), String::new(), offset));
    }

    let flag = data[offset];
    offset += 1;
    if flag == 0xFF {
        return Ok((String::new(), String::new(), String::new(), offset));
    }

    if offset + 8 > data.len() {
        return Err(ProtoError::BufferTooShort {
            expected: offset + 8,
            actual: data.len(),
        });
    }
    offset += 8;

    let (_rdbnam, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (_corname_m, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (_corname_s, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (basename_m, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (basename_s, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (schema_m, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (schema_s, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (xname_m, next) = read_len_prefixed_string(data, offset)?;
    offset = next;
    let (xname_s, next) = read_len_prefixed_string(data, offset)?;
    offset = next;

    let base_name = if !basename_m.is_empty() {
        basename_m
    } else {
        basename_s
    };
    let schema_name = if !schema_m.is_empty() {
        schema_m
    } else {
        schema_s
    };
    let column_name = if !xname_m.is_empty() {
        xname_m
    } else {
        xname_s
    };

    Ok((base_name, schema_name, column_name, offset))
}

fn read_u16(byte_order: ByteOrder, bytes: [u8; 2]) -> u16 {
    match byte_order {
        ByteOrder::BigEndian => u16::from_be_bytes(bytes),
        ByteOrder::LittleEndian => u16::from_le_bytes(bytes),
    }
}

fn read_u64(byte_order: ByteOrder, bytes: [u8; 8]) -> u64 {
    match byte_order {
        ByteOrder::BigEndian => u64::from_be_bytes(bytes),
        ByteOrder::LittleEndian => u64::from_le_bytes(bytes),
    }
}

fn is_known_sql_type(sql_type: u16) -> bool {
    matches!(
        sql_type & !1,
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
            | 904
            | 908
            | 912
            | 988
            | 996
    )
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

fn is_probably_identifier_text(value: &str) -> bool {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 128 || is_generated_column_name(trimmed) {
        return false;
    }

    trimmed
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'#' | b'@'))
}

fn decode_text(bytes: &[u8]) -> String {
    if bytes.is_ascii() {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        ebcdic037_to_utf8(bytes).trim().to_string()
    }
}

fn db2_type_from_sqlda(
    sql_type: u16,
    raw_length: u64,
    precision: u8,
    scale: u8,
    ccsid: u16,
) -> Db2Type {
    match sql_type & !1 {
        384 => Db2Type::Date,
        388 => Db2Type::Time,
        392 => Db2Type::Timestamp,
        404 => Db2Type::Blob,
        408 => Db2Type::Clob,
        412 => Db2Type::DbClob,
        448 | 456 if ccsid == 0 => Db2Type::VarBinary(raw_length.min(u16::MAX as u64) as u16),
        448 | 456 => Db2Type::VarChar(raw_length.min(u16::MAX as u64) as u16),
        452 if ccsid == 0 => Db2Type::Binary(raw_length.min(u16::MAX as u64) as u16),
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
        904 => Db2Type::RowId(raw_length.min(u16::MAX as u64) as u16),
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
        904 => raw_length.min(u16::MAX as u64) as u16,
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
        Db2Type::BlobLocator => DRDA_TYPE_LOBLOC,
        Db2Type::Date => DRDA_TYPE_DATE,
        Db2Type::Time => DRDA_TYPE_TIME,
        Db2Type::Timestamp => DRDA_TYPE_TIMESTAMP,
        Db2Type::Graphic(_) => DRDA_TYPE_GRAPHIC,
        Db2Type::VarGraphic(_) => DRDA_TYPE_VARGRAPH,
        Db2Type::DbClob => DRDA_TYPE_DBCLOB,
        Db2Type::ClobLocator => DRDA_TYPE_CLOBLOC,
        Db2Type::DbClobLocator => DRDA_TYPE_DBCLOBLOC,
        Db2Type::LobBytes(_) => FDOCA_TYPE_LOBBYTES,
        Db2Type::LobChar(_) => FDOCA_TYPE_LOBCHAR,
        Db2Type::RowId(_) => DRDA_TYPE_ROWID,
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
    fn test_parse_standard_big_endian_sqldard() {
        let mut data = Vec::new();
        data.push(0x00);
        data.extend_from_slice(&0i32.to_be_bytes());
        data.extend_from_slice(b"00000");
        data.extend_from_slice(b"SQLPROC1");
        data.push(0xFF);
        data.push(0xFF);
        data.extend_from_slice(&2u16.to_be_bytes());

        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&4u64.to_be_bytes());
        data.extend_from_slice(&496u16.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.push(0xFF);

        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&0u16.to_be_bytes());
        data.extend_from_slice(&26u64.to_be_bytes());
        data.extend_from_slice(&392u16.to_be_bytes());
        data.extend_from_slice(&1208u16.to_be_bytes());
        data.push(0xFF);

        let mut builder = crate::ddm::DdmBuilder::new(SQLDARD);
        builder.add_raw(&data);
        let bytes = builder.build();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();

        let dard = parse_sqldard(&obj).unwrap();
        assert_eq!(dard.num_columns, 2);
        assert_eq!(dard.columns[0].db2_type, Db2Type::Integer);
        assert_eq!(dard.columns[0].length, 4);
        assert_eq!(dard.columns[0].byte_order, ByteOrder::BigEndian);
        assert_eq!(dard.columns[1].db2_type, Db2Type::Timestamp);
        assert_eq!(dard.columns[1].length, 26);
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

    #[test]
    fn test_scan_standard_sqldagroups_finds_padded_descriptors() {
        let mut data = vec![0; 96];
        data.extend_from_slice(&standard_descriptor("POLICY_ID", 0, 0, 6, 484, 0));
        data.extend_from_slice(&standard_descriptor("POLICY_NUM", 0, 0, 20, 464, 1200));

        let dard = parse_sqldard_data(&data).unwrap();
        assert_eq!(dard.num_columns, 2);
        assert_eq!(dard.columns[0].name, "POLICY_ID");
        assert_eq!(dard.columns[1].name, "POLICY_NUM");
    }

    #[test]
    fn test_parse_sqldard_prefers_scanned_names_over_generated_names() {
        let mut data = Vec::new();
        data.push(0x00);
        data.extend_from_slice(&0i32.to_be_bytes());
        data.extend_from_slice(b"00000");
        data.extend_from_slice(b"SQLPROC1");
        data.push(0xFF);
        data.push(0xFF);
        data.extend_from_slice(&1u16.to_be_bytes());
        data.extend_from_slice(&unnamed_standard_descriptor(4, 496, 0));

        data.extend_from_slice(&standard_descriptor("POLICY_ID", 0, 0, 6, 484, 0));
        data.extend_from_slice(&standard_descriptor("POLICY_NUM", 0, 0, 20, 464, 1200));

        let dard = parse_sqldard_data(&data).unwrap();
        assert_eq!(dard.num_columns, 2);
        assert_eq!(dard.columns[0].name, "POLICY_ID");
        assert_eq!(dard.columns[1].name, "POLICY_NUM");
    }

    #[test]
    fn test_parse_standard_column_descriptor_uses_sqlxname() {
        let descriptor = standard_descriptor_with_sqlxname("BASE_ID", 0, 0, 4, 496, 0);
        let (column, next) =
            parse_standard_column_descriptor(&descriptor, 0, 0, ByteOrder::BigEndian).unwrap();
        assert_eq!(next, descriptor.len());
        assert_eq!(column.name, "BASE_ID");
    }

    #[test]
    fn test_parse_standard_rowid_descriptor() {
        let descriptor = standard_descriptor("DB2_GENERATED_ROWID_FOR_LOBS", 0, 0, 40, 904, 1208);
        let (column, next) =
            parse_standard_column_descriptor(&descriptor, 0, 0, ByteOrder::BigEndian).unwrap();
        assert_eq!(next, descriptor.len());
        assert_eq!(column.name, "DB2_GENERATED_ROWID_FOR_LOBS");
        assert_eq!(column.db2_type, Db2Type::RowId(40));
        assert_eq!(column.drda_type, crate::types::DRDA_TYPE_ROWID);
    }

    #[test]
    fn test_scan_column_names_ignores_sqltype_values() {
        let mut data = vec![0; 32];
        data.extend_from_slice(&standard_descriptor_with_type(
            "POLICY_ID",
            0,
            0,
            6,
            0xF101,
            0,
        ));
        data.extend_from_slice(&standard_descriptor_with_type(
            "POLICY_NUM",
            0,
            0,
            20,
            0xF101,
            1200,
        ));

        let names = scan_column_names(&data);
        assert_eq!(names, vec!["POLICY_ID", "POLICY_NUM"]);
    }

    #[test]
    fn test_scan_column_names_extracts_repeated_qualified_pattern() {
        let mut data = Vec::new();
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "PROP_ID");
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "PLCY_SNPST");
        push_identifier(&mut data, "FIREINSP");
        push_identifier(&mut data, "PROP_ID");
        data.extend_from_slice(&[0xFF, 0x00, 0x00, 0x00]);
        push_identifier(&mut data, "AGRE_ACCES_KEY");
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "PLCY_SNPST");
        push_identifier(&mut data, "FIREINSP");
        push_identifier(&mut data, "AGRE_ACCES_KEY");

        let names = scan_column_names(&data);
        assert_eq!(names, vec!["PROP_ID", "AGRE_ACCES_KEY"]);
    }

    #[test]
    fn test_scan_column_names_extracts_lob_descriptors_before_rdb_name() {
        let mut data = Vec::new();
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "INSP_RPT_ID");
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "INSP_RPT");
        push_identifier(&mut data, "FIREINSP");
        push_identifier(&mut data, "INSP_RPT_ID");
        push_identifier(&mut data, "INSP_RPT_DETL_DOC");
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "UNDWR_ACTN_DETL_DOC");
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "DB2_GENERATED_ROWID_FOR_LOBS");
        push_identifier(&mut data, "DDFIC0AG");
        push_identifier(&mut data, "INSP_RPT");
        push_identifier(&mut data, "FIREINSP");
        push_identifier(&mut data, "DB2_GENERATED_ROWID_FOR_LOBS");

        let names = scan_column_names(&data);
        assert_eq!(
            names,
            vec![
                "INSP_RPT_ID",
                "INSP_RPT_DETL_DOC",
                "UNDWR_ACTN_DETL_DOC",
                "DB2_GENERATED_ROWID_FOR_LOBS"
            ]
        );
    }

    #[test]
    fn test_diagnose_column_names_reports_len_prefixed_candidates() {
        let mut data = vec![0; 16];
        data.extend_from_slice(&8u16.to_be_bytes());
        data.extend_from_slice(b"POLICYID");

        let diagnostics = diagnose_column_names(&data);
        assert!(diagnostics
            .iter()
            .any(|line| line.contains("text=POLICYID")));
    }

    fn push_identifier(data: &mut Vec<u8>, value: &str) {
        data.extend_from_slice(&(value.len() as u16).to_be_bytes());
        data.extend_from_slice(value.as_bytes());
    }

    fn unnamed_standard_descriptor(length: u64, sql_type: u16, ccsid: u16) -> Vec<u8> {
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor.extend_from_slice(&length.to_be_bytes());
        descriptor.extend_from_slice(&sql_type.to_be_bytes());
        descriptor.extend_from_slice(&ccsid.to_be_bytes());
        descriptor.push(0xFF);
        descriptor
    }

    fn standard_descriptor(
        name: &str,
        precision: u16,
        scale: u16,
        length: u64,
        sql_type: u16,
        ccsid: u16,
    ) -> Vec<u8> {
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&precision.to_be_bytes());
        descriptor.extend_from_slice(&scale.to_be_bytes());
        descriptor.extend_from_slice(&length.to_be_bytes());
        descriptor.extend_from_slice(&sql_type.to_be_bytes());
        descriptor.extend_from_slice(&ccsid.to_be_bytes());
        descriptor.push(0x00);
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor.extend_from_slice(&(name.len() as u16).to_be_bytes());
        descriptor.extend_from_slice(name.as_bytes());
        for _ in 0..5 {
            descriptor.extend_from_slice(&0u16.to_be_bytes());
        }
        descriptor.push(0xFF);
        descriptor.push(0xFF);
        descriptor
    }

    fn standard_descriptor_with_type(
        name: &str,
        precision: u16,
        scale: u16,
        length: u64,
        sql_type: u16,
        ccsid: u16,
    ) -> Vec<u8> {
        let mut descriptor = standard_descriptor(name, precision, scale, length, 496, ccsid);
        descriptor[12..14].copy_from_slice(&sql_type.to_be_bytes());
        descriptor
    }

    fn standard_descriptor_with_sqlxname(
        name: &str,
        precision: u16,
        scale: u16,
        length: u64,
        sql_type: u16,
        ccsid: u16,
    ) -> Vec<u8> {
        let mut descriptor = Vec::new();
        descriptor.extend_from_slice(&precision.to_be_bytes());
        descriptor.extend_from_slice(&scale.to_be_bytes());
        descriptor.extend_from_slice(&length.to_be_bytes());
        descriptor.extend_from_slice(&sql_type.to_be_bytes());
        descriptor.extend_from_slice(&ccsid.to_be_bytes());
        descriptor.push(0x00);
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        for _ in 0..6 {
            descriptor.extend_from_slice(&0u16.to_be_bytes());
        }
        descriptor.push(0xFF);
        descriptor.push(0x00);
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        for _ in 0..7 {
            descriptor.extend_from_slice(&0u16.to_be_bytes());
        }
        descriptor.extend_from_slice(&(name.len() as u16).to_be_bytes());
        descriptor.extend_from_slice(name.as_bytes());
        descriptor.extend_from_slice(&0u16.to_be_bytes());
        descriptor
    }
}
