//! Parse SQLCARD (SQL Communications Area Reply Data).
use crate::codepage::ebcdic037_to_utf8;
///
/// SQLCARD contains:
///   - Null indicator (1 byte): 0xFF = null (success, no details), 0x00 = has data
///   - If not null:
///     - SQLCODE (4 bytes, BE i32): negative=error, positive=warning, 0=success
///     - SQLSTATE (5 bytes, EBCDIC)
///     - SQLERRPROC (8 bytes, EBCDIC)
///     - SQLCAXGRP: error detail group
///       - SQLERRD (6 x i32 = 24 bytes) — sqlerrd[2] (3rd element) = row count for DML
///       - SQLWARN (11 bytes of warning flags)
///       - SQLERRMC (length-prefixed EBCDIC string with tokens separated by 0xFF)
use crate::codepoints::SQLCARD;
use crate::ddm::DdmObject;
use crate::{ProtoError, Result};

/// Parsed SQL Communications Area.
#[derive(Debug, Clone)]
pub struct SqlCard {
    /// True if this is a null SQLCARD (meaning simple success with no details).
    pub is_null: bool,
    /// SQL return code: negative = error, 0 = success, positive = warning.
    pub sqlcode: i32,
    /// SQLSTATE (5-character string, e.g., "00000", "42S02").
    pub sqlstate: String,
    /// Procedure that generated the error.
    pub sqlerrproc: String,
    /// SQLERRD array (6 values).
    /// sqlerrd[2] typically contains the row count for INSERT/UPDATE/DELETE.
    pub sqlerrd: [i32; 6],
    /// Rows fetched, when present in the compact SQLCAXGRP.
    pub rows_fetched: u64,
    /// Rows updated, when present in the compact SQLCAXGRP.
    pub rows_updated: u32,
    /// SQL warning flags (11 bytes).
    pub sqlwarn: [u8; 11],
    /// Error message tokens (separated by 0xFF in the original encoding).
    pub sqlerrmc: String,
}

impl SqlCard {
    /// Create a null (success) SQLCARD.
    pub fn success() -> Self {
        Self {
            is_null: true,
            sqlcode: 0,
            sqlstate: "00000".to_string(),
            sqlerrproc: String::new(),
            sqlerrd: [0; 6],
            rows_fetched: 0,
            rows_updated: 0,
            sqlwarn: [0; 11],
            sqlerrmc: String::new(),
        }
    }

    /// Is this a success result (SQLCODE >= 0)?
    pub fn is_success(&self) -> bool {
        self.sqlcode >= 0
    }

    /// Is this an error result (SQLCODE < 0)?
    pub fn is_error(&self) -> bool {
        self.sqlcode < 0
    }

    /// Is this a warning (SQLCODE > 0)?
    pub fn is_warning(&self) -> bool {
        self.sqlcode > 0
    }

    /// Get the row count from sqlerrd[2] (the third element).
    pub fn row_count(&self) -> i32 {
        if self.rows_updated != 0 {
            self.rows_updated as i32
        } else if self.rows_fetched != 0 {
            self.rows_fetched as i32
        } else {
            self.sqlerrd[2]
        }
    }
}

/// Parse an SQLCARD DDM object.
pub fn parse_sqlcard(obj: &DdmObject) -> Result<SqlCard> {
    if obj.code_point != SQLCARD {
        return Err(ProtoError::UnexpectedReply {
            expected: SQLCARD,
            actual: obj.code_point,
        });
    }

    parse_sqlcard_data(&obj.data)
}

/// Parse SQLCARD from raw data bytes (without the DDM header).
pub fn parse_sqlcard_data(data: &[u8]) -> Result<SqlCard> {
    if data.is_empty() {
        return Ok(SqlCard::success());
    }

    // First byte: null indicator
    let null_indicator = data[0];
    if null_indicator == 0xFF {
        return Ok(SqlCard::success());
    }

    // Minimum: 1 (SQLCA flag) + 4 (sqlcode) + 5 (sqlstate) + 8 (sqlerrproc) + 1 (SQLCAX flag)
    if data.len() < 19 {
        return Err(ProtoError::InvalidSqlcard(format!(
            "SQLCARD data too short: {} bytes",
            data.len()
        )));
    }

    let mut offset = 1;

    // SQLCODE: 4 bytes little-endian i32 (when using QTDSQLX86 TYPDEFNAM)
    let sqlcode = i32::from_le_bytes([
        data[offset],
        data[offset + 1],
        data[offset + 2],
        data[offset + 3],
    ]);
    offset += 4;

    // SQLSTATE: 5 bytes — may be EBCDIC or UTF-8 depending on negotiated CCSID
    // Detect: if bytes are in ASCII range (0x30-0x5A), treat as UTF-8
    let sqlstate_bytes = &data[offset..offset + 5];
    let sqlstate = if sqlstate_bytes.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        String::from_utf8_lossy(sqlstate_bytes).to_string()
    } else {
        ebcdic037_to_utf8(sqlstate_bytes)
    };
    offset += 5;

    // SQLERRPROC: 8 bytes — may be EBCDIC or UTF-8
    let sqlerrproc_bytes = &data[offset..offset + 8];
    let sqlerrproc = if sqlerrproc_bytes.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        String::from_utf8_lossy(sqlerrproc_bytes).to_string()
    } else {
        ebcdic037_to_utf8(sqlerrproc_bytes)
    };
    offset += 8;

    // SQLCAXGRP - may or may not be present
    let mut sqlerrd = [0i32; 6];
    let mut rows_fetched = 0u64;
    let mut rows_updated = 0u32;
    let mut sqlwarn = [0u8; 11];
    let mut sqlerrmc = String::new();

    // Check for SQLCAXGRP null indicator
    if offset < data.len() {
        let caxgrp_null = data[offset];
        offset += 1;

        if caxgrp_null != 0xFF && offset + 24 <= data.len() {
            // Compact LUW SQLCAXGRP:
            // rows_fetched (u64 LE), rows_updated (u32 LE), sqlerrd[0..3] raw bytes.
            rows_fetched = u64::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
                data[offset + 4],
                data[offset + 5],
                data[offset + 6],
                data[offset + 7],
            ]);
            offset += 8;

            rows_updated = u32::from_le_bytes([
                data[offset],
                data[offset + 1],
                data[offset + 2],
                data[offset + 3],
            ]);
            offset += 4;

            for item in sqlerrd.iter_mut().take(3) {
                *item = i32::from_le_bytes([
                    data[offset],
                    data[offset + 1],
                    data[offset + 2],
                    data[offset + 3],
                ]);
                offset += 4;
            }

            // SQLWARN: 11 bytes
            if offset + 11 <= data.len() {
                sqlwarn.copy_from_slice(&data[offset..offset + 11]);
                offset += 11;
            }

            // SQLRDBNAME, SQLERRMSGM, SQLERRMSGS use 2-byte BE lengths.
            let (_rdbname, next) = decode_len_prefixed_string(data, offset)?;
            offset = next;
            let (errm, next) = decode_len_prefixed_string(data, offset)?;
            offset = next;
            let (errs, _) = decode_len_prefixed_string(data, offset)?;
            sqlerrmc = if !errm.is_empty() { errm } else { errs };

            if rows_updated != 0 {
                sqlerrd[2] = rows_updated as i32;
            } else if rows_fetched != 0 {
                sqlerrd[2] = rows_fetched as i32;
            }
        }
    }

    Ok(SqlCard {
        is_null: false,
        sqlcode,
        sqlstate,
        sqlerrproc,
        sqlerrd,
        rows_fetched,
        rows_updated,
        sqlwarn,
        sqlerrmc,
    })
}

fn decode_len_prefixed_string(data: &[u8], offset: usize) -> Result<(String, usize)> {
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

    let bytes = &data[start..end];
    let text = if bytes.iter().all(|&b| (0x20..=0x7E).contains(&b)) {
        String::from_utf8_lossy(bytes).to_string()
    } else {
        ebcdic037_to_utf8(bytes)
    };
    Ok((text, end))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_null_sqlcard() {
        let data = vec![0xFF];
        let card = parse_sqlcard_data(&data).unwrap();
        assert!(card.is_null);
        assert!(card.is_success());
        assert_eq!(card.sqlcode, 0);
    }

    #[test]
    fn test_sqlcard_with_data() {
        let mut data = Vec::new();
        data.push(0x00); // not null
        data.extend_from_slice(&100i32.to_le_bytes()); // sqlcode = 100 (no data found), little-endian
        data.extend_from_slice(b"02000"); // sqlstate "02000" in ASCII (end of data)
        data.extend_from_slice(b"SQLPROC1"); // sqlerrproc in ASCII
        data.push(0xFF); // SQLCAXGRP null indicator (null = no SQLCAX)

        let card = parse_sqlcard_data(&data).unwrap();
        assert!(!card.is_null);
        assert_eq!(card.sqlcode, 100);
        assert!(card.is_warning());
    }
}
