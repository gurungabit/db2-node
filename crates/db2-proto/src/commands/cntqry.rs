//! Build CNTQRY (Continue Query) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build a CNTQRY DDM command.
///
/// Parameters:
///   - pkgnamcsn: Pre-built PKGNAMCSN bytes
///   - qryblksz: Query block size
///   - maxblkext: Maximum number of extra blocks
///   - nbrrow: Number of rows to fetch
pub fn build_cntqry(
    pkgnamcsn: &[u8],
    qryinsid: Option<&[u8]>,
    qryblksz: u32,
    maxblkext: Option<i16>,
    qryrowset: Option<u32>,
) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(CNTQRY);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    ddm.add_u32(QRYBLKSZ, qryblksz);
    if let Some(ext) = maxblkext {
        ddm.add_u16(MAXBLKEXT, ext as u16);
    }
    if let Some(qryinsid) = qryinsid {
        ddm.add_code_point(QRYINSID, qryinsid);
    }
    if let Some(rows) = qryrowset {
        ddm.add_u32(QRYROWSET, rows);
    }

    ddm.build()
}

/// Build CNTQRY with typical defaults.
pub fn build_cntqry_default(pkgnamcsn: &[u8]) -> Vec<u8> {
    build_cntqry(pkgnamcsn, None, 32767, Some(-1), Some(100))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_cntqry() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 1);
        let bytes = build_cntqry_default(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, CNTQRY);
    }
}
