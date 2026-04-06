/// FD:OCA (Formatted Data Object Content Architecture) decoder.
///
/// Parses column descriptors from QRYDSC and decodes row data from QRYDTA.
use crate::types::{self, Db2Type, Db2Value};
use crate::{ProtoError, Result};
use std::env;

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
        let length = u16::from_be_bytes([data[offset + 1], data[offset + 2]]);
        let (db2_type, nullable) = Db2Type::from_drda_type(drda_type, length, 0, 0);
        let ccsid = match db2_type {
            Db2Type::Char(_)
            | Db2Type::VarChar(_)
            | Db2Type::LongVarChar
            | Db2Type::Clob
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
            precision: 0,
            scale: 0,
            nullable,
            ccsid,
            db2_type,
        });

        offset += 3;
    }

    descriptors
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
    if data.len() >= 2 && data[0] == 0xFF {
        let (values, consumed) = decode_row_body(&data[2..], columns)?;
        return Ok((values, consumed + 2));
    }

    decode_row_body(data, columns)
}

fn decode_row_body(data: &[u8], columns: &[ColumnDescriptor]) -> Result<(Vec<Db2Value>, usize)> {
    let mut values = Vec::with_capacity(columns.len());
    let mut offset = 0;

    for col in columns {
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
    let mut rows = Vec::new();
    let mut buffer = Vec::new();
    if !tail.is_empty() {
        buffer.extend_from_slice(tail);
        tail.clear();
    }
    buffer.extend_from_slice(data);

    let mut offset = 0;

    while offset < buffer.len() {
        match decode_row(&buffer[offset..], columns) {
            Ok((row, consumed)) => {
                if consumed == 0 {
                    break;
                }
                rows.push(row);
                offset += consumed;
            }
            Err(ProtoError::BufferTooShort { .. }) => {
                let tail_start = find_partial_row_start(&buffer, offset, columns).unwrap_or(offset);
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
        decode_row_strict(&buffer[offset..], columns),
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
                decode_row_strict(&buffer[pos..], columns),
                Err(ProtoError::BufferTooShort { .. })
            )
        {
            return Some(pos);
        }
    }

    None
}

fn decode_row_strict(data: &[u8], columns: &[ColumnDescriptor]) -> Result<(Vec<Db2Value>, usize)> {
    if data.len() >= 2 && data[0] == 0xFF {
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
            let val = i16::from_le_bytes([data[0], data[1]]);
            Ok((Db2Value::SmallInt(val), 2))
        }
        Db2Type::Integer => {
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
            }
            let val = i32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            Ok((Db2Value::Integer(val), 4))
        }
        Db2Type::BigInt => {
            if data.len() < 8 {
                return Err(ProtoError::BufferTooShort {
                    expected: 8,
                    actual: data.len(),
                });
            }
            let val = i64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
            Ok((Db2Value::BigInt(val), 8))
        }
        Db2Type::Real => {
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
            }
            let val = f32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            Ok((Db2Value::Real(val), 4))
        }
        Db2Type::Double => {
            if data.len() < 8 {
                return Err(ProtoError::BufferTooShort {
                    expected: 8,
                    actual: data.len(),
                });
            }
            let val = f64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]);
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
            let s = if col.ccsid == 500 || col.ccsid == 37 {
                crate::codepage::ebcdic037_to_utf8(&data[..flen])
            } else {
                String::from_utf8_lossy(&data[..flen]).to_string()
            };
            Ok((Db2Value::Char(s), flen))
        }
        Db2Type::VarChar(_) | Db2Type::LongVarChar => {
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
            let s = if col.ccsid == 500 || col.ccsid == 37 {
                crate::codepage::ebcdic037_to_utf8(&data[2..total])
            } else {
                String::from_utf8_lossy(&data[2..total]).to_string()
            };
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
            let s = if col.ccsid == 500 || col.ccsid == 37 {
                crate::codepage::ebcdic037_to_utf8(&data[..flen])
            } else {
                String::from_utf8_lossy(&data[..flen]).to_string()
            };
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
            let s = if col.ccsid == 500 || col.ccsid == 37 {
                crate::codepage::ebcdic037_to_utf8(&data[..flen])
            } else {
                String::from_utf8_lossy(&data[..flen]).to_string()
            };
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
            let s = if col.ccsid == 500 || col.ccsid == 37 {
                crate::codepage::ebcdic037_to_utf8(&data[..flen])
            } else {
                String::from_utf8_lossy(&data[..flen]).to_string()
            };
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
            // LOB types use a 4-byte length prefix
            if data.len() < 4 {
                return Err(ProtoError::BufferTooShort {
                    expected: 4,
                    actual: data.len(),
                });
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
        Db2Type::Graphic(len) => {
            let flen = (*len as usize) * 2;
            if data.len() < flen {
                return Err(ProtoError::BufferTooShort {
                    expected: flen,
                    actual: data.len(),
                });
            }
            let s = String::from_utf8_lossy(&data[..flen]).to_string();
            Ok((Db2Value::Char(s), flen))
        }
        Db2Type::VarGraphic(_) => {
            if data.len() < 2 {
                return Err(ProtoError::BufferTooShort {
                    expected: 2,
                    actual: data.len(),
                });
            }
            let char_len = u16::from_be_bytes([data[0], data[1]]) as usize;
            let byte_len = char_len * 2;
            let total = 2 + byte_len;
            if data.len() < total {
                return Err(ProtoError::BufferTooShort {
                    expected: total,
                    actual: data.len(),
                });
            }
            let s = String::from_utf8_lossy(&data[2..total]).to_string();
            Ok((Db2Value::VarChar(s), total))
        }
        Db2Type::Null => Ok((Db2Value::Null, 0)),
    }
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
        }];

        let row_data = vec![0xFF]; // null indicator
        let (values, _) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(values[0], Db2Value::Null);
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
        }];

        let row_data = vec![0xFF, 0x00, 0x01, 0x00, 0x00, 0x00];
        let (values, consumed) = decode_row(&row_data, &cols).unwrap();
        assert_eq!(consumed, row_data.len());
        assert_eq!(values[0], Db2Value::Integer(1));
    }
}
