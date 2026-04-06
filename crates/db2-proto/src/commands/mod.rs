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
    // PKGNAMCSN uses UTF-8/ASCII encoding, NOT EBCDIC
    // Each field is right-padded with spaces to a fixed width
    // RDBNAM case must match how the database was created
    data.extend_from_slice(&pad_utf8(rdbnam, 18));
    data.extend_from_slice(&pad_utf8(rdbcolid, 18));
    data.extend_from_slice(&pad_utf8(pkgid, 18));
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
