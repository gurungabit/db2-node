//! Build EXCSAT (Exchange Server Attributes) command.
use crate::codepoints::*;
use crate::ddm::DdmBuilder;

/// Manager level entry: (manager code point, level).
#[derive(Debug, Clone)]
pub struct ManagerLevel {
    pub code_point: u16,
    pub level: u16,
}

/// Default manager levels for a DB2 client (matching pydrda/DB2 CLI).
pub fn default_manager_levels() -> Vec<ManagerLevel> {
    vec![
        ManagerLevel {
            code_point: AGENT,
            level: 10,
        },
        ManagerLevel {
            code_point: SQLAM,
            level: 11,
        },
        ManagerLevel {
            code_point: CMNTCPIP,
            level: 5,
        },
        ManagerLevel {
            code_point: RDB,
            level: 12,
        },
        ManagerLevel {
            code_point: SECMGR,
            level: 9,
        },
        ManagerLevel {
            code_point: UNICODEMGR,
            level: 1208,
        },
    ]
}

/// Build an EXCSAT DDM command.
///
/// Parameters:
///   - external_name: Client external name (e.g., "db2driver-node")
///   - server_name: Client server name (e.g., hostname)
///   - class_name: Server class name (e.g., "QDB2/JVM")
///   - product_level: Server release level (e.g., "JCC04200")
///   - manager_levels: List of manager code points and their supported levels
pub fn build_excsat(
    external_name: &str,
    server_name: &str,
    class_name: &str,
    product_level: &str,
    manager_levels: &[ManagerLevel],
) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(EXCSAT);

    // EXCSAT strings are EBCDIC-encoded (before CCSID negotiation)
    ddm.add_ebcdic_string(EXTNAM, external_name);
    ddm.add_ebcdic_string(SRVNAM, server_name);
    ddm.add_ebcdic_string(SRVRLSLV, product_level);
    // Note: SRVCLSNM is also EBCDIC but we add it after MGRLVLLS (matching pydrda order)

    // MGRLVLLS: encoded as a sequence of (code_point: u16, level: u16) pairs
    // wrapped in a single parameter.
    let mut mgr_data = Vec::with_capacity(manager_levels.len() * 4);
    for ml in manager_levels {
        mgr_data.extend_from_slice(&ml.code_point.to_be_bytes());
        mgr_data.extend_from_slice(&ml.level.to_be_bytes());
    }
    ddm.add_code_point(MGRLVLLS, &mgr_data);
    ddm.add_ebcdic_string(SRVCLSNM, class_name);

    ddm.build()
}

/// Build an EXCSAT with sensible defaults matching DB2 CLI client.
pub fn build_excsat_default() -> Vec<u8> {
    build_excsat(
        "db2wire",
        "db2wire-client",
        "QDB2/JVM",
        "SQL11014",
        &default_manager_levels(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddm::DdmObject;

    #[test]
    fn test_build_excsat() {
        let bytes = build_excsat_default();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, EXCSAT);
        let params = obj.parameters();
        assert!(params.iter().any(|p| p.code_point == EXTNAM));
        assert!(params.iter().any(|p| p.code_point == MGRLVLLS));
    }
}
