use bytes::BytesMut;
use tracing::{debug, trace};

use crate::config::Config;
use crate::error::Error;
use crate::transport::Transport;
use db2_proto::codepoints;
use db2_proto::ddm::DdmObject;
use db2_proto::dss::{DssReader, DssWriter};

/// Information about the DB2 server, gathered during the authentication handshake.
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub product_name: String,
    pub server_release: String,
    pub server_class: String,
    pub manager_levels: Vec<(u16, u16)>,
}

/// Perform the full DRDA authentication handshake.
///
/// The flow consists of two exchanges:
/// 1. EXCSAT (chained) + ACCSEC -> EXSATRD + ACCSECRD
/// 2. SECCHK (chained) + ACCRDB -> SECCHKRM + ACCRDBRM
///
/// Returns the server info and the next correlation ID to use.
pub async fn authenticate(
    transport: &mut Transport,
    config: &Config,
) -> Result<(ServerInfo, u16), Error> {
    debug!("Starting DRDA authentication handshake");

    // Phase 1: EXCSAT + ACCSEC(encrypted) — negotiate security with DH key exchange
    let excsat_data = db2_proto::commands::excsat::build_excsat_default();

    // Generate DH key pair for encrypted auth
    let client_private = db2_proto::secmec9::generate_private_key();
    let client_public = db2_proto::secmec9::calculate_public_key(&client_private);

    // Build ACCSEC with encrypted mechanism (0x0009) and our public key as SECTKN
    let accsec_data = {
        let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::ACCSEC);
        ddm.add_u16(codepoints::SECMEC, codepoints::SECMEC_EUSRIDPWD);
        ddm.add_code_point(
            codepoints::RDBNAM,
            &db2_proto::codepage::pad_rdbnam(&config.database),
        );
        ddm.add_code_point(codepoints::SECTKN, &client_public);
        ddm.build()
    };

    let mut writer = DssWriter::new(1);
    writer.write_request(&excsat_data, true); // chained
    writer.set_correlation_id(2);
    writer.write_request(&accsec_data, false); // not chained

    let send_buf = writer.finish();
    transport.write_bytes(&send_buf).await?;
    debug!("Sent EXCSAT + ACCSEC(encrypted)");

    // Read phase 1 responses
    let mut recv_buf = BytesMut::with_capacity(4096);
    transport.read_at_least(&mut recv_buf, 6).await?;

    let frames = loop {
        let mut reader = DssReader::new(recv_buf.to_vec());
        let frames = reader
            .read_all_frames()
            .map_err(|e| Error::Protocol(e.to_string()))?;
        if frames.len() >= 2 {
            let remaining = reader.into_remaining();
            recv_buf = BytesMut::from(remaining.as_slice());
            break frames;
        }
        transport.read_bytes(&mut recv_buf).await?;
    };

    let mut server_info = ServerInfo::default();

    // Parse EXSATRD
    let exsatrd_frame = &frames[0];
    let (exsatrd_obj, _) =
        DdmObject::parse(&exsatrd_frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;

    if exsatrd_obj.code_point == codepoints::EXSATRD {
        let attrs = db2_proto::replies::exsatrd::parse_exsatrd(&exsatrd_obj)
            .map_err(|e| Error::Protocol(e.to_string()))?;
        server_info.product_name = attrs.server_name.unwrap_or_default();
        server_info.server_release = attrs.product_release_level.unwrap_or_default();
        server_info.server_class = attrs.server_class_name.unwrap_or_default();
        server_info.manager_levels = attrs.manager_levels;
    } else {
        return Err(Error::Protocol(format!(
            "Expected EXSATRD, got 0x{:04X}",
            exsatrd_obj.code_point
        )));
    }

    // Parse ACCSECRD — get server's accepted mechanism and SECTKN
    let accsecrd_frame = &frames[1];
    let (accsecrd_obj, _) =
        DdmObject::parse(&accsecrd_frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;

    let accepted_secmec = match accsecrd_obj.code_point {
        codepoints::ACCSECRD => {
            let reply = db2_proto::replies::accsecrd::parse_accsecrd(&accsecrd_obj)
                .map_err(|e| Error::Protocol(e.to_string()))?;
            reply.security_mechanism
        }
        codepoints::RDBNACRM | 0x221A => {
            // Some DB2 LUW servers reject an unknown RDB name during ACCSEC
            // instead of waiting until the later ACCRDB phase.
            return Err(Error::Connection(
                "RDB not accessed or database not found".into(),
            ));
        }
        other => {
            return Err(Error::Protocol(format!(
                "Expected ACCSECRD, got 0x{:04X}",
                other
            )));
        }
    };

    debug!(
        "Phase 1 complete: server={}, secmec=0x{:04X}",
        server_info.product_name, accepted_secmec
    );

    // Phase 2: ACCSEC(cleartext) + SECCHK + ACCRDB (matching pydrda flow)
    let accsec_cleartext = db2_proto::commands::accsec::build_accsec_usridpwd(&config.database);
    let secchk_data = db2_proto::commands::secchk::build_secchk_usridpwd(
        &config.database,
        &config.user,
        &config.password,
    );
    let accrdb_data = db2_proto::commands::accrdb::build_accrdb_default(&config.database);

    let mut writer = DssWriter::new(1);
    writer.write_request(&accsec_cleartext, true); // chained
    writer.set_correlation_id(2);
    writer.write_request(&secchk_data, true); // chained
    writer.set_correlation_id(3);
    writer.write_request(&accrdb_data, false); // not chained

    let send_buf = writer.finish();
    transport.write_bytes(&send_buf).await?;
    debug!("Sent ACCSEC(cleartext) + SECCHK + ACCRDB");

    // Read phase 2 responses (ACCSECRD + SECCHKRM + ACCRDBRM/SQLCARD)
    if recv_buf.len() < 6 {
        transport.read_at_least(&mut recv_buf, 6).await?;
    }

    let frames = loop {
        let mut reader = DssReader::new(recv_buf.to_vec());
        let frames = reader
            .read_all_frames()
            .map_err(|e| Error::Protocol(e.to_string()))?;
        if frames.len() >= 3 {
            let _ = reader.into_remaining();
            break frames;
        }

        match transport.read_bytes(&mut recv_buf).await {
            Ok(_) => {}
            Err(Error::Connection(msg))
                if msg.to_lowercase().contains("closed by server") && !frames.is_empty() =>
            {
                break frames;
            }
            Err(Error::Connection(msg)) if msg.to_lowercase().contains("closed by server") => {
                return Err(Error::Connection(
                    "RDB not accessed or database not found".into(),
                ));
            }
            Err(err) => return Err(err),
        }
    };

    // Parse phase 2 replies — DB2 may return only SECCHKRM on auth failure,
    // or close the socket immediately after sending the terminal error frame.
    let mut saw_secchkrm = false;
    let mut found_accrdbrm = false;
    let mut access_error: Option<Error> = None;

    for frame in &frames {
        let (obj, _) =
            DdmObject::parse(&frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;
        match obj.code_point {
            codepoints::ACCSECRD => {}
            codepoints::SECCHKRM => {
                trace!("Received SECCHKRM frame");
                let reply = db2_proto::replies::secchkrm::parse_secchkrm(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                saw_secchkrm = true;
                if !reply.is_success() {
                    return Err(Error::Auth(format!(
                        "Security check failed: severity={}, check_code={}",
                        reply.severity_code,
                        format_security_check_code(reply.security_check_code)
                    )));
                }
                debug!("Security check passed");
            }
            codepoints::ACCRDBRM => {
                let reply = db2_proto::replies::accrdbrm::parse_accrdbrm(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if !reply.is_success() {
                    access_error = Some(Error::Connection(format!(
                        "Database access failed: severity={}",
                        reply.severity_code
                    )));
                }
                found_accrdbrm = true;
                debug!("Received ACCRDBRM, success={}", reply.is_success());
            }
            codepoints::SQLCARD => {
                let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if card.is_error() {
                    access_error = Some(Error::Connection(format!(
                        "Database access failed: SQLCODE={}, SQLSTATE={}, {}",
                        card.sqlcode, card.sqlstate, card.sqlerrmc
                    )));
                } else if !found_accrdbrm && card.is_success() {
                    // Some DB2 servers send SQLCARD with sqlcode=0 as a success indicator
                    found_accrdbrm = true;
                }
            }
            codepoints::RDBNACRM => {
                return Err(Error::Connection(
                    "RDB not accessed or database not found".into(),
                ));
            }
            other => {
                debug!(
                    "Received unexpected code point 0x{:04X} during ACCRDB",
                    other
                );
            }
        }
    }

    if !saw_secchkrm {
        return Err(Error::Protocol(
            "No SECCHKRM received during authentication".into(),
        ));
    }
    if let Some(err) = access_error {
        return Err(err);
    }
    if !found_accrdbrm {
        return Err(Error::Protocol(
            "No ACCRDBRM or success SQLCARD received during database access".into(),
        ));
    }
    debug!("Database access granted");

    // Post-auth: connection is established

    debug!("Authentication handshake complete");

    // Reset correlation ID for SQL operations
    Ok((server_info, 1))
}

fn format_security_check_code(code: Option<u8>) -> String {
    match code {
        Some(0x0F) => "0x0F (invalid password)".into(),
        Some(0x10) => "0x10 (missing or invalid user id)".into(),
        Some(0x14) => "0x14 (security mechanism not supported)".into(),
        Some(other) => format!("0x{other:02X}"),
        None => "unknown".into(),
    }
}
