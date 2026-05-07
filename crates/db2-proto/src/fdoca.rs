/// FD:OCA (Formatted Data Object Content Architecture) decoder.
///
/// Parses column descriptors from QRYDSC and decodes row data from QRYDTA.
use crate::types::{self, Db2Type, Db2Value};
use crate::{ProtoError, Result};
use std::env;

/// Byte order used for fixed-width numeric values in row data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ByteOrder {
    BigEndian,
    LittleEndian,
}

/// Describes a single column from the FD:OCA descriptor.
#[derive(Debug, Clone)]
pub struct ColumnDescriptor {
    pub column_index: usize,
    pub drda_type: u8,
    pub length: u16,
    pub precision: u8,
    pub scale: u8,
    pub nullable: bool,
    pub ccsid: u16,
    pub db2_type: Db2Type,
    pub byte_order: ByteOrder,
}

/// FD:OCA triplet types used in QRYDSC.
/// Each triplet is: length(1) + type(1) + data.
const TRIPLET_TYPE_SDA: u8 = 0x70; // Structured Data Area
const TRIPLET_TYPE_RLO: u8 = 0x71; // Row Layout Object
const TRIPLET_TYPE_GDA: u8 = 0x76; // Group Data Area

/// Parse FD:OCA triplets from QRYDSC payload to produce column descriptors.
///
/// The QRYDSC format:
///   - Sequence of triplets, each: length(1 byte) + type_id(1 byte) + data
///   - SDA triplets (0x70) define individual column format
///   - RLO triplets (0x71) define row structure
///   - GDA triplets (0x76) define group structure
///
/// In an SDA triplet for a column, the data typically contains:
///   - byte 0: FD:OCA type code (the DRDA type)
///   - bytes 1-2: length (u16 BE)
///   - For DECIMAL: byte 3 = precision, byte 4 = scale
///   - bytes after: CCSID (u16 BE) for character types
pub fn parse_qrydsc(data: &[u8]) -> Result<Vec<ColumnDescriptor>> {
    let mut descriptors = Vec::new();
    let mut offset = 0;
    let mut col_index = 0;

    while offset < data.len() {
        if offset + 2 > data.len() {
            break;
        }

        let triplet_len = data[offset] as usize;
        if triplet_len < 2 || offset + triplet_len > data.len() {
            break;
        }

        let triplet_type = data[offset + 1];
        let triplet_data = &data[offset + 2..offset + triplet_len];

        match triplet_type {
            TRIPLET_TYPE_SDA => {
                if let Some(desc) = parse_sda_triplet(triplet_data, col_index) {
                    descriptors.push(desc);
                    col_index += 1;
                }
            }
            TRIPLET_TYPE_RLO => {
                // Row Layout Object — contains references to column descriptors
                // We parse individual SDAs, so we mostly skip RLOs.
            }
            TRIPLET_TYPE_GDA => {
                if triplet_data.first() == Some(&0xD0) {
                    let compact_descriptors = parse_compact_gda_triplet(triplet_data, col_index);
                    col_index += compact_descriptors.len();
                    descriptors.extend(compact_descriptors);
                } else {
                    // Group Data Area — defines groups of columns
                    // Parse contained SDAs
                    let mut gda_offset = 0;
                    while gda_offset + 2 < triplet_data.len() {
                        let inner_len = triplet_data[gda_offset] as usize;
                        if inner_len < 2 || gda_offset + inner_len > triplet_data.len() {
                            break;
                        }
                        let inner_type = triplet_data[gda_offset + 1];
                        let inner_data = &triplet_data[gda_offset + 2..gda_offset + inner_len];
                        if inner_type == TRIPLET_TYPE_SDA {
                            if let Some(desc) = parse_sda_triplet(inner_data, col_index) {
                                descriptors.push(desc);
                                col_index += 1;
                            }
                        }
                        gda_offset += inner_len;
                    }
                }
            }
            _ => {
                // Unknown triplet type; skip.
            }
        }

        offset += triplet_len;
    }

    Ok(descriptors)
}

fn parse_compact_gda_triplet(data: &[u8], start_index: usize) -> Vec<ColumnDescriptor> {
    let mut descriptors = Vec::new();
    if data.len() < 4 || data[0] != 0xD0 {
        return descriptors;
    }

    let mut offset = 1;
    while offset + 2 < data.len() {
        let drda_type = data[offset];
        let attr1 = data[offset + 1];
        let attr2 = data[offset + 2];
        let (db2_type, nullable, length, precision, scale) =
            compact_gda_descriptor_type(drda_type, attr1, attr2);
        let ccsid = match db2_type {
            Db2Type::Char(_)
            | Db2Type::VarChar(_)
            | Db2Type::LongVarChar
            | Db2Type::Clob
            | Db2Type::ClobLocator
            | Db2Type::DbClobLocator
            | Db2Type::LobChar(_)
            | Db2Type::Date
            | Db2Type::Time
            | Db2Type::Timestamp
            | Db2Type::Xml => 1208,
            _ => 0,
        };

        descriptors.push(ColumnDescriptor {
            column_index: start_index + descriptors.len(),
            drda_type,
            length,
            precision,
            scale,
            nullable,
            ccsid,
            db2_type,
            byte_order: ByteOrder::BigEndian,
        });

        offset += 3;
    }

    descriptors
}

fn compact_gda_descriptor_type(
    drda_type: u8,
    attr1: u8,
    attr2: u8,
) -> (Db2Type, bool, u16, u8, u8) {
    let nullable = (drda_type & 0x01) != 0;
    let base = drda_type & 0xFE;

    if base == 0x0E {
        let precision = attr1;
        let scale = attr2;
        let length = ((precision as usize + 2) / 2) as u16;
        return (
            Db2Type::Decimal { precision, scale },
            nullable,
            length,
            precision,
            scale,
        );
    }

    let length = u16::from_be_bytes([attr1, attr2]);
    let (db2_type, nullable) = Db2Type::from_drda_type(drda_type, length, 0, 0);
    (db2_type, nullable, length, 0, 0)
}

/// Parse a single SDA (Structured Data Area) triplet's data portion.
fn parse_sda_triplet(data: &[u8], col_index: usize) -> Option<ColumnDescriptor> {
    if data.is_empty() {
        return None;
    }

    let drda_type = data[0];
    let nullable = (drda_type & 0x01) != 0;

    let length = if data.len() >= 3 {
        u16::from_be_bytes([data[1], data[2]])
    } else {
        0
    };

    let mut precision = 0u8;
    let mut scale = 0u8;
    let mut ccsid = 0u16;

    let base_type = drda_type & 0xFE;
    match base_type {
        0x0C => {
            // DECIMAL: precision and scale follow length
            if data.len() >= 5 {
                precision = data[3];
                scale = data[4];
            }
            if data.len() >= 7 {
                ccsid = u16::from_be_bytes([data[5], data[6]]);
            }
        }
        0x30 | 0x32 | 0x34 | 0x3C | 0x3E | 0xCA | 0xCC => {
            // Character/graphic types: CCSID follows length
            if data.len() >= 5 {
                ccsid = u16::from_be_bytes([data[3], data[4]]);
            }
        }
        _ => {
            if data.len() >= 5 {
                ccsid = u16::from_be_bytes([data[3], data[4]]);
            }
        }
    }

    let (db2_type, _) = Db2Type::from_drda_type(drda_type, length, precision, scale);

    Some(ColumnDescriptor {
        column_index: col_index,
        drda_type,
        length,
        precision,
        scale,
        nullable,
        ccsid,
        db2_type,
        byte_order: ByteOrder::LittleEndian,
    })
}

/// Decode a single row of data from QRYDTA bytes using column descriptors.
///
/// Returns (row values, bytes consumed).
///
/// Row format:
///   - For each column (in descriptor order):
///     - If nullable: 1 byte null indicator (0xFF = null, 0x00 = not null)
///     - If not null: data bytes (fixed or variable length depending on type)
///   - Variable-length types have a 2-byte BE length prefix before the data.
pub fn decode_row(data: &[u8], columns: &[ColumnDescriptor]) -> Result<(Vec<Db2Value>, usize)> {
    decode_row_with_prefix(data, columns, true)
}

fn decode_row_with_prefix(
    data: &[u8],
    columns: &[ColumnDescriptor],
    allow_prefix: bool,
) -> Result<(Vec<Db2Value>, usize)> {
    if allow_prefix && data.len() >= 2 && data[0] == 0xFF {
        let (values, consumed) = decode_row_body(&data[2..], columns)?;
        return Ok((values, consumed + 2));
    }

    decode_row_body(data, columns)
}

fn decode_row_body(data: &[u8], columns: &[ColumnDescriptor]) -> Result<(Vec<Db2Value>, usize)> {
    let mut values = Vec::with_capacity(columns.len());
    let mut offset = 0;

    for col in columns {
        if col.nullable && matches!(col.db2_type, Db2Type::Boolean) {
            if offset >= data.len() {
                return Err(ProtoError::BufferTooShort {
                    expected: offset + 1,
                    actual: data.len(),
                });
            }
            let marker = data[offset];
            offset += 1;
            if marker == 0xFF {
                values.push(Db2Value::Null);
            } else {
                values.push(Db2Value::Boolean(marker != 0));
            }
            continue;
        }

        // Check null indicator for nullable columns
        if col.nullable {
            if offset >= data.len() {
                return Err(ProtoError::BufferTooShort {
                    expected: offset + 1,
                    actual: data.len(),
                });
            }
            let null_ind = data[offset];
            offset += 1;
            if null_ind == 0xFF {
                values.push(Db2Value::Null);
                continue;
            }
        }

        let remaining = &data[offset..];
        let (value, consumed) = decode_column_value(remaining, col)?;
        values.push(value);
        offset += consumed;
    }

    Ok((values, offset))
}

/// Decode multiple rows from QRYDTA data.
///
/// The first byte of QRYDTA is often a consistency byte or row indicator.
/// In limited-block protocol, rows are packed sequentially until the data ends.
pub fn decode_rows(data: &[u8], columns: &[ColumnDescriptor]) -> Result<Vec<Vec<Db2Value>>> {
    let mut tail = Vec::new();
    decode_rows_with_tail(data, columns, &mut tail)
}

/// Decode multiple rows while preserving a trailing partial row across blocks.
pub fn decode_rows_with_tail(
    data: &[u8],
    columns: &[ColumnDescriptor],
    tail: &mut Vec<u8>,
) -> Result<Vec<Vec<Db2Value>>> {
    if tail.is_empty() {
        return decode_rows_from_buffer(data, columns, tail);
    }

    let mut buffer = Vec::with_capacity(tail.len() + data.len());
    buffer.extend_from_slice(tail);
    tail.clear();
    buffer.extend_from_slice(data);
    decode_rows_from_buffer(&buffer, columns, tail)
}

fn decode_rows_from_buffer(
    buffer: &[u8],
    columns: &[ColumnDescriptor],
    tail: &mut Vec<u8>,
) -> Result<Vec<Vec<Db2Value>>> {
    let mut rows = Vec::new();
    let mut offset = 0;

    while offset < buffer.len() {
        match decode_row_with_prefix(&buffer[offset..], columns, offset == 0) {
            Ok((row, consumed)) => {
                if consumed == 0 {
                    break;
                }
                rows.push(row);
                offset += consumed;
            }
            Err(ProtoError::BufferTooShort { .. }) => {
                let tail_start = find_partial_row_start(buffer, offset, columns).unwrap_or(offset);
                if env::var_os("DB2_WIRE_DEBUG_HEX").is_some() {
                    eprintln!(
                        "[db2-wire] FD:OCA partial row at offset {} of {} tail_start={} preview={}",
                        offset,
                        buffer.len(),
                        tail_start,
                        format_hex_preview(&buffer[tail_start..], 96)
                    );
                }
                tail.extend_from_slice(&buffer[tail_start..]);
                break;
            }
            Err(err) => {
                if env::var_os("DB2_WIRE_DEBUG_HEX").is_some() {
                    eprintln!(
                        "[db2-wire] FD:OCA decode stopped at offset {} of {}: {}",
                        offset,
                        buffer.len(),
                        err
                    );
                }
                return Err(err);
            }
        }
    }

    Ok(rows)
}

/// Explain how far a QRYDTA row block can be decoded with the given descriptors.
///
/// This is intended for diagnostics when a z/OS server returns data but the
/// client cannot yet materialize a row. It reports the first column/offset where
/// decoding stops, including a short byte preview around the failure.
pub fn describe_decode_progress(data: &[u8], columns: &[ColumnDescriptor]) -> String {
    if columns.is_empty() {
        return format!("no descriptors; data_len={}", data.len());
    }

    let (prefix_len, body) = if data.len() >= 2 && data[0] == 0xFF {
        (2usize, &data[2..])
    } else {
        (0usize, data)
    };

    let mut body_offset = 0usize;
    for col in columns {
        let col_start = body_offset;
        if col.nullable {
            if body_offset >= body.len() {
                return format!(
                    "stopped before null indicator: column={} abs_offset={} body_offset={} type={:?} drda=0x{:02X} remaining=0",
                    col.column_index + 1,
                    prefix_len + body_offset,
                    body_offset,
                    col.db2_type,
                    col.drda_type
                );
            }

            let null_indicator = body[body_offset];
            body_offset += 1;
            if null_indicator == 0xFF {
                continue;
            }
        }

        match decode_column_value(&body[body_offset..], col) {
            Ok((_value, consumed)) => {
                body_offset += consumed;
            }
            Err(err) => {
                return format!(
                    "stopped at column={} abs_offset={} body_offset={} col_start={} type={:?} drda=0x{:02X} len={} nullable={} ccsid={} byte_order={:?} err={} remaining={} preview={}",
                    col.column_index + 1,
                    prefix_len + body_offset,
                    body_offset,
                    col_start,
                    col.db2_type,
                    col.drda_type,
                    col.length,
                    col.nullable,
                    col.ccsid,
                    col.byte_order,
                    err,
                    body.len().saturating_sub(body_offset),
                    format_hex_preview(&body[body_offset..], 96)
                );
            }
        }
    }

    format!(
        "decoded one row: consumed={} total={} columns={} remaining={} preview={}",
        prefix_len + body_offset,
        data.len(),
        columns.len(),
        body.len().saturating_sub(body_offset),
        format_hex_preview(&body[body_offset..], 96)
    )
}

pub fn is_ignorable_final_row_tail(data: &[u8]) -> bool {
    matches!(data, [0x00] | [0xFF])
}

fn format_hex_preview(data: &[u8], max_bytes: usize) -> String {
    let take = data.len().min(max_bytes);
    let mut out = String::new();
    for (index, byte) in data[..take].iter().enumerate() {
        if index > 0 {
            if index % 16 == 0 {
                out.push_str(" | ");
            } else {
                out.push(' ');
            }
        }
        out.push_str(&format!("{:02X}", byte));
    }
    if data.len() > max_bytes {
        out.push_str(" ...");
    }
    out
}

fn find_partial_row_start(
    buffer: &[u8],
    offset: usize,
    columns: &[ColumnDescriptor],
) -> Option<usize> {
    if offset >= buffer.len() {
        return None;
    }
    if matches!(
        decode_row_strict(&buffer[offset..], columns, offset == 0),
        Err(ProtoError::BufferTooShort { .. })
    ) {
        return Some(offset);
    }

    let search_start = offset.saturating_sub(64);
    for pos in (search_start..offset).rev() {
        if buffer[pos] == 0xFF
            && pos + 1 < buffer.len()
            && buffer[pos + 1] == 0x00
            && matches!(
                decode_row_strict(&buffer[pos..], columns, pos == 0),
                Err(ProtoError::BufferTooShort { .. })
            )
        {
            return Some(pos);
        }
    }

    None
}

fn decode_row_strict(
    data: &[u8],
    columns: &[ColumnDescriptor],
    allow_prefix: bool,
) -> Result<(Vec<Db2Value>, usize)> {
    if allow_prefix && data.len() >= 2 && data[0] == 0xFF {
        let (values, consumed) = decode_row_body(&data[2..], columns)?;
        return Ok((values, consumed + 2));
    }

    decode_row_body(data, columns)
}

/// Decode a single column value from bytes based on its descriptor.
fn decode_column_value(data: &[u8], col: &ColumnDescriptor) -> Result<(Db2Value, usize)> {
    match &col.db2_type {
        Db2Type::SmallInt => {
            if data.len() < 2 {
                return Err(ProtoError::BufferTooShort {
                    expected: 2,
                    actual: data.len(),
                });
            }
            let bytes = [data[0], data[1]];
            let val = match col.byte_order {
                ByteOrder::BigEndian => i16::from_be_bytes(bytes),
                ByteOrder::LittleEndian => i16::from_le_bytes(bytes),
            };
            Ok((Db2Value::SmallInt(val), 2))
        }
        Db2Type::Integer => {
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
            }
            let bytes = [data[0], data[1], data[2], data[3]];
            let val = match col.byte_order {
                ByteOrder::BigEndian => i32::from_be_bytes(bytes),
                ByteOrder::LittleEndian => i32::from_le_bytes(bytes),
            };
            Ok((Db2Value::Integer(val), 4))
        }
        Db2Type::BigInt => {
            if data.len() < 8 {
                return Err(ProtoError::BufferTooShort {
                    expected: 8,
                    actual: data.len(),
                });
            }
            let bytes = [
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ];
            let val = match col.byte_order {
                ByteOrder::BigEndian => i64::from_be_bytes(bytes),
                ByteOrder::LittleEndian => i64::from_le_bytes(bytes),
            };
            Ok((Db2Value::BigInt(val), 8))
        }
        Db2Type::Real => {
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
            }
            let bytes = [data[0], data[1], data[2], data[3]];
            let val = match col.byte_order {
                ByteOrder::BigEndian => f32::from_be_bytes(bytes),
                ByteOrder::LittleEndian => f32::from_le_bytes(bytes),
            };
            Ok((Db2Value::Real(val), 4))
        }
        Db2Type::Double => {
            if data.len() < 8 {
                return Err(ProtoError::BufferTooShort {
                    expected: 8,
                    actual: data.len(),
                });
            }
            let bytes = [
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ];
            let val = match col.byte_order {
                ByteOrder::BigEndian => f64::from_be_bytes(bytes),
                ByteOrder::LittleEndian => f64::from_le_bytes(bytes),
            };
            Ok((Db2Value::Double(val), 8))
        }
        Db2Type::DecFloat(digits) => {
            let byte_len = if *digits >= 34 { 16 } else { 8 };
            if data.len() < byte_len {
                return Err(ProtoError::BufferTooShort {
                    expected: byte_len,
                    actual: data.len(),
                });
            }
            let val = types::decode_decfloat(&data[..byte_len], *digits)?;
            Ok((Db2Value::Decimal(val), byte_len))
        }
        Db2Type::Decimal { precision, scale } => {
            let byte_len = ((*precision as usize) + 2) / 2;
            if data.len() < byte_len {
                return Err(ProtoError::BufferTooShort {
                    expected: byte_len,
                    actual: data.len(),
                });
            }
            let val = types::decode_packed_decimal(&data[..byte_len], *precision, *scale)?;
            Ok((Db2Value::Decimal(val), byte_len))
        }
        Db2Type::Char(len) => {
            let flen = *len as usize;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let s = decode_character_bytes(&data[..flen], col.ccsid);
            Ok((Db2Value::Char(s), flen))
        }
        Db2Type::VarChar(len) => {
            if len & 0x8000 != 0 {
                return decode_externalized_lob_header(data, *len, true);
            }
            if data.len() < 2 {
                return Err(ProtoError::BufferTooShort {
                    expected: 2,
                    actual: data.len(),
                });
            }
            let str_len = u16::from_be_bytes([data[0], data[1]]) as usize;
            let total = 2 + str_len;
            if data.len() < total {
                return Err(ProtoError::BufferTooShort {
                    expected: total,
                    actual: data.len(),
                });
            }
            let s = decode_character_bytes(&data[2..total], col.ccsid);
            Ok((Db2Value::VarChar(s), total))
        }
        Db2Type::LongVarChar => {
            if data.len() < 2 {
                return Err(ProtoError::BufferTooShort {
                    expected: 2,
                    actual: data.len(),
                });
            }
            let str_len = u16::from_be_bytes([data[0], data[1]]) as usize;
            let total = 2 + str_len;
            if data.len() < total {
                return Err(ProtoError::BufferTooShort {
                    expected: total,
                    actual: data.len(),
                });
            }
            let s = decode_character_bytes(&data[2..total], col.ccsid);
            Ok((Db2Value::VarChar(s), total))
        }
        Db2Type::Binary(len) => {
            let flen = *len as usize;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            Ok((Db2Value::Binary(data[..flen].to_vec()), flen))
        }
        Db2Type::VarBinary(_) => {
            if data.len() < 2 {
                return Err(ProtoError::BufferTooShort {
                    expected: 2,
                    actual: data.len(),
                });
            }
            let bin_len = u16::from_be_bytes([data[0], data[1]]) as usize;
            let total = 2 + bin_len;
            if data.len() < total {
                return Err(ProtoError::BufferTooShort {
                    expected: total,
                    actual: data.len(),
                });
            }
            Ok((Db2Value::Binary(data[2..total].to_vec()), total))
        }
        Db2Type::Date => {
            let flen = 10;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let s = decode_character_bytes(&data[..flen], col.ccsid);
            Ok((Db2Value::Date(s), flen))
        }
        Db2Type::Time => {
            let flen = 8;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let s = decode_character_bytes(&data[..flen], col.ccsid);
            Ok((Db2Value::Time(s), flen))
        }
        Db2Type::Timestamp => {
            // Timestamp length can vary; use the descriptor length or default 26
            let flen = if col.length > 0 {
                col.length as usize
            } else {
                26
            };
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let s = decode_character_bytes(&data[..flen], col.ccsid);
            Ok((Db2Value::Timestamp(s), flen))
        }
        Db2Type::Boolean => {
            if data.is_empty() {
                return Err(ProtoError::BufferTooShort {
                    expected: 1,
                    actual: 0,
                });
            }
            Ok((Db2Value::Boolean(data[0] != 0), 1))
        }
        Db2Type::Blob | Db2Type::Clob | Db2Type::DbClob | Db2Type::Xml => {
            // LOB values may be materialized inline as a 4-byte length plus
            // bytes, or represented by a 4-byte server-side LOB locator.
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
            }
            if col.length == 0
                && matches!(
                    col.db2_type,
                    Db2Type::Blob | Db2Type::Clob | Db2Type::DbClob
                )
            {
                let locator = format_lob_locator(&data[..4]);
                return match &col.db2_type {
                    Db2Type::Blob => Ok((Db2Value::Blob(data[..4].to_vec()), 4)),
                    Db2Type::Clob | Db2Type::DbClob => Ok((Db2Value::Clob(locator), 4)),
                    _ => unreachable!(),
                };
            }
            let lob_len = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as usize;
            let total = 4 + lob_len;
            if data.len() < total {
                return Err(ProtoError::BufferTooShort {
                    expected: total,
                    actual: data.len(),
                });
            }
            match &col.db2_type {
                Db2Type::Blob => Ok((Db2Value::Blob(data[4..total].to_vec()), total)),
                Db2Type::Clob | Db2Type::DbClob => {
                    let s = String::from_utf8_lossy(&data[4..total]).to_string();
                    Ok((Db2Value::Clob(s), total))
                }
                Db2Type::Xml => {
                    let s = String::from_utf8_lossy(&data[4..total]).to_string();
                    Ok((Db2Value::Xml(s), total))
                }
                _ => unreachable!(),
            }
        }
        Db2Type::BlobLocator | Db2Type::ClobLocator | Db2Type::DbClobLocator => {
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
            }
            match &col.db2_type {
                Db2Type::BlobLocator => Ok((Db2Value::Blob(data[..4].to_vec()), 4)),
                Db2Type::ClobLocator | Db2Type::DbClobLocator => {
                    Ok((Db2Value::Clob(format_lob_locator(&data[..4])), 4))
                }
                _ => unreachable!(),
            }
        }
        Db2Type::LobBytes(len) => {
            let flen = (*len & 0x7FFF) as usize;
            if data.len() < flen {
                if is_short_externalized_lob_reference(data, flen) {
                    return Ok((Db2Value::Blob(data.to_vec()), data.len()));
                }
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let bytes = &data[..flen];
            if bytes_are_likely_text(bytes) {
                Ok((
                    Db2Value::VarChar(decode_character_bytes(bytes, col.ccsid)),
                    flen,
                ))
            } else {
                Ok((Db2Value::Blob(bytes.to_vec()), flen))
            }
        }
        Db2Type::LobChar(len) => {
            let flen = (*len & 0x7FFF) as usize;
            if data.len() < flen {
                if is_short_externalized_lob_reference(data, flen) {
                    return Ok((Db2Value::Clob(format_lob_locator(data)), data.len()));
                }
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            Ok((
                Db2Value::Clob(decode_character_bytes(&data[..flen], col.ccsid)),
                flen,
            ))
        }
        Db2Type::RowId(len) => {
            let flen = *len as usize;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            Ok((Db2Value::RowId(format_rowid(&data[..flen])), flen))
        }
        Db2Type::Graphic(len) => {
            let flen = *len as usize;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let s = decode_graphic_bytes(&data[..flen], col.ccsid);
            Ok((Db2Value::Char(s), flen))
        }
        Db2Type::VarGraphic(len) => {
            if len & 0x8000 != 0 {
                return decode_externalized_lob_header(data, *len, true);
            }
            if data.len() < 2 {
                return Err(ProtoError::BufferTooShort {
                    expected: 2,
                    actual: data.len(),
                });
            }
            let byte_len = u16::from_be_bytes([data[0], data[1]]) as usize;
            let total = 2 + byte_len;
            if data.len() < total {
                return Err(ProtoError::BufferTooShort {
                    expected: total,
                    actual: data.len(),
                });
            }
            let s = decode_graphic_bytes(&data[2..total], col.ccsid);
            Ok((Db2Value::VarChar(s), total))
        }
        Db2Type::Null => Ok((Db2Value::Null, 0)),
    }
}

fn decode_externalized_lob_header(
    data: &[u8],
    descriptor_len: u16,
    is_character: bool,
) -> Result<(Db2Value, usize)> {
    let descriptor_header_len = (descriptor_len & 0x7FFF) as usize;
    let (header_start, header_len, total) = if data.len() >= 2 {
        let value_len = u16::from_be_bytes([data[0], data[1]]);
        if value_len & 0x8000 != 0 {
            let header_len = (value_len & 0x7FFF) as usize;
            (2usize, header_len, 2 + header_len)
        } else {
            (0usize, descriptor_header_len, descriptor_header_len)
        }
    } else {
        (0usize, descriptor_header_len, descriptor_header_len)
    };

    if data.len() < total {
        return Err(ProtoError::BufferTooShort {
            expected: total,
            actual: data.len(),
        });
    }
    let header = &data[header_start..header_start + header_len];

    if is_character {
        Ok((Db2Value::Clob(format_lob_locator(header)), total))
    } else {
        Ok((Db2Value::Blob(header.to_vec()), total))
    }
}

fn format_lob_locator(bytes: &[u8]) -> String {
    format!("LOB locator 0x{}", bytes_to_hex(bytes))
}

fn is_short_externalized_lob_reference(data: &[u8], declared_len: usize) -> bool {
    declared_len >= 1024
        && (16..declared_len).contains(&data.len())
        && data.iter().filter(|byte| byte.is_ascii_hexdigit()).count() >= 16
}

fn format_rowid(bytes: &[u8]) -> String {
    format!("0x{}", bytes_to_hex(bytes))
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{:02X}", byte));
    }
    out
}

fn bytes_are_likely_text(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(text) => text
            .chars()
            .all(|ch| !ch.is_control() || matches!(ch, '\t' | '\n' | '\r')),
        Err(_) => false,
    }
}

fn decode_character_bytes(data: &[u8], ccsid: u16) -> String {
    if matches!(ccsid, 37 | 500) {
        return crate::codepage::ebcdic037_to_utf8(data);
    }

    if data.is_ascii() {
        return ascii_bytes_to_string(data);
    }

    // z/OS compact QRYDSC often omits CCSID detail even though row bytes are
    // still EBCDIC. Prefer readable text for those result blocks.
    crate::codepage::ebcdic037_to_utf8(data)
}

fn ascii_bytes_to_string(data: &[u8]) -> String {
    let mut value = String::with_capacity(data.len());
    // Safe because callers already proved every byte is ASCII, which is valid UTF-8.
    unsafe {
        value.as_mut_vec().extend_from_slice(data);
    }
    value
}

fn decode_graphic_bytes(data: &[u8], ccsid: u16) -> String {
    if matches!(ccsid, 1200 | 13488) && data.len().is_multiple_of(2) {
        let units: Vec<u16> = data
            .chunks_exact(2)
            .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
            .collect();
        return String::from_utf16_lossy(&units);
    }

    decode_character_bytes(data, ccsid)
}

/// Parse row data from QRYDTA using a simple heuristic: assume `num_columns`
/// nullable VARCHAR columns when no column descriptors are available.
///
/// This is a convenience entry point used by db2-client when column descriptors
/// have already been parsed separately. For full-fidelity decoding, prefer
/// `decode_rows` with proper `ColumnDescriptor` slices.
pub fn parse_qrydta(data: &[u8], num_columns: usize) -> Result<Vec<Vec<Db2Value>>> {
    // Build default column descriptors — treat every column as nullable VARCHAR(32672)
    // so the decoder will read a null-indicator byte + 2-byte length prefix + data.
    let columns: Vec<ColumnDescriptor> = (0..num_columns)
        .map(|i| ColumnDescriptor {
            column_index: i,
            drda_type: 0x33, // NVARCHAR (nullable)
            length: 32672,
            precision: 0,
            scale: 0,
            nullable: true,
            ccsid: 1208,
            db2_type: Db2Type::VarChar(32672),
            byte_order: ByteOrder::LittleEndian,
        })
        .collect();
    decode_rows(data, &columns)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_simple_row() {
        let cols = vec![
            ColumnDescriptor {
                column_index: 0,
                drda_type: 0x02, // INTEGER not nullable
                length: 4,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 0,
                db2_type: Db2Type::Integer,
                byte_order: ByteOrder::LittleEndian,
            },
            ColumnDescriptor {
                column_index: 1,
                drda_type: 0x33, // VARCHAR nullable
                length: 100,
                precision: 0,
                scale: 0,
                nullable: true,
                ccsid: 1208,
                db2_type: Db2Type::VarChar(100),
                byte_order: ByteOrder::LittleEndian,
            },
        ];

        // Row: integer 42 (little-endian), then null indicator 0x00, then varchar "hi"
        let mut row_data = Vec::new();
        row_data.extend_from_slice(&42i32.to_le_bytes()); // INTEGER
        row_data.push(0x00); // not null
        row_data.extend_from_slice(&2u16.to_be_bytes()); // varchar length
        row_data.extend_from_slice(b"hi");

        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Integer(42));
        assert_eq!(values[1], Db2Value::VarChar("hi".to_string()));
    }

    #[test]
    fn test_decode_null_value() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x03, // INTEGER nullable
            length: 4,
            precision: 0,
            scale: 0,
            nullable: true,
            ccsid: 0,
            db2_type: Db2Type::Integer,
            byte_order: ByteOrder::LittleEndian,
        }];

        let row_data = vec![0xFF]; // null indicator
        let (values, _) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(values[0], Db2Value::Null);
    }

    #[test]
    fn test_decode_nullable_boolean_single_byte_values() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0xBF,
            length: 2,
            precision: 0,
            scale: 0,
            nullable: true,
            ccsid: 0,
            db2_type: Db2Type::Boolean,
            byte_order: ByteOrder::BigEndian,
        }];

        let (values, consumed) = decode_row(&[0x01], &cols).unwrap();
        assert_eq!(consumed, 1);
        assert_eq!(values[0], Db2Value::Boolean(true));

        let (values, consumed) = decode_row(&[0x00], &cols).unwrap();
        assert_eq!(consumed, 1);
        assert_eq!(values[0], Db2Value::Boolean(false));

        let (values, consumed) = decode_row(&[0xFF], &cols).unwrap();
        assert_eq!(consumed, 1);
        assert_eq!(values[0], Db2Value::Null);
    }

    #[test]
    fn test_decode_nullable_boolean_uses_compact_marker_in_multi_column_rows() {
        let cols = vec![
            ColumnDescriptor {
                column_index: 0,
                drda_type: 0x02,
                length: 4,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 0,
                db2_type: Db2Type::Integer,
                byte_order: ByteOrder::LittleEndian,
            },
            ColumnDescriptor {
                column_index: 1,
                drda_type: 0xBF,
                length: 2,
                precision: 0,
                scale: 0,
                nullable: true,
                ccsid: 0,
                db2_type: Db2Type::Boolean,
                byte_order: ByteOrder::BigEndian,
            },
        ];

        let mut row_data = Vec::new();
        row_data.extend_from_slice(&1i32.to_le_bytes());
        row_data.push(0x01);

        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Integer(1));
        assert_eq!(values[1], Db2Value::Boolean(true));
    }

    #[test]
    fn test_decode_rows_applies_consistency_prefix_only_once() {
        let cols = vec![
            ColumnDescriptor {
                column_index: 0,
                drda_type: 0x02,
                length: 4,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 0,
                db2_type: Db2Type::Integer,
                byte_order: ByteOrder::LittleEndian,
            },
            ColumnDescriptor {
                column_index: 1,
                drda_type: 0xBF,
                length: 2,
                precision: 0,
                scale: 0,
                nullable: true,
                ccsid: 0,
                db2_type: Db2Type::Boolean,
                byte_order: ByteOrder::BigEndian,
            },
        ];

        let mut rows_data = vec![0xFF, 0x00];
        rows_data.extend_from_slice(&1i32.to_le_bytes());
        rows_data.push(0x01);
        rows_data.extend_from_slice(&2i32.to_le_bytes());
        rows_data.push(0x00);
        rows_data.extend_from_slice(&3i32.to_le_bytes());
        rows_data.push(0xFF);

        let rows = decode_rows(&rows_data, &cols).unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], Db2Value::Integer(1));
        assert_eq!(rows[0][1], Db2Value::Boolean(true));
        assert_eq!(rows[1][0], Db2Value::Integer(2));
        assert_eq!(rows[1][1], Db2Value::Boolean(false));
        assert_eq!(rows[2][0], Db2Value::Integer(3));
        assert_eq!(rows[2][1], Db2Value::Null);
    }

    #[test]
    fn test_decode_row_with_prefix() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x02,
            length: 4,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 0,
            db2_type: Db2Type::Integer,
            byte_order: ByteOrder::LittleEndian,
        }];

        let row_data = vec![0xFF, 0x00, 0x01, 0x00, 0x00, 0x00];
        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Integer(1));
    }

    #[test]
    fn test_decode_big_endian_integer() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x02,
            length: 4,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 0,
            db2_type: Db2Type::Integer,
            byte_order: ByteOrder::BigEndian,
        }];

        let row_data = vec![0x00, 0x00, 0x00, 0x01];
        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Integer(1));
    }

    #[test]
    fn test_parse_compact_qrydsc_uses_big_endian_row_values() {
        let qrydsc = [
            0x06, 0x76, 0xD0, 0x02, 0x00, 0x04, 0x09, 0x71, 0xE0, 0x54, 0x00, 0x01, 0xD0, 0x00,
            0x01, 0x06, 0x71, 0xF0, 0xE0, 0x00, 0x00,
        ];
        let descriptors = parse_qrydsc(&qrydsc).unwrap();
        assert_eq!(descriptors.len(), 1);
        assert_eq!(descriptors[0].byte_order, ByteOrder::BigEndian);

        let row_data = vec![0xFF, 0x00, 0x00, 0x00, 0x00, 0x01];
        let (values, consumed) = decode_row(&row_data, &descriptors).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Integer(1));
    }

    #[test]
    fn test_parse_compact_qrydsc_decimal_uses_precision_scale_bytes() {
        let qrydsc = [0x09, 0x76, 0xD0, 0x0E, 0x0B, 0x00, 0x3E, 0x00, 0x14];
        let descriptors = parse_qrydsc(&qrydsc).unwrap();
        assert_eq!(descriptors.len(), 2);
        assert_eq!(
            descriptors[0].db2_type,
            Db2Type::Decimal {
                precision: 11,
                scale: 0
            }
        );
        assert_eq!(descriptors[0].length, 6);
        assert_eq!(descriptors[0].precision, 11);
        assert_eq!(descriptors[0].scale, 0);
        assert_eq!(descriptors[1].db2_type, Db2Type::VarGraphic(20));

        let row_data = b"\xFF\x00\x00\x00\x00\x00\x00\x3C\x00\x0E112042F8730CA1";
        let (values, consumed) = decode_row(row_data, &descriptors).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Decimal("3".to_string()));
        assert_eq!(values[1], Db2Value::VarChar("112042F8730CA1".to_string()));
    }

    #[test]
    fn test_graphic_lengths_are_decoded_as_bytes() {
        let cols = vec![
            ColumnDescriptor {
                column_index: 0,
                drda_type: 0x3C,
                length: 4,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 1200,
                db2_type: Db2Type::Graphic(4),
                byte_order: ByteOrder::BigEndian,
            },
            ColumnDescriptor {
                column_index: 1,
                drda_type: 0x3E,
                length: 8,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 1200,
                db2_type: Db2Type::VarGraphic(8),
                byte_order: ByteOrder::BigEndian,
            },
        ];

        let row_data = vec![
            0x00, 0x41, 0x00, 0x42, // GRAPHIC "AB" as 4 bytes
            0x00, 0x04, 0x00, 0x43, 0x00, 0x44, // VARGRAPHIC "CD" as 4 bytes
        ];
        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Char("AB".to_string()));
        assert_eq!(values[1], Db2Value::VarChar("CD".to_string()));
    }

    #[test]
    fn test_clob_locator_consumes_four_bytes_when_lob_length_is_unknown() {
        let cols = vec![
            ColumnDescriptor {
                column_index: 0,
                drda_type: 0xCA,
                length: 0,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 1208,
                db2_type: Db2Type::Clob,
                byte_order: ByteOrder::BigEndian,
            },
            ColumnDescriptor {
                column_index: 1,
                drda_type: 0x02,
                length: 4,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 0,
                db2_type: Db2Type::Integer,
                byte_order: ByteOrder::BigEndian,
            },
        ];

        let row_data = vec![0x12, 0x34, 0x56, 0x78, 0x00, 0x00, 0x00, 0x2A];
        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(
            values[0],
            Db2Value::Clob("LOB locator 0x12345678".to_string())
        );
        assert_eq!(values[1], Db2Value::Integer(42));
    }

    #[test]
    fn test_rowid_decodes_as_hex_string() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x60,
            length: 4,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 0,
            db2_type: Db2Type::RowId(4),
            byte_order: ByteOrder::BigEndian,
        }];

        let (values, consumed) = decode_row(&[0xDE, 0xAD, 0xBE, 0xEF], &cols).unwrap();
        assert_eq!(consumed, 4);
        assert_eq!(values[0], Db2Value::RowId("0xDEADBEEF".to_string()));
    }

    #[test]
    fn test_lobbytes_consumes_fixed_length_without_varchar_prefix() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x50,
            length: 8,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 1208,
            db2_type: Db2Type::LobBytes(8),
            byte_order: ByteOrder::BigEndian,
        }];

        let (values, consumed) = decode_row(b"D356673C", &cols).unwrap();
        assert_eq!(consumed, 8);
        assert_eq!(values[0], Db2Value::VarChar("D356673C".to_string()));
    }

    #[test]
    fn test_lobbytes_short_externalized_reference_consumes_available_bytes() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x50,
            length: 4096,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 1,
            db2_type: Db2Type::LobBytes(4096),
            byte_order: ByteOrder::LittleEndian,
        }];
        let data = [
            0x00, 0x00, 0x00, 0x00, 0x00, 0x1C, 0x00, 0x00, 0xDA, 0xDB, 0xFF, 0x38, b'8', b'0',
            b'6', b'1', b'D', b'F', b'D', b'3', b'5', b'6', b'6', b'7', b'3', b'C', b'2', b'8',
            b'E',
        ];

        let (values, consumed) = decode_row(&data, &cols).unwrap();
        assert_eq!(consumed, data.len());
        assert_eq!(values[0], Db2Value::Blob(data.to_vec()));
    }

    #[test]
    fn test_lobchar_decodes_fixed_length_clob_text() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x51,
            length: 11,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 1208,
            db2_type: Db2Type::LobChar(11),
            byte_order: ByteOrder::BigEndian,
        }];

        let (values, consumed) = decode_row(b"hello clob!", &cols).unwrap();
        assert_eq!(consumed, 11);
        assert_eq!(values[0], Db2Value::Clob("hello clob!".to_string()));
    }

    #[test]
    fn test_high_bit_varchar_consumes_externalized_lob_header() {
        let cols = vec![ColumnDescriptor {
            column_index: 0,
            drda_type: 0x32,
            length: 0x8009,
            precision: 0,
            scale: 0,
            nullable: false,
            ccsid: 1208,
            db2_type: Db2Type::VarChar(0x8009),
            byte_order: ByteOrder::BigEndian,
        }];

        let data = [
            0x80, 0x09, 0x02, 0x00, 0x00, 0x00, 0x00, 0x12, 0x34, 0x56, 0x78,
        ];
        let (values, consumed) = decode_row(&data, &cols).unwrap();
        assert_eq!(consumed, 11);
        assert_eq!(
            values[0],
            Db2Value::Clob("LOB locator 0x020000000012345678".to_string())
        );
    }
}
