/// DB2 SQL type definitions and value encoding/decoding.
use crate::{ProtoError, Result};

// ============================================================
// FD:OCA / DRDA type codes
// ============================================================
// For nullable types, the code is the non-nullable code + 1 (odd = nullable).

// Integer types
pub const DRDA_TYPE_INTEGER: u8 = 0x02;
pub const DRDA_TYPE_NINTEGER: u8 = 0x03;
pub const DRDA_TYPE_SMALLINT: u8 = 0x04;
pub const DRDA_TYPE_NSMALLINT: u8 = 0x05;
pub const DRDA_TYPE_BIGINT: u8 = 0x16;
pub const DRDA_TYPE_NBIGINT: u8 = 0x17;

// Floating point
pub const DRDA_TYPE_FLOAT4: u8 = 0x0A;
pub const DRDA_TYPE_NFLOAT4: u8 = 0x0B;
pub const DRDA_TYPE_FLOAT8: u8 = 0x0E;
pub const DRDA_TYPE_NFLOAT8: u8 = 0x0F;

// Decimal (packed BCD)
pub const DRDA_TYPE_DECIMAL: u8 = 0x0C;
pub const DRDA_TYPE_NDECIMAL: u8 = 0x0D;

// Character
pub const DRDA_TYPE_CHAR: u8 = 0x30;
pub const DRDA_TYPE_NCHAR: u8 = 0x31;
pub const DRDA_TYPE_VARCHAR: u8 = 0x32;
pub const DRDA_TYPE_NVARCHAR: u8 = 0x33;
pub const DRDA_TYPE_LONG_VARCHAR: u8 = 0x34;
pub const DRDA_TYPE_NLONG_VARCHAR: u8 = 0x35;

// Binary
pub const DRDA_TYPE_BINARY: u8 = 0x60;
pub const DRDA_TYPE_NBINARY: u8 = 0x61;
pub const DRDA_TYPE_VARBINARY: u8 = 0x62;
pub const DRDA_TYPE_NVARBINARY: u8 = 0x63;

// LOB types
pub const DRDA_TYPE_BLOB: u8 = 0xC8;
pub const DRDA_TYPE_NBLOB: u8 = 0xC9;
pub const DRDA_TYPE_CLOB: u8 = 0xCA;
pub const DRDA_TYPE_NCLOB: u8 = 0xCB;
pub const DRDA_TYPE_DBCLOB: u8 = 0xCC;
pub const DRDA_TYPE_NDBCLOB: u8 = 0xCD;

// Date/Time
pub const DRDA_TYPE_DATE: u8 = 0x20;
pub const DRDA_TYPE_NDATE: u8 = 0x21;
pub const DRDA_TYPE_TIME: u8 = 0x22;
pub const DRDA_TYPE_NTIME: u8 = 0x23;
pub const DRDA_TYPE_TIMESTAMP: u8 = 0x24;
pub const DRDA_TYPE_NTIMESTAMP: u8 = 0x25;

// Graphic (DBCS) strings
pub const DRDA_TYPE_GRAPHIC: u8 = 0x3C;
pub const DRDA_TYPE_NGRAPHIC: u8 = 0x3D;
pub const DRDA_TYPE_VARGRAPH: u8 = 0x3E;
pub const DRDA_TYPE_NVARGRAPH: u8 = 0x3F;

// Boolean
pub const DRDA_TYPE_BOOLEAN: u8 = 0xBE;
pub const DRDA_TYPE_NBOOLEAN: u8 = 0xBF;

// Decimal floating point (IEEE 754 decimal64 / decimal128)
pub const DRDA_TYPE_DECFLOAT: u8 = 0xBA;
pub const DRDA_TYPE_NDECFLOAT: u8 = 0xBB;

// XML
pub const DRDA_TYPE_XML: u8 = 0xDC;
pub const DRDA_TYPE_NXML: u8 = 0xDD;

/// SQL type enumeration.
#[derive(Debug, Clone, PartialEq)]
pub enum Db2Type {
    SmallInt,
    Integer,
    BigInt,
    Real,
    Double,
    Decimal { precision: u8, scale: u8 },
    DecFloat(u8),
    Char(u16),
    VarChar(u16),
    LongVarChar,
    Clob,
    Binary(u16),
    VarBinary(u16),
    Blob,
    Date,
    Time,
    Timestamp,
    Graphic(u16),
    VarGraphic(u16),
    DbClob,
    Boolean,
    Xml,
    Null,
}

impl Db2Type {
    /// Decode a Db2Type from a DRDA type code and length.
    /// The `nullable` return value indicates if this is a nullable variant.
    pub fn from_drda_type(type_code: u8, length: u16, precision: u8, scale: u8) -> (Self, bool) {
        let nullable = (type_code & 0x01) != 0;
        let base = type_code & 0xFE; // strip nullable bit
        let ty = match base {
            0x02 => Db2Type::Integer,
            0x04 => Db2Type::SmallInt,
            0x0A => Db2Type::Real,
            0x0C => Db2Type::Decimal { precision, scale },
            0x0E => Db2Type::Double,
            0x16 => Db2Type::BigInt,
            0xBA => Db2Type::DecFloat(if length > 8 { 34 } else { 16 }),
            0x20 => Db2Type::Date,
            0x22 => Db2Type::Time,
            0x24 => Db2Type::Timestamp,
            0x30 => Db2Type::Char(length),
            0x32 => Db2Type::VarChar(length),
            0x34 => Db2Type::LongVarChar,
            0x3C => Db2Type::Graphic(length),
            0x3E => Db2Type::VarGraphic(length),
            0x60 => Db2Type::Binary(length),
            0x62 => Db2Type::VarBinary(length),
            0xBE => Db2Type::Boolean,
            0xC8 => Db2Type::Blob,
            0xCA => Db2Type::Clob,
            0xCC => Db2Type::DbClob,
            0xDC => Db2Type::Xml,
            _ => Db2Type::VarChar(length), // fallback
        };
        (ty, nullable)
    }

    /// Return the fixed byte-length for this type, or None for variable-length types.
    pub fn fixed_length(&self) -> Option<usize> {
        match self {
            Db2Type::SmallInt => Some(2),
            Db2Type::Integer => Some(4),
            Db2Type::BigInt => Some(8),
            Db2Type::Real => Some(4),
            Db2Type::Double => Some(8),
            Db2Type::Decimal { precision, .. } => {
                // Packed decimal: ceil((precision + 1) / 2) bytes
                Some(((*precision as usize) + 2) / 2)
            }
            Db2Type::DecFloat(34) => Some(16),
            Db2Type::DecFloat(_) => Some(8),
            Db2Type::Char(len) => Some(*len as usize),
            Db2Type::Date => Some(10),
            Db2Type::Time => Some(8),
            Db2Type::Timestamp => Some(26),
            Db2Type::Boolean => Some(1),
            Db2Type::Binary(len) => Some(*len as usize),
            Db2Type::Graphic(len) => Some(*len as usize * 2),
            _ => None, // variable-length
        }
    }
}

/// Runtime value from a DB2 column.
#[derive(Debug, Clone, PartialEq)]
pub enum Db2Value {
    Null,
    SmallInt(i16),
    Integer(i32),
    BigInt(i64),
    Real(f32),
    Double(f64),
    Decimal(String),
    Char(String),
    VarChar(String),
    Binary(Vec<u8>),
    Blob(Vec<u8>),
    Clob(String),
    Date(String),
    Time(String),
    Timestamp(String),
    Boolean(bool),
    Xml(String),
}

impl Db2Value {
    /// Check if this is a Null value.
    pub fn is_null(&self) -> bool {
        matches!(self, Db2Value::Null)
    }

    /// Try to get as i64, converting from any integer type.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Db2Value::SmallInt(v) => Some(*v as i64),
            Db2Value::Integer(v) => Some(*v as i64),
            Db2Value::BigInt(v) => Some(*v),
            _ => None,
        }
    }

    /// Try to get as f64.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Db2Value::Real(v) => Some(*v as f64),
            Db2Value::Double(v) => Some(*v),
            Db2Value::SmallInt(v) => Some(*v as f64),
            Db2Value::Integer(v) => Some(*v as f64),
            Db2Value::BigInt(v) => Some(*v as f64),
            _ => None,
        }
    }

    /// Try to get as string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Db2Value::Char(s)
            | Db2Value::VarChar(s)
            | Db2Value::Clob(s)
            | Db2Value::Date(s)
            | Db2Value::Time(s)
            | Db2Value::Timestamp(s)
            | Db2Value::Decimal(s)
            | Db2Value::Xml(s) => Some(s),
            _ => None,
        }
    }
}

// ============================================================
// Encoding / Decoding helpers
// ============================================================

/// Decode a SMALLINT (2 bytes, big-endian signed).
pub fn decode_smallint(data: &[u8]) -> Result<i16> {
    if data.len() < 2 {
        return Err(ProtoError::BufferTooShort {
            expected: 2,
            actual: data.len(),
        });
    }
    Ok(i16::from_be_bytes([data[0], data[1]]))
}

/// Decode an INTEGER (4 bytes, big-endian signed).
pub fn decode_integer(data: &[u8]) -> Result<i32> {
    if data.len() < 4 {
        return Err(ProtoError::BufferTooShort {
            expected: 4,
            actual: data.len(),
        });
    }
    Ok(i32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

/// Decode a BIGINT (8 bytes, big-endian signed).
pub fn decode_bigint(data: &[u8]) -> Result<i64> {
    if data.len() < 8 {
        return Err(ProtoError::BufferTooShort {
            expected: 8,
            actual: data.len(),
        });
    }
    Ok(i64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]))
}

/// Decode a REAL / FLOAT4 (4 bytes, big-endian IEEE 754).
pub fn decode_float4(data: &[u8]) -> Result<f32> {
    if data.len() < 4 {
        return Err(ProtoError::BufferTooShort {
            expected: 4,
            actual: data.len(),
        });
    }
    Ok(f32::from_be_bytes([data[0], data[1], data[2], data[3]]))
}

/// Decode a DOUBLE / FLOAT8 (8 bytes, big-endian IEEE 754).
pub fn decode_float8(data: &[u8]) -> Result<f64> {
    if data.len() < 8 {
        return Err(ProtoError::BufferTooShort {
            expected: 8,
            actual: data.len(),
        });
    }
    Ok(f64::from_be_bytes([
        data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
    ]))
}

/// Decode a DECFLOAT value from DB2's big-endian decimal64/decimal128 interchange bytes.
pub fn decode_decfloat(data: &[u8], digits: u8) -> Result<String> {
    match digits {
        34 => {
            if data.len() < 16 {
                return Err(ProtoError::BufferTooShort {
                    expected: 16,
                    actual: data.len(),
                });
            }
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&data[..16]);
            Ok(dec::Decimal128::from_be_bytes(bytes).to_string())
        }
        _ => {
            if data.len() < 8 {
                return Err(ProtoError::BufferTooShort {
                    expected: 8,
                    actual: data.len(),
                });
            }
            let mut bytes = [0u8; 8];
            bytes.copy_from_slice(&data[..8]);
            Ok(dec::Decimal64::from_be_bytes(bytes).to_string())
        }
    }
}

/// Encode a SMALLINT to big-endian bytes.
pub fn encode_smallint(val: i16) -> [u8; 2] {
    val.to_be_bytes()
}

/// Encode an INTEGER to big-endian bytes.
pub fn encode_integer(val: i32) -> [u8; 4] {
    val.to_be_bytes()
}

/// Encode a BIGINT to big-endian bytes.
pub fn encode_bigint(val: i64) -> [u8; 8] {
    val.to_be_bytes()
}

/// Encode a REAL to big-endian bytes.
pub fn encode_float4(val: f32) -> [u8; 4] {
    val.to_be_bytes()
}

/// Encode a DOUBLE to big-endian bytes.
pub fn encode_float8(val: f64) -> [u8; 8] {
    val.to_be_bytes()
}

/// Encode a string as DB2 DECFLOAT decimal64/decimal128 bytes.
pub fn encode_decfloat(value: &str, digits: u8) -> Result<Vec<u8>> {
    match digits {
        34 => {
            let parsed = value
                .parse::<dec::Decimal128>()
                .map_err(|e| ProtoError::Other(format!("invalid DECFLOAT(34) value: {}", e)))?;
            Ok(parsed.to_be_bytes().to_vec())
        }
        _ => {
            let parsed = value
                .parse::<dec::Decimal64>()
                .map_err(|e| ProtoError::Other(format!("invalid DECFLOAT(16) value: {}", e)))?;
            Ok(parsed.to_be_bytes().to_vec())
        }
    }
}

// ============================================================
// Packed Decimal (BCD) encoding/decoding
// ============================================================

/// Decode a packed decimal (BCD) value.
///
/// Packed decimal stores two digits per byte in the high and low nibbles.
/// The last nibble is the sign: 0xC = positive, 0xD = negative, 0xF = unsigned.
pub fn decode_packed_decimal(data: &[u8], precision: u8, scale: u8) -> Result<String> {
    if data.is_empty() {
        return Err(ProtoError::Other("empty packed decimal data".into()));
    }

    let mut digits = String::new();

    // Extract all nibbles
    let mut nibbles = Vec::with_capacity(data.len() * 2);
    for &b in data {
        nibbles.push((b >> 4) & 0x0F);
        nibbles.push(b & 0x0F);
    }

    // Last nibble is the sign
    let sign_nibble = nibbles.pop().unwrap_or(0x0C);
    let is_negative = sign_nibble == 0x0D;

    // All remaining nibbles are digits
    for &n in &nibbles {
        digits.push((b'0' + n) as char);
    }

    // Strip leading zeros but keep at least one digit before decimal point
    let total_digits = digits.len();
    let integer_digits = total_digits.saturating_sub(scale as usize);

    let mut result = String::new();
    if is_negative {
        result.push('-');
    }

    if integer_digits == 0 {
        result.push('0');
    } else {
        let int_part = &digits[..integer_digits];
        let trimmed = int_part.trim_start_matches('0');
        if trimmed.is_empty() {
            result.push('0');
        } else {
            result.push_str(trimmed);
        }
    }

    if scale > 0 {
        result.push('.');
        if total_digits >= scale as usize {
            result.push_str(&digits[integer_digits..]);
        } else {
            // Need leading zeros in fractional part
            for _ in 0..(scale as usize - total_digits) {
                result.push('0');
            }
            result.push_str(&digits);
        }
    }

    let _ = precision; // used for byte-length calculation, not directly in formatting
    Ok(result)
}

/// Encode a decimal string value as packed BCD.
///
/// The string should be like "123.45" or "-67.890".
pub fn encode_packed_decimal(value: &str, precision: u8, scale: u8) -> Result<Vec<u8>> {
    let byte_len = ((precision as usize) + 2) / 2;
    let is_negative = value.starts_with('-');
    let abs_value = value.trim_start_matches('-').trim_start_matches('+');

    // Split into integer and fractional parts
    let (int_part, frac_part) = if let Some(dot_pos) = abs_value.find('.') {
        (&abs_value[..dot_pos], &abs_value[dot_pos + 1..])
    } else {
        (abs_value, "")
    };

    // Build digit string: integer digits + fractional digits padded/truncated to scale
    let mut digit_str = String::new();
    digit_str.push_str(int_part);

    let mut frac = frac_part.to_string();
    while frac.len() < scale as usize {
        frac.push('0');
    }
    frac.truncate(scale as usize);
    digit_str.push_str(&frac);

    // Total nibbles needed = precision + 1 (for sign)
    let total_nibbles = byte_len * 2;
    // Pad digit string with leading zeros
    while digit_str.len() < total_nibbles - 1 {
        digit_str.insert(0, '0');
    }
    // Truncate if too long
    if digit_str.len() > total_nibbles - 1 {
        digit_str = digit_str[digit_str.len() - (total_nibbles - 1)..].to_string();
    }

    let sign_nibble: u8 = if is_negative { 0x0D } else { 0x0C };

    let mut nibbles: Vec<u8> = digit_str.bytes().map(|b| b - b'0').collect();
    nibbles.push(sign_nibble);

    // Pack nibbles into bytes
    let mut result = Vec::with_capacity(byte_len);
    for chunk in nibbles.chunks(2) {
        let high = chunk[0];
        let low = if chunk.len() > 1 { chunk[1] } else { 0 };
        result.push((high << 4) | low);
    }

    Ok(result)
}

/// Decode a variable-length string from a buffer (2-byte length prefix + data).
/// Returns (string, bytes consumed).
pub fn decode_varlen_string(data: &[u8]) -> Result<(String, usize)> {
    if data.len() < 2 {
        return Err(ProtoError::BufferTooShort {
            expected: 2,
            actual: data.len(),
        });
    }
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let total = 2 + len;
    if data.len() < total {
        return Err(ProtoError::BufferTooShort {
            expected: total,
            actual: data.len(),
        });
    }
    let s = String::from_utf8(data[2..total].to_vec())?;
    Ok((s, total))
}

/// Decode a variable-length byte array from a buffer (2-byte length prefix + data).
/// Returns (bytes, total consumed).
pub fn decode_varlen_bytes(data: &[u8]) -> Result<(Vec<u8>, usize)> {
    if data.len() < 2 {
        return Err(ProtoError::BufferTooShort {
            expected: 2,
            actual: data.len(),
        });
    }
    let len = u16::from_be_bytes([data[0], data[1]]) as usize;
    let total = 2 + len;
    if data.len() < total {
        return Err(ProtoError::BufferTooShort {
            expected: total,
            actual: data.len(),
        });
    }
    Ok((data[2..total].to_vec(), total))
}

/// Encode a Db2Value into bytes suitable for sending as SQLDTA parameter data.
pub fn encode_db2_value(value: &Db2Value) -> Vec<u8> {
    match value {
        Db2Value::Null => {
            // Null indicator byte
            vec![0xFF]
        }
        Db2Value::SmallInt(v) => encode_smallint(*v).to_vec(),
        Db2Value::Integer(v) => encode_integer(*v).to_vec(),
        Db2Value::BigInt(v) => encode_bigint(*v).to_vec(),
        Db2Value::Real(v) => encode_float4(*v).to_vec(),
        Db2Value::Double(v) => encode_float8(*v).to_vec(),
        Db2Value::Decimal(s) => {
            // Encode as variable-length string representation
            let bytes = s.as_bytes();
            let mut out = Vec::with_capacity(2 + bytes.len());
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
            out
        }
        Db2Value::Char(s) | Db2Value::VarChar(s) | Db2Value::Clob(s) | Db2Value::Xml(s) => {
            let bytes = s.as_bytes();
            let mut out = Vec::with_capacity(2 + bytes.len());
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
            out
        }
        Db2Value::Date(s) | Db2Value::Time(s) | Db2Value::Timestamp(s) => {
            let bytes = s.as_bytes();
            let mut out = Vec::with_capacity(2 + bytes.len());
            out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
            out.extend_from_slice(bytes);
            out
        }
        Db2Value::Binary(b) | Db2Value::Blob(b) => {
            let mut out = Vec::with_capacity(2 + b.len());
            out.extend_from_slice(&(b.len() as u16).to_be_bytes());
            out.extend_from_slice(b);
            out
        }
        Db2Value::Boolean(v) => {
            vec![if *v { 1 } else { 0 }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_packed_decimal_roundtrip() {
        let encoded = encode_packed_decimal("123.45", 5, 2).unwrap();
        let decoded = decode_packed_decimal(&encoded, 5, 2).unwrap();
        assert_eq!(decoded, "123.45");
    }

    #[test]
    fn test_packed_decimal_negative() {
        let encoded = encode_packed_decimal("-42.00", 4, 2).unwrap();
        let decoded = decode_packed_decimal(&encoded, 4, 2).unwrap();
        assert_eq!(decoded, "-42.00");
    }

    #[test]
    fn test_integer_roundtrip() {
        let bytes = encode_integer(12345);
        assert_eq!(decode_integer(&bytes).unwrap(), 12345);
    }

    #[test]
    fn test_bigint_roundtrip() {
        let bytes = encode_bigint(-999_999_999_999);
        assert_eq!(decode_bigint(&bytes).unwrap(), -999_999_999_999);
    }

    #[test]
    fn test_from_drda_type() {
        let (ty, nullable) = Db2Type::from_drda_type(DRDA_TYPE_NINTEGER, 4, 0, 0);
        assert_eq!(ty, Db2Type::Integer);
        assert!(nullable);

        let (ty2, nullable2) = Db2Type::from_drda_type(DRDA_TYPE_VARCHAR, 100, 0, 0);
        assert_eq!(ty2, Db2Type::VarChar(100));
        assert!(!nullable2);

        let (ty3, nullable3) = Db2Type::from_drda_type(DRDA_TYPE_DECFLOAT, 16, 0, 0);
        assert_eq!(ty3, Db2Type::DecFloat(34));
        assert!(!nullable3);
    }

    #[test]
    fn test_decfloat_roundtrip() {
        let encoded64 = encode_decfloat("123.45", 16).unwrap();
        assert_eq!(decode_decfloat(&encoded64, 16).unwrap(), "123.45");

        let encoded128 = encode_decfloat("-987654321.00001", 34).unwrap();
        assert_eq!(
            decode_decfloat(&encoded128, 34).unwrap(),
            "-987654321.00001"
        );
    }
}
