//! Build SECCHK (Security Check) command.
use crate::codepage::{pad_rdbnam, utf8_to_ebcdic037};
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build a SECCHK DDM command with user ID, password, and database name.
///
/// Parameters:
///   - security_mechanism: Security mechanism code
///   - rdbnam: Database name (included for DB2 LUW compatibility)
///   - user_id: User ID (will be EBCDIC-encoded)
///   - password: Password (will be EBCDIC-encoded)
pub fn build_secchk(
    security_mechanism: u16,
    rdbnam: &str,
    user_id: &str,
    password: &str,
) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(SECCHK);
    ddm.add_u16(SECMEC, security_mechanism);
    ddm.add_code_point(RDBNAM, &pad_rdbnam(rdbnam));
    ddm.add_code_point(USRID, &utf8_to_ebcdic037(user_id));
    ddm.add_code_point(PASSWORD, &utf8_to_ebcdic037(password));
    ddm.build()
}

/// Build SECCHK for user ID + password authentication.
pub fn build_secchk_usridpwd(rdbnam: &str, user_id: &str, password: &str) -> Vec<u8> {
    build_secchk(SECMEC_USRIDPWD, rdbnam, user_id, password)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_secchk() {
        let bytes = build_secchk_usridpwd("testdb", "db2inst1", "password123");
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SECCHK);
        let params = obj.parameters();
        assert!(params.iter().any(|p| p.code_point == RDBNAM));
        assert!(params.iter().any(|p| p.code_point == USRID));
        assert!(params.iter().any(|p| p.code_point == PASSWORD));
    }
}
