//! Build DRPPKG (Drop Package) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Build a DRPPKG command for a single prepared statement package.
pub fn build_drppkg(pkgnamcsn: &[u8]) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(DRPPKG);
    ddm.add_code_point(PKGNAMCSN, pkgnamcsn);
    ddm.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::build_default_pkgnamcsn;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_drppkg() {
        let pkgnamcsn = build_default_pkgnamcsn("TESTDB", 65);
        let bytes = build_drppkg(&pkgnamcsn);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, DRPPKG);

        let params = obj.parameters();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].code_point, PKGNAMCSN);
        assert_eq!(params[0].data, pkgnamcsn);
    }
}
