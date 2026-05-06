//! Build SQLATTR (SQL Statement Attributes) DDM object.
use crate::codepoints::SQLATTR;
use crate::ddm::DdmBuilder;

/// Build SQLATTR for a read-only cursor.
///
/// Db2 for z/OS expects SELECT statements opened through OPNQRY to be prepared
/// as cursors. IBM requester traces include this attribute before SQLSTT.
pub fn build_sqlattr_for_read_only_cursor() -> Vec<u8> {
    let attribute = "FOR FETCH ONLY";
    let mut ddm = DdmBuilder::new(SQLATTR);
    ddm.add_raw(&crate::commands::sqlstt::build_zos_nocs_payload(attribute));
    ddm.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_sqlattr_for_read_only_cursor() {
        let bytes = build_sqlattr_for_read_only_cursor();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SQLATTR);
        assert_eq!(&obj.data[..2], &[0xFF, 0x00]);
        assert_eq!(
            u32::from_be_bytes([obj.data[2], obj.data[3], obj.data[4], obj.data[5]]),
            14
        );
        assert_eq!(&obj.data[6..], b"FOR FETCH ONLY");
    }
}
