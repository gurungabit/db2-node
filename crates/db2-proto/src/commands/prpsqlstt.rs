//! Build PRPSQLSTT (Prepare SQL Statement) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build a PRPSQLSTT DDM command.
///
/// Parameters:
///   - pkgnamcsn: Pre-built PKGNAMCSN bytes (see commands::build_pkgnamcsn)
///   - rtnsqlda: Whether to return SQLDA (descriptor area) with the prepare response
///     - 0 = do not return
///     - 1 = return standard
///     - 2 = return extended
pub fn build_prpsqlstt(pkgnamcsn: &[u8], rtnsqlda: bool) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(PRPSQLSTT);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    if rtnsqlda {
        // RTNSQLDA: 0xF1 = EBCDIC 'Y' (yes, return SQLDA)
        ddm.add_code_point(RTNSQLDA, &[0xF1]);
        // Match Derby and request the extended output SQLDA format.
        ddm.add_code_point(TYPSQLDA, &[TYPSQLDA_X_OUTPUT as u8]);
    }
    ddm.build()
}

/// Build PRPSQLSTT requesting the SQL descriptor area.
pub fn build_prpsqlstt_with_sqlda(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_prpsqlstt(pkgnamcsn, true)
}

/// Build PRPSQLSTT without requesting the SQL descriptor area.
pub fn build_prpsqlstt_without_sqlda(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_prpsqlstt(pkgnamcsn, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_prpsqlstt() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let bytes = build_prpsqlstt_with_sqlda(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, PRPSQLSTT);
    }
}
