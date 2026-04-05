use db2_proto::codepoints;
/// Protocol-level unit tests for DDM (Distributed Data Management) building and parsing.
/// These tests do NOT require a DB2 server.
use db2_proto::ddm::*;

// ---------------------------------------------------------------------------
// Basic DDM build / parse
// ---------------------------------------------------------------------------

#[test]
fn test_ddm_build_simple() {
    // Build a DDM with a single string parameter
    let mut builder = DdmBuilder::new(codepoints::EXCSAT);
    builder.add_string(codepoints::EXTNAM, "db2wire_test");
    let bytes = builder.build();

    // DDM header is 4 bytes: length (u16 BE) + code point (u16 BE)
    assert!(bytes.len() >= 4, "DDM must have at least a header");
    let ddm_length = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    assert_eq!(
        ddm_length,
        bytes.len(),
        "DDM length field must match actual length"
    );

    let code_point = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(code_point, codepoints::EXCSAT);
}

#[test]
fn test_ddm_parse_simple() {
    let mut builder = DdmBuilder::new(codepoints::EXCSAT);
    builder.add_string(codepoints::SRVNAM, "TestServer");
    let bytes = builder.build();

    let (ddm, _consumed) = DdmObject::parse(&bytes).expect("should parse DDM");
    assert_eq!(ddm.code_point, codepoints::EXCSAT);
    assert!(!ddm.data.is_empty(), "DDM should have payload");
}

#[test]
fn test_ddm_nested_params() {
    let mut builder = DdmBuilder::new(codepoints::EXCSAT);
    builder.add_string(codepoints::EXTNAM, "db2wire");
    builder.add_string(codepoints::SRVNAM, "myserver");
    builder.add_string(codepoints::SRVRLSLV, "V12R1M0");
    let bytes = builder.build();

    let (ddm, _) = DdmObject::parse(&bytes).expect("should parse");
    let params = ddm.parameters();
    assert_eq!(params.len(), 3, "should have 3 nested parameters");
    assert_eq!(params[0].code_point, codepoints::EXTNAM);
    assert_eq!(params[1].code_point, codepoints::SRVNAM);
    assert_eq!(params[2].code_point, codepoints::SRVRLSLV);
}

#[test]
fn test_ddm_roundtrip() {
    let mut builder = DdmBuilder::new(codepoints::ACCSEC);
    builder.add_u16(codepoints::SECMEC, codepoints::SECMEC_USRIDPWD);
    builder.add_code_point(
        codepoints::RDBNAM,
        &db2_proto::codepage::pad_rdbnam("TESTDB"),
    );
    let built = builder.build();

    let (ddm, _) = DdmObject::parse(&built).expect("roundtrip parse");
    assert_eq!(ddm.code_point, codepoints::ACCSEC);
    let params = ddm.parameters();
    assert_eq!(params.len(), 2);
    assert_eq!(params[0].code_point, codepoints::SECMEC);
    assert_eq!(params[1].code_point, codepoints::RDBNAM);
}

#[test]
fn test_ddm_excsat_build() {
    // Build a realistic EXCSAT command
    let mut builder = DdmBuilder::new(codepoints::EXCSAT);
    builder.add_string(codepoints::EXTNAM, "db2wire_test_client");
    builder.add_string(codepoints::SRVNAM, "localhost");
    builder.add_string(codepoints::SRVCLSNM, "DB2/LINUX");
    builder.add_string(codepoints::SRVRLSLV, "V12R1M000");

    // Build manager-level list
    let mgrlvl_data = build_mgrlvl_list(&[
        (codepoints::AGENT, 7),
        (codepoints::SQLAM, 7),
        (codepoints::RDB, 7),
        (codepoints::SECMGR, 7),
    ]);
    builder.add_code_point(codepoints::MGRLVLLS, &mgrlvl_data);

    let bytes = builder.build();
    let code_point = u16::from_be_bytes([bytes[2], bytes[3]]);
    assert_eq!(
        code_point,
        codepoints::EXCSAT,
        "code point should be EXCSAT"
    );

    // Verify we can parse it back
    let (ddm, _) = DdmObject::parse(&bytes).expect("parse EXCSAT");
    let params = ddm.parameters();
    assert!(params.len() >= 5, "EXCSAT should have at least 5 params");
}

#[test]
fn test_ddm_empty_data() {
    // A DDM with no nested parameters (just the 4-byte header)
    let builder = DdmBuilder::new(codepoints::RDBCMM);
    let bytes = builder.build();

    assert_eq!(
        bytes.len(),
        4,
        "empty DDM should be exactly 4 bytes (header only)"
    );
    let (ddm, _) = DdmObject::parse(&bytes).expect("parse empty DDM");
    assert_eq!(ddm.code_point, codepoints::RDBCMM);
    let params = ddm.parameters();
    assert_eq!(params.len(), 0, "empty DDM should have no params");
}

// ---------------------------------------------------------------------------
// Helper to build a manager-level list (pairs of u16 code-point + u16 level)
// ---------------------------------------------------------------------------

fn build_mgrlvl_list(levels: &[(u16, u16)]) -> Vec<u8> {
    let mut data = Vec::with_capacity(levels.len() * 4);
    for &(cp, lvl) in levels {
        data.extend_from_slice(&cp.to_be_bytes());
        data.extend_from_slice(&lvl.to_be_bytes());
    }
    data
}
