pub mod accrdb;
pub mod accsec;
pub mod clsqry;
pub mod cntqry;
pub mod drppkg;
pub mod dscsqlstt;
pub mod excsat;
pub mod excsqlimm;
pub mod excsqlstt;
pub mod opnqry;
pub mod prpsqlstt;
pub mod rdbcmm;
pub mod rdbrllbck;
pub mod secchk;
pub mod sqlattr;
pub mod sqlstt;

/// Default package collection ID (EBCDIC padded to 18 bytes).
pub const DEFAULT_RDBCOLID: &str = "NULLID";

/// Default package ID for dynamic SQL (EBCDIC padded to 18 bytes).
pub const DEFAULT_PKGID: &str = "SYSSH200";

/// Consistency token for EXCSQLSET/EXCSQLIMM (0x01 bytes).
pub const PKGCNSTKN_EXCSQLSET: [u8; 8] = [0x01; 8];

/// Consistency token for PRPSQLSTT/OPNQRY ("SYSLVL01" in ASCII).
pub const PKGCNSTKN_PRPSQLSTT: [u8; 8] = *b"SYSLVL01";

/// Default consistency token — must match UNIQUE_ID in SYSCAT.PACKAGES for SYSSH200.
pub const DEFAULT_PKGCNSTKN: [u8; 8] = *b"SYSLVL01";

/// Build a PKGNAMCSN (Package Name, Consistency Token, Section Number) byte sequence.
///
/// Format:
///   - RDBNAM: 18 bytes EBCDIC padded
///   - RDBCOLID: 18 bytes EBCDIC padded
///   - PKGID: 18 bytes EBCDIC padded
///   - PKGCNSTKN: 8 bytes (consistency token)
///   - PKGSN: 2 bytes (section number, big-endian u16)
pub fn build_pkgnamcsn(
    rdbnam: &str,
    rdbcolid: &str,
    pkgid: &str,
    pkgcnstkn: &[u8; 8],
    section_number: u16,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(64);
    // LUW accepts UTF-8/ASCII package-name fields.
    // Each field is right-padded with spaces to a fixed width
    // RDBNAM case must match how the database was created
    data.extend_from_slice(&pad_utf8(rdbnam, 18));
    data.extend_from_slice(&pad_utf8(rdbcolid, 18));
    data.extend_from_slice(&pad_utf8(pkgid, 18));
    data.extend_from_slice(pkgcnstkn);
    data.extend_from_slice(&section_number.to_be_bytes());
    data
}

/// Build a PKGNAMCSN with EBCDIC package-name fields.
///
/// Db2 for z/OS dynamic packages use EBCDIC fixed-width RDB/package fields,
/// while the consistency token remains the raw package token bytes.
pub fn build_pkgnamcsn_ebcdic_names(
    rdbnam: &str,
    rdbcolid: &str,
    pkgid: &str,
    pkgcnstkn: &[u8; 8],
    section_number: u16,
) -> Vec<u8> {
    let mut data = Vec::with_capacity(64);
    data.extend_from_slice(&crate::codepage::pad_ebcdic(rdbnam, 18));
    data.extend_from_slice(&crate::codepage::pad_ebcdic(rdbcolid, 18));
    data.extend_from_slice(&crate::codepage::pad_ebcdic(pkgid, 18));
    data.extend_from_slice(pkgcnstkn);
    data.extend_from_slice(&section_number.to_be_bytes());
    data
}

/// Pad a UTF-8 string with spaces to exactly `length` bytes.
fn pad_utf8(s: &str, length: usize) -> Vec<u8> {
    let mut bytes = s.as_bytes().to_vec();
    bytes.truncate(length);
    while bytes.len() < length {
        bytes.push(b' '); // ASCII space
    }
    bytes
}

/// Build a PKGNAMCSN with default collection ID, package ID, and consistency token.
/// Uses the EXCSQLSET token (0x01 bytes) — suitable for EXCSQLSET/EXCSQLIMM.
pub fn build_default_pkgnamcsn(rdbnam: &str, section_number: u16) -> Vec<u8> {
    build_pkgnamcsn(
        rdbnam,
        DEFAULT_RDBCOLID,
        DEFAULT_PKGID,
        &DEFAULT_PKGCNSTKN,
        section_number,
    )
}

/// Build a PKGNAMCSN for prepared statements (PRPSQLSTT/OPNQRY).
/// Uses the "SYSLVL01" consistency token.
pub fn build_query_pkgnamcsn(rdbnam: &str, section_number: u16) -> Vec<u8> {
    build_pkgnamcsn(
        rdbnam,
        DEFAULT_RDBCOLID,
        DEFAULT_PKGID,
        &PKGCNSTKN_PRPSQLSTT,
        section_number,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_pkgnamcsn_ebcdic_names() {
        let bytes = build_pkgnamcsn_ebcdic_names(
            "DBX9Q2A",
            DEFAULT_RDBCOLID,
            DEFAULT_PKGID,
            &DEFAULT_PKGCNSTKN,
            4,
        );

        assert_eq!(bytes.len(), 64);
        assert_eq!(&bytes[..7], &[0xC4, 0xC2, 0xE7, 0xF9, 0xD8, 0xF2, 0xC1]);
        assert_eq!(bytes[7], 0x40);
        assert_eq!(&bytes[54..62], b"SYSLVL01");
        assert_eq!(&bytes[62..64], &4u16.to_be_bytes());
    }
}
