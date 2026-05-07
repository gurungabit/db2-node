//! Build CLSQRY (Close Query) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build a CLSQRY DDM command.
///
/// Parameters:
///   - pkgnamcsn: Pre-built PKGNAMCSN bytes identifying the query to close
pub fn build_clsqry(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_clsqry_with_qryinsid(pkgnamcsn, None)
}

/// Build a CLSQRY DDM command with an optional query instance id.
///
/// Db2 for z/OS returns QRYINSID in OPNQRYRM and expects follow-up query
/// commands for that cursor to carry it when available.
pub fn build_clsqry_with_qryinsid(pkgnamcsn: &[u8], qryinsid: Option<&[u8]>) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(CLSQRY);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    if let Some(qryinsid) = qryinsid {
        ddm.add_code_point(QRYINSID, qryinsid);
    }
    ddm.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_clsqry() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let bytes = build_clsqry(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, CLSQRY);
    }

    #[test]
    fn test_build_clsqry_with_qryinsid() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let qryinsid = [0x00, 0x00, 0x00, 0x07];
        let bytes = build_clsqry_with_qryinsid(&pkgnamcsn, Some(&qryinsid));
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        let params = obj.parameters();

        assert_eq!(obj.code_point, CLSQRY);
        assert!(params
            .iter()
            .any(|param| param.code_point == QRYINSID && param.data == qryinsid));
    }
}
