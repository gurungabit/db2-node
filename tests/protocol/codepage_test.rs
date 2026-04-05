/// EBCDIC Code Page 037 conversion tests.
/// These tests do NOT require a DB2 server.
use db2_proto::codepage::*;

#[test]
fn test_ebcdic_roundtrip() {
    let input = "Hello, World! 0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ abcdefghijklmnopqrstuvwxyz";
    let ebcdic = utf8_to_ebcdic037(input);
    let back = ebcdic037_to_utf8(&ebcdic);
    assert_eq!(input, back, "EBCDIC roundtrip should be lossless for ASCII");
}

#[test]
fn test_ebcdic_roundtrip_special_chars() {
    // Test punctuation and special characters that exist in EBCDIC 037
    let input = "SELECT * FROM T WHERE X = 'abc' AND Y > 0;";
    let ebcdic = utf8_to_ebcdic037(input);
    let back = ebcdic037_to_utf8(&ebcdic);
    assert_eq!(input, back);
}

#[test]
fn test_ebcdic_known_values() {
    // Verify specific well-known EBCDIC 037 mappings
    let ebcdic_a = utf8_to_ebcdic037("A");
    assert_eq!(ebcdic_a[0], 0xC1, "'A' should be 0xC1 in EBCDIC 037");

    let ebcdic_space = utf8_to_ebcdic037(" ");
    assert_eq!(ebcdic_space[0], 0x40, "space should be 0x40 in EBCDIC 037");

    let ebcdic_zero = utf8_to_ebcdic037("0");
    assert_eq!(ebcdic_zero[0], 0xF0, "'0' should be 0xF0 in EBCDIC 037");

    let ebcdic_nine = utf8_to_ebcdic037("9");
    assert_eq!(ebcdic_nine[0], 0xF9, "'9' should be 0xF9 in EBCDIC 037");

    let ebcdic_z = utf8_to_ebcdic037("Z");
    assert_eq!(ebcdic_z[0], 0xE9, "'Z' should be 0xE9 in EBCDIC 037");
}

#[test]
fn test_ebcdic_lowercase() {
    let ebcdic_a = utf8_to_ebcdic037("a");
    assert_eq!(ebcdic_a[0], 0x81, "'a' should be 0x81 in EBCDIC 037");

    let ebcdic_z = utf8_to_ebcdic037("z");
    assert_eq!(ebcdic_z[0], 0xA9, "'z' should be 0xA9 in EBCDIC 037");
}

#[test]
fn test_rdbnam_padding() {
    // RDBNAM must be exactly 18 bytes, EBCDIC-encoded, right-padded with 0x40 (EBCDIC space)
    let padded = pad_rdbnam("testdb");
    assert_eq!(padded.len(), 18, "padded RDBNAM must be exactly 18 bytes");

    // Verify the first 6 bytes are EBCDIC for "testdb"
    let expected_prefix = utf8_to_ebcdic037("testdb");
    assert_eq!(
        &padded[..6],
        &expected_prefix[..],
        "prefix should be EBCDIC 'testdb'"
    );

    // Remaining 12 bytes should be EBCDIC space (0x40)
    for (i, &byte) in padded.iter().enumerate().take(18).skip(6) {
        assert_eq!(
            byte, 0x40,
            "byte {} should be EBCDIC space (0x40), got 0x{:02X}",
            i, byte
        );
    }
}

#[test]
fn test_rdbnam_padding_exact_length() {
    // If the name is exactly 18 characters, no padding is needed
    let name = "ABCDEFGHIJKLMNOPQR"; // 18 chars
    let padded = pad_rdbnam(name);
    assert_eq!(padded.len(), 18);
    // Should all be EBCDIC letters, no 0x40 padding
    let roundtrip = ebcdic037_to_utf8(&padded);
    assert_eq!(roundtrip, name);
}

#[test]
fn test_rdbnam_padding_truncation() {
    // If the name is longer than 18 characters, it should be truncated
    let name = "THIS_IS_A_VERY_LONG_DATABASE_NAME";
    let padded = pad_rdbnam(name);
    assert_eq!(padded.len(), 18, "padded RDBNAM must always be 18 bytes");
}

#[test]
fn test_pad_ebcdic_custom_length() {
    // Test pad_ebcdic with a custom length
    let padded = pad_ebcdic("AB", 8);
    assert_eq!(padded.len(), 8);
    let prefix = utf8_to_ebcdic037("AB");
    assert_eq!(&padded[..2], &prefix[..]);
    for &byte in &padded[2..8] {
        assert_eq!(byte, 0x40);
    }
}

#[test]
fn test_ebcdic_empty_string() {
    let ebcdic = utf8_to_ebcdic037("");
    assert_eq!(
        ebcdic.len(),
        0,
        "empty string should produce empty EBCDIC bytes"
    );

    let back = ebcdic037_to_utf8(&[]);
    assert_eq!(back, "", "empty EBCDIC bytes should produce empty string");
}
