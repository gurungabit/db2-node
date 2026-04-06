//! Build DSCSQLSTT (Describe SQL Statement) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build a DSCSQLSTT DDM command.
///
/// Parameters:
///   - pkgnamcsn: Pre-built PKGNAMCSN bytes
///   - typsqlda: Optional TYPSQLDA selector. Use `TYPSQLDA_X_INPUT` for
///     extended input descriptors.
pub fn build_dscsqlstt(pkgnamcsn: &[u8], typsqlda: Option<u8>) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(DSCSQLSTT);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    if let Some(selector) = typsqlda {
        ddm.add_code_point(TYPSQLDA, &[selector]);
    }
    ddm.build()
}

/// Build DSCSQLSTT requesting the extended input SQLDA.
pub fn build_dscsqlstt_input(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_dscsqlstt(pkgnamcsn, Some(TYPSQLDA_X_INPUT as u8))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_dscsqlstt_input() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let bytes = build_dscsqlstt_input(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, DSCSQLSTT);
    }
}
