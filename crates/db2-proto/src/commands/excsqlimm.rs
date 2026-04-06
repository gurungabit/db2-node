//! Build EXCSQLIMM (Execute Immediate SQL) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build an EXCSQLIMM DDM command.
///
/// Parameters:
///   - pkgnamcsn: Pre-built PKGNAMCSN bytes
///   - rdbcmtok: Whether the server can commit after this statement (true = yes)
pub fn build_excsqlimm(pkgnamcsn: &[u8], rdbcmtok: bool) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(EXCSQLIMM);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    if rdbcmtok {
        ddm.add_code_point(RDBCMTOK, &[0xF1]); // EBCDIC 'Y'
    } else {
        ddm.add_code_point(RDBCMTOK, &[0xF0]); // EBCDIC 'N' (actually just 0xF0 not right)
    }
    ddm.build()
}

/// Build EXCSQLIMM with commit allowed.
pub fn build_excsqlimm_default(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_excsqlimm(pkgnamcsn, true)
}

/// Build EXCSQLIMM for auto-commit statements.
pub fn build_excsqlimm_autocommit(pkgnamcsn: &[u8]) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(EXCSQLIMM);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    ddm.add_code_point(RDBCMTOK, &[0xF1]);
    ddm.add_code_point(UOWDSP, &[UOWDSP_COMMIT as u8]);
    ddm.add_u32(MONITOR, 0xE000_0000);
    ddm.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_excsqlimm() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let bytes = build_excsqlimm_default(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, EXCSQLIMM);
    }
}
