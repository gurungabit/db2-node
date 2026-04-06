//! Build SQLSTT (SQL Statement) DDM object.
//!
//! SQLSTT carries the SQL text. It is typically sent as a chained Object DSS
//! following a PRPSQLSTT, EXCSQLIMM, or EXCSQLSTT command.
use crate::codepoints::SQLSTT;
use crate::ddm::DdmBuilder;

/// Build an SQLSTT DDM object carrying the SQL text.
///
/// The SQLSTT data format used by DB2 LUW:
///   - 3 zero bytes (padding/flags)
///   - 2-byte big-endian SQL text length
///   - SQL text as UTF-8
///   - 0xFF terminator
pub fn build_sqlstt(sql: &str) -> Vec<u8> {
    let sql_bytes = sql.as_bytes();
    let mut ddm = DdmBuilder::new(SQLSTT);

    // 3 zero bytes (padding/flags)
    ddm.add_raw(&[0x00, 0x00, 0x00]);
    ddm.add_raw(&(sql_bytes.len() as u16).to_be_bytes());

    // SQL text
    ddm.add_raw(sql_bytes);

    // 0xFF terminator
    ddm.add_raw(&[0xFF]);

    ddm.build()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_sqlstt() {
        let sql = "SELECT * FROM employees WHERE id = 1";
        let bytes = build_sqlstt(sql);
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SQLSTT);
        // Data format: 3 zero bytes + 2-byte length + SQL text + 0xFF terminator
        assert_eq!(obj.data[0..3], [0x00, 0x00, 0x00]);
        assert_eq!(
            u16::from_be_bytes([obj.data[3], obj.data[4]]) as usize,
            sql.len()
        );
        let sql_start = 5;
        let sql_end = sql_start + sql.len();
        assert_eq!(&obj.data[sql_start..sql_end], sql.as_bytes());
        assert_eq!(obj.data[sql_end], 0xFF);
    }
}
