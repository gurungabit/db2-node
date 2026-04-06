//! Build EXCSQLSTT (Execute SQL Statement) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build an EXCSQLSTT DDM command.
///
/// Parameters:
///   - pkgnamcsn: Pre-built PKGNAMCSN bytes
///   - rtnsqlda: Whether to return SQLDA (0=no, 1=standard, 2=extended)
pub fn build_excsqlstt(pkgnamcsn: &[u8], rtnsqlda: Option<u16>) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(EXCSQLSTT);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    ddm.add_code_point(RDBCMTOK, &[0xF1]);
    if let Some(val) = rtnsqlda {
        ddm.add_u16(RTNSQLDA, val);
    }
    ddm.build()
}

/// Build EXCSQLSTT without requesting SQLDA return.
pub fn build_excsqlstt_default(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_excsqlstt(pkgnamcsn, None)
}

/// Build EXCSQLSTT for auto-commit statements.
pub fn build_excsqlstt_autocommit(pkgnamcsn: &[u8]) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(EXCSQLSTT);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    ddm.add_code_point(RDBCMTOK, &[0xF1]);
    ddm.add_code_point(UOWDSP, &[UOWDSP_COMMIT as u8]);
    ddm.add_u32(MONITOR, 0xE000_0000);
    ddm.build()
}

/// Build EXCSQLSTT requesting a result set reply.
pub fn build_excsqlstt_output(pkgnamcsn: &[u8]) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(EXCSQLSTT);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    ddm.add_code_point(RDBCMTOK, &[0xF1]);
    ddm.add_code_point(OUTEXP, &[0xF1]);
    ddm.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_excsqlstt() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let bytes = build_excsqlstt_default(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, EXCSQLSTT);
    }
}
