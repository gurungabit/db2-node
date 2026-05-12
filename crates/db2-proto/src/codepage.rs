//! EBCDIC Code Page 037 (US/Canada/Netherlands) to Unicode/ASCII conversion.

/// EBCDIC 037 -> Unicode mapping table (256 entries).
/// Each index is an EBCDIC byte value; the value is the corresponding Unicode code point.
#[rustfmt::skip]
pub static EBCDIC_037_TO_UNICODE: [u16; 256] = [
    // 0x00-0x0F
    0x0000, 0x0001, 0x0002, 0x0003, 0x009C, 0x0009, 0x0086, 0x007F,
    0x0097, 0x008D, 0x008E, 0x000B, 0x000C, 0x000D, 0x000E, 0x000F,
    // 0x10-0x1F
    0x0010, 0x0011, 0x0012, 0x0013, 0x009D, 0x0085, 0x0008, 0x0087,
    0x0018, 0x0019, 0x0092, 0x008F, 0x001C, 0x001D, 0x001E, 0x001F,
    // 0x20-0x2F
    0x0080, 0x0081, 0x0082, 0x0083, 0x0084, 0x000A, 0x0017, 0x001B,
    0x0088, 0x0089, 0x008A, 0x008B, 0x008C, 0x0005, 0x0006, 0x0007,
    // 0x30-0x3F
    0x0090, 0x0091, 0x0016, 0x0093, 0x0094, 0x0095, 0x0096, 0x0004,
    0x0098, 0x0099, 0x009A, 0x009B, 0x0014, 0x0015, 0x009E, 0x001A,
    // 0x40-0x4F
    0x0020, 0x00A0, 0x00E2, 0x00E4, 0x00E0, 0x00E1, 0x00E3, 0x00E5,
    0x00E7, 0x00F1, 0x00A2, 0x002E, 0x003C, 0x0028, 0x002B, 0x007C,
    // 0x50-0x5F
    0x0026, 0x00E9, 0x00EA, 0x00EB, 0x00E8, 0x00ED, 0x00EE, 0x00EF,
    0x00EC, 0x00DF, 0x0021, 0x0024, 0x002A, 0x0029, 0x003B, 0x00AC,
    // 0x60-0x6F
    0x002D, 0x002F, 0x00C2, 0x00C4, 0x00C0, 0x00C1, 0x00C3, 0x00C5,
    0x00C7, 0x00D1, 0x00A6, 0x002C, 0x0025, 0x005F, 0x003E, 0x003F,
    // 0x70-0x7F
    0x00F8, 0x00C9, 0x00CA, 0x00CB, 0x00C8, 0x00CD, 0x00CE, 0x00CF,
    0x00CC, 0x0060, 0x003A, 0x0023, 0x0040, 0x0027, 0x003D, 0x0022,
    // 0x80-0x8F
    0x00D8, 0x0061, 0x0062, 0x0063, 0x0064, 0x0065, 0x0066, 0x0067,
    0x0068, 0x0069, 0x00AB, 0x00BB, 0x00F0, 0x00FD, 0x00FE, 0x00B1,
    // 0x90-0x9F
    0x00B0, 0x006A, 0x006B, 0x006C, 0x006D, 0x006E, 0x006F, 0x0070,
    0x0071, 0x0072, 0x00AA, 0x00BA, 0x00E6, 0x00B8, 0x00C6, 0x00A4,
    // 0xA0-0xAF
    0x00B5, 0x007E, 0x0073, 0x0074, 0x0075, 0x0076, 0x0077, 0x0078,
    0x0079, 0x007A, 0x00A1, 0x00BF, 0x00D0, 0x00DD, 0x00DE, 0x00AE,
    // 0xB0-0xBF
    0x005E, 0x00A3, 0x00A5, 0x00B7, 0x00A9, 0x00A7, 0x00B6, 0x00BC,
    0x00BD, 0x00BE, 0x005B, 0x005D, 0x00AF, 0x00A8, 0x00B4, 0x00D7,
    // 0xC0-0xCF
    0x007B, 0x0041, 0x0042, 0x0043, 0x0044, 0x0045, 0x0046, 0x0047,
    0x0048, 0x0049, 0x00AD, 0x00F4, 0x00F6, 0x00F2, 0x00F3, 0x00F5,
    // 0xD0-0xDF
    0x007D, 0x004A, 0x004B, 0x004C, 0x004D, 0x004E, 0x004F, 0x0050,
    0x0051, 0x0052, 0x00B9, 0x00FB, 0x00FC, 0x00F9, 0x00FA, 0x00FF,
    // 0xE0-0xEF
    0x005C, 0x00F7, 0x0053, 0x0054, 0x0055, 0x0056, 0x0057, 0x0058,
    0x0059, 0x005A, 0x00B2, 0x00D4, 0x00D6, 0x00D2, 0x00D3, 0x00D5,
    // 0xF0-0xFF
    0x0030, 0x0031, 0x0032, 0x0033, 0x0034, 0x0035, 0x0036, 0x0037,
    0x0038, 0x0039, 0x00B3, 0x00DB, 0x00DC, 0x00D9, 0x00DA, 0x009F,
];

// We avoid external crates. Use std::sync::OnceLock for safe one-time initialization.
static UNICODE_TO_EBCDIC_037_TABLE: std::sync::OnceLock<[u8; 256]> = std::sync::OnceLock::new();

fn get_unicode_to_ebcdic_table() -> &'static [u8; 256] {
    UNICODE_TO_EBCDIC_037_TABLE.get_or_init(|| {
        let mut table = [0x3Fu8; 256];
        for i in 0..256u16 {
            let unicode_val = EBCDIC_037_TO_UNICODE[i as usize];
            if unicode_val < 256 {
                table[unicode_val as usize] = i as u8;
            }
        }
        table
    })
}

/// Convert EBCDIC Code Page 037 bytes to a UTF-8 String.
pub fn ebcdic037_to_utf8(input: &[u8]) -> String {
    let mut s = String::with_capacity(input.len());
    for &b in input {
        let unicode = EBCDIC_037_TO_UNICODE[b as usize];
        if let Some(c) = char::from_u32(unicode as u32) {
            s.push(c);
        } else {
            s.push('?');
        }
    }
    s
}

/// Convert a UTF-8 string to EBCDIC Code Page 037 bytes.
/// Characters outside the Latin-1 range (> U+00FF) are mapped to EBCDIC '?' (0x3F).
pub fn utf8_to_ebcdic037(input: &str) -> Vec<u8> {
    let table = get_unicode_to_ebcdic_table();
    let mut out = Vec::with_capacity(input.len());
    for c in input.chars() {
        let cp = c as u32;
        if cp < 256 {
            out.push(table[cp as usize]);
        } else {
            out.push(0x3F); // EBCDIC '?'
        }
    }
    out
}

/// Convert a string to EBCDIC 037 and right-pad with EBCDIC spaces (0x40)
/// to exactly `length` bytes.
pub fn pad_ebcdic(name: &str, length: usize) -> Vec<u8> {
    let mut ebcdic = utf8_to_ebcdic037(name);
    ebcdic.truncate(length);
    while ebcdic.len() < length {
        ebcdic.push(0x40); // EBCDIC space
    }
    ebcdic
}

/// Encode a DRDA RDBNAM.
///
/// Db2 for z/OS location names are limited to 16 bytes and can reject padded
/// 18-byte values. Send the actual trimmed name length instead of padding.
pub fn pad_rdbnam(db_name: &str) -> Vec<u8> {
    let mut rdbnam = utf8_to_ebcdic037(db_name.trim());
    rdbnam.truncate(16);
    rdbnam
}

/// Alias for utf8_to_ebcdic037 (used by db2-client).
pub fn string_to_ebcdic(input: &str) -> Vec<u8> {
    utf8_to_ebcdic037(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_ascii() {
        let input = "HELLO WORLD 123";
        let ebcdic = utf8_to_ebcdic037(input);
        let back = ebcdic037_to_utf8(&ebcdic);
        assert_eq!(input, back);
    }

    #[test]
    fn test_known_mappings() {
        // A=0xC1, Z=0xE9, 0=0xF0, 9=0xF9, space=0x40
        let table = get_unicode_to_ebcdic_table();
        assert_eq!(table[b'A' as usize], 0xC1);
        assert_eq!(table[b'Z' as usize], 0xE9);
        assert_eq!(table[b'0' as usize], 0xF0);
        assert_eq!(table[b'9' as usize], 0xF9);
        assert_eq!(table[b' ' as usize], 0x40);
    }

    #[test]
    fn test_pad_rdbnam() {
        let padded = pad_rdbnam("TESTDB");
        assert_eq!(padded.len(), 6);
        // First 6 bytes should be EBCDIC for "TESTDB"
        let back = ebcdic037_to_utf8(&padded);
        assert_eq!(back, "TESTDB");
    }

    #[test]
    fn test_pad_rdbnam_limits_zos_name_to_sixteen_bytes() {
        let padded = pad_rdbnam("ABCDEFGHIJKLMNOPQR");
        assert_eq!(padded.len(), 16);

        let back = ebcdic037_to_utf8(&padded);
        assert_eq!(back, "ABCDEFGHIJKLMNOP");
    }

    #[test]
    fn test_pad_rdbnam_trims_outer_whitespace() {
        let padded = pad_rdbnam(" DBX9Q2A ");
        let back = ebcdic037_to_utf8(&padded);
        assert_eq!(back, "DBX9Q2A");
    }
}
