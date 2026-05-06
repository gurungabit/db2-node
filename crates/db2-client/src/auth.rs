use bytes::BytesMut;
use std::time::Instant;
use tracing::{debug, trace, warn};

use crate::config::{
    Config, CredentialEncoding, EncryptedPasswordEncoding, EncryptionAlgorithm, SecurityMechanism,
};
use crate::error::Error;
use crate::transport::Transport;
use db2_proto::codepoints;
use db2_proto::ddm::DdmObject;
use db2_proto::dss::{DssFrame, DssReader, DssWriter};

/// Information about the DB2 server, gathered during the authentication handshake.
#[derive(Debug, Clone, Default)]
pub struct ServerInfo {
    pub product_name: String,
    pub server_release: String,
    pub server_class: String,
    pub manager_levels: Vec<(u16, u16)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccsecRdbnamMode {
    Trimmed,
    LuwLegacy,
}

/// Perform the full DRDA authentication handshake.
///
/// The flow consists of two exchanges:
/// 1. EXCSAT (chained) + ACCSEC -> EXSATRD + ACCSECRD
/// 2. SECCHK (chained) + ACCRDB -> SECCHKRM + ACCRDBRM
///
/// Returns the server info and the next correlation ID to use.
pub(crate) async fn authenticate(
    transport: &mut Transport,
    config: &Config,
    accsec_rdbnam_mode: AccsecRdbnamMode,
    mut diagnostics: Option<&mut Vec<String>>,
) -> Result<(ServerInfo, u16), Error> {
    debug!("Starting DRDA authentication handshake");

    // Phase 1: EXCSAT + ACCSEC — negotiate the requested security mechanism.
    let luw_legacy = accsec_rdbnam_mode == AccsecRdbnamMode::LuwLegacy;
    let requested_encryption_algorithm = if luw_legacy {
        db2_proto::secmec9::EncryptionAlgorithm::Des
    } else {
        proto_encryption_algorithm(config.encryption_algorithm)
    };
    let requested_secmec = security_mechanism_code(config.security_mechanism);
    let excsat_security_manager_level = if luw_legacy {
        7
    } else {
        match config.security_mechanism {
            SecurityMechanism::UserPassword | SecurityMechanism::UserOnly => 7,
            _ => security_manager_level(config.encryption_algorithm),
        }
    };
    let excsat_data = db2_proto::commands::excsat::build_excsat_with_security_manager_level(
        excsat_security_manager_level,
    );
    if matches!(
        config.security_mechanism,
        SecurityMechanism::UserPassword | SecurityMechanism::UserOnly
    ) && !config.ssl
    {
        warn!(
            "DRDA security mechanism 0x{:04X} sends credentials without DRDA encryption; enable TLS for production use",
            requested_secmec
        );
    }

    // Generate a DH key pair for encrypted auth and for any server-requested
    // renegotiation back to encrypted credentials.
    let client_private =
        db2_proto::secmec9::generate_private_key_with_algorithm(requested_encryption_algorithm);
    let client_public = db2_proto::secmec9::calculate_public_key_with_algorithm(
        &client_private,
        requested_encryption_algorithm,
    );

    let accsec_data = build_accsec_for_mechanism(
        requested_secmec,
        &config.database,
        &client_public,
        requested_encryption_algorithm,
        accsec_rdbnam_mode,
    )?;

    let mut writer = DssWriter::new(1);
    writer.write_request(&excsat_data, true); // chained
    writer.set_correlation_id(2);
    writer.write_request(&accsec_data, false); // not chained

    let send_buf = writer.finish();
    let phase1_send_started = diagnostics.as_ref().map(|_| Instant::now());
    transport.write_bytes(&send_buf).await?;
    push_auth_elapsed(
        &mut diagnostics,
        "db2_connect_auth_phase1_send_ms",
        phase1_send_started,
    );
    debug!("Sent EXCSAT + ACCSEC(secmec=0x{requested_secmec:04X})");

    // Read phase 1 responses
    let mut recv_buf = BytesMut::with_capacity(4096);
    let phase1_read_started = diagnostics.as_ref().map(|_| Instant::now());
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
    push_auth_elapsed(
        &mut diagnostics,
        "db2_connect_auth_phase1_read_ms",
        phase1_read_started,
    );

    let phase1_parse_started = diagnostics.as_ref().map(|_| Instant::now());
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
    let credential_encoding = effective_credential_encoding(config, &server_info);
    let mut encrypted_password_encoding = effective_encrypted_password_encoding(
        config.encrypted_password_encoding,
        credential_encoding,
    );
    let mut encrypted_password_token_encoding = effective_encrypted_password_encoding(
        config.encrypted_password_token_encoding,
        credential_encoding,
    );
    // Parse ACCSECRD — get server's accepted mechanism and SECTKN
    let accsecrd_frame = &frames[1];
    let (accsecrd_obj, _) =
        DdmObject::parse(&accsecrd_frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;

    let (
        accepted_secmec,
        server_sectkn,
        accepted_encryption_algorithm_code,
        accepted_encryption_key_length,
    ) = match accsecrd_obj.code_point {
        codepoints::ACCSECRD => {
            let reply = db2_proto::replies::accsecrd::parse_accsecrd(&accsecrd_obj)
                .map_err(|e| Error::Protocol(e.to_string()))?;
            (
                reply.security_mechanism,
                reply.security_token,
                reply.encryption_algorithm,
                reply.encryption_key_length,
            )
        }
        codepoints::RDBNACRM | 0x221A | 0x2211 => {
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
    let negotiated_encryption_algorithm = negotiated_encryption_algorithm(
        accepted_encryption_algorithm_code,
        requested_encryption_algorithm,
    )?;
    (
        encrypted_password_encoding,
        encrypted_password_token_encoding,
    ) = jcc_compatible_encrypted_password_encodings(
        accepted_secmec,
        negotiated_encryption_algorithm,
        encrypted_password_encoding,
        encrypted_password_token_encoding,
    );
    let accsecrd_detail = format_reply_detail(&accsecrd_obj);
    let credential_options = AuthCredentialOptions {
        credential_encoding,
        encrypted_password_encoding,
        encrypted_password_token_encoding,
        encryption_algorithm: negotiated_encryption_algorithm,
    };

    debug!(
        "Phase 1 complete: server={}, secmec=0x{:04X}, sectkn={}, encalg={:?}, enckeylen={:?}",
        server_info.product_name,
        accepted_secmec,
        server_sectkn.as_ref().map_or(0, Vec::len),
        accepted_encryption_algorithm_code,
        accepted_encryption_key_length
    );
    push_auth_elapsed(
        &mut diagnostics,
        "db2_connect_auth_phase1_parse_ms",
        phase1_parse_started,
    );

    // Phase 2: SECCHK + ACCRDB. If the server negotiated a mechanism other
    // than the one we initially requested, send a matching ACCSEC first.
    let renegotiate_security = accepted_secmec != requested_secmec;
    let is_zos_server = !luw_legacy && server_info_is_zos(&server_info);
    let secchk_data = if luw_legacy {
        build_luw_legacy_secchk_for_mechanism(
            accepted_secmec,
            server_sectkn.as_deref(),
            &client_private,
            config,
            &accsecrd_detail,
        )?
    } else {
        build_secchk_for_mechanism(
            accepted_secmec,
            server_sectkn.as_deref(),
            &client_private,
            config,
            credential_options,
            &accsecrd_detail,
        )?
    };
    let accrdb_data = if luw_legacy {
        db2_proto::commands::accrdb::build_accrdb_luw(&config.database)
    } else if is_zos_server {
        let type_definition_name = match config.type_definition_name.as_deref() {
            Some("") => None,
            Some(value) => Some(value),
            None => Some(db2_proto::commands::accrdb::DEFAULT_TYPDEFNAM),
        };
        db2_proto::commands::accrdb::build_accrdb_with_optional_type_definition(
            &config.database,
            db2_proto::commands::accrdb::DEFAULT_PRDID,
            type_definition_name,
            db2_proto::commands::accrdb::DEFAULT_CCSID_SBC,
            db2_proto::commands::accrdb::DEFAULT_CCSID_DBC,
            db2_proto::commands::accrdb::DEFAULT_CCSID_MBC,
        )
    } else {
        db2_proto::commands::accrdb::build_accrdb_luw(&config.database)
    };

    let mut writer = DssWriter::new(1);
    if renegotiate_security {
        let accsec_data = build_accsec_for_mechanism(
            accepted_secmec,
            &config.database,
            &client_public,
            negotiated_encryption_algorithm,
            accsec_rdbnam_mode,
        )?;
        writer.write_request(&accsec_data, true); // chained
        writer.set_correlation_id(2);
    }
    writer.write_request(&secchk_data, true); // chained
    writer.next_correlation_id();
    writer.write_request(&accrdb_data, false); // not chained

    let send_buf = writer.finish();
    let phase2_send_started = diagnostics.as_ref().map(|_| Instant::now());
    transport.write_bytes(&send_buf).await?;
    push_auth_elapsed(
        &mut diagnostics,
        "db2_connect_auth_phase2_send_ms",
        phase2_send_started,
    );
    debug!(
        "Sent {}SECCHK + ACCRDB using {:?} credential encoding",
        if renegotiate_security {
            "ACCSEC + "
        } else {
            ""
        },
        credential_encoding
    );

    // Read phase 2 responses: optional ACCSECRD, SECCHKRM, ACCRDBRM/SQLCARD.
    let phase2_read_started = diagnostics.as_ref().map(|_| Instant::now());
    if recv_buf.len() < 6 {
        transport.read_at_least(&mut recv_buf, 6).await?;
    }

    let min_success_frames = if renegotiate_security { 3 } else { 2 };
    let frames = loop {
        let mut reader = DssReader::new(recv_buf.to_vec());
        let frames = reader
            .read_all_frames()
            .map_err(|e| Error::Protocol(e.to_string()))?;
        if phase2_frames_complete(&frames, min_success_frames)? {
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
    push_auth_elapsed(
        &mut diagnostics,
        "db2_connect_auth_phase2_read_ms",
        phase2_read_started,
    );

    // Parse phase 2 replies — DB2 may return only SECCHKRM on auth failure,
    // or close the socket immediately after sending the terminal error frame.
    let phase2_parse_started = diagnostics.as_ref().map(|_| Instant::now());
    let mut saw_secchkrm = false;
    let mut found_accrdbrm = false;
    let mut access_error: Option<Error> = None;
    let mut received_code_points = Vec::new();

    for frame in &frames {
        let (obj, _) =
            DdmObject::parse(&frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;
        received_code_points.push(obj.code_point);
        match obj.code_point {
            codepoints::ACCSECRD => {}
            codepoints::SECCHKRM => {
                trace!("Received SECCHKRM frame");
                let reply = db2_proto::replies::secchkrm::parse_secchkrm(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                saw_secchkrm = true;
                if !reply.is_success() {
                    let encoding_detail = format_auth_encoding_detail(
                        accepted_secmec,
                        credential_encoding,
                        encrypted_password_encoding,
                        encrypted_password_token_encoding,
                        negotiated_encryption_algorithm,
                        accepted_encryption_algorithm_code,
                        accepted_encryption_key_length,
                    );
                    return Err(Error::Auth(format!(
                        "Security check failed: severity={}, check_code={}, requested_secmec=0x{:04X}, accepted_secmec=0x{:04X}, {}",
                        reply.severity_code,
                        format_security_check_code(reply.security_check_code),
                        requested_secmec,
                        accepted_secmec,
                        encoding_detail
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
            codepoints::RDBNACRM | 0x2211 => {
                return Err(Error::Connection(
                    "RDB not accessed or database not found".into(),
                ));
            }
            codepoints::PRCCNVRM => {
                return Err(Error::Protocol(format!(
                    "Server returned DRDA protocol error PRCCNVRM during authentication; received {}",
                    format_code_points(&received_code_points)
                )));
            }
            codepoints::CMDNSPRM | codepoints::PRMNSPRM | codepoints::VALNSPRM => {
                let typdef_detail = if server_info_is_zos(&server_info) {
                    match config.type_definition_name.as_deref() {
                        Some("") => "<omitted>",
                        Some(value) => value,
                        None => db2_proto::commands::accrdb::DEFAULT_TYPDEFNAM,
                    }
                } else {
                    "QTDSQLX86"
                };
                return Err(Error::Protocol(format!(
                    "Server rejected an authentication parameter with {}: {}; accrdb_type_definition_name={}; received {}",
                    code_point_name(obj.code_point),
                    format_reply_detail(&obj),
                    typdef_detail,
                    format_code_points(&received_code_points)
                )));
            }
            codepoints::SYNTAXRM | codepoints::CMDCHKRM | codepoints::OBJNSPRM => {
                return Err(Error::Protocol(format!(
                    "Server rejected an authentication command with {}: {}; received {}",
                    code_point_name(obj.code_point),
                    format_reply_detail(&obj),
                    format_code_points(&received_code_points)
                )));
            }
            codepoints::SQLERRRM => {
                let reply = db2_proto::replies::sqlerrrm::parse_sqlerrrm(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                access_error = Some(Error::Protocol(format!(
                    "Server returned SQLERRRM during authentication: severity={}",
                    reply.severity_code
                )));
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
        return Err(Error::Protocol(format!(
            "No SECCHKRM received during authentication; received {}",
            format_code_points(&received_code_points)
        )));
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
    push_auth_elapsed(
        &mut diagnostics,
        "db2_connect_auth_phase2_parse_ms",
        phase2_parse_started,
    );

    // Reset correlation ID for SQL operations
    Ok((server_info, 1))
}

fn push_auth_elapsed(
    diagnostics: &mut Option<&mut Vec<String>>,
    name: &str,
    started: Option<Instant>,
) {
    if let (Some(diagnostics), Some(started)) = (diagnostics.as_deref_mut(), started) {
        diagnostics.push(format!(
            "{}={:.3}",
            name,
            started.elapsed().as_secs_f64() * 1000.0
        ));
    }
}

fn security_mechanism_code(security_mechanism: SecurityMechanism) -> u16 {
    match security_mechanism {
        SecurityMechanism::EncryptedUserPassword => codepoints::SECMEC_EUSRIDPWD,
        SecurityMechanism::EncryptedPassword => codepoints::SECMEC_USRENCPWD,
        SecurityMechanism::UserPassword => codepoints::SECMEC_USRIDPWD,
        SecurityMechanism::UserOnly => codepoints::SECMEC_USRIDONL,
    }
}

fn security_manager_level(encryption_algorithm: EncryptionAlgorithm) -> u16 {
    match encryption_algorithm {
        EncryptionAlgorithm::Des => 7,
        EncryptionAlgorithm::Aes => 9,
    }
}

fn proto_encryption_algorithm(
    encryption_algorithm: EncryptionAlgorithm,
) -> db2_proto::secmec9::EncryptionAlgorithm {
    match encryption_algorithm {
        EncryptionAlgorithm::Des => db2_proto::secmec9::EncryptionAlgorithm::Des,
        EncryptionAlgorithm::Aes => db2_proto::secmec9::EncryptionAlgorithm::Aes,
    }
}

fn negotiated_encryption_algorithm(
    accepted_encryption_algorithm_code: Option<u16>,
    requested_encryption_algorithm: db2_proto::secmec9::EncryptionAlgorithm,
) -> Result<db2_proto::secmec9::EncryptionAlgorithm, Error> {
    match accepted_encryption_algorithm_code {
        Some(codepoints::ENCALG_DES) => Ok(db2_proto::secmec9::EncryptionAlgorithm::Des),
        Some(codepoints::ENCALG_AES) => Ok(db2_proto::secmec9::EncryptionAlgorithm::Aes),
        Some(other) => Err(Error::Protocol(format!(
            "Server selected unsupported DRDA encryption algorithm 0x{other:04X}"
        ))),
        None => Ok(requested_encryption_algorithm),
    }
}

fn server_info_is_zos(server_info: &ServerInfo) -> bool {
    let release = server_info.server_release.trim().to_ascii_uppercase();
    let class = server_info.server_class.trim().to_ascii_uppercase();
    release.starts_with("DSN") || class.contains("Z/OS") || class.contains("MVS")
}

#[derive(Debug, Clone, Copy)]
struct AuthCredentialOptions {
    credential_encoding: db2_proto::commands::secchk::CredentialEncoding,
    encrypted_password_encoding: db2_proto::commands::secchk::CredentialEncoding,
    encrypted_password_token_encoding: db2_proto::commands::secchk::CredentialEncoding,
    encryption_algorithm: db2_proto::secmec9::EncryptionAlgorithm,
}

fn build_accsec_for_mechanism(
    security_mechanism: u16,
    rdbnam: &str,
    client_public: &[u8],
    encryption_algorithm: db2_proto::secmec9::EncryptionAlgorithm,
    rdbnam_mode: AccsecRdbnamMode,
) -> Result<Vec<u8>, Error> {
    match security_mechanism {
        codepoints::SECMEC_EUSRIDPWD
        | codepoints::SECMEC_USRENCPWD
        | codepoints::SECMEC_USRIDPWD
        | codepoints::SECMEC_USRIDONL => {
            let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::ACCSEC);
            ddm.add_u16(codepoints::SECMEC, security_mechanism);
            let rdbnam = match rdbnam_mode {
                AccsecRdbnamMode::Trimmed => db2_proto::codepage::pad_rdbnam(rdbnam),
                AccsecRdbnamMode::LuwLegacy => db2_proto::codepage::pad_ebcdic(rdbnam, 18),
            };
            ddm.add_code_point(codepoints::RDBNAM, &rdbnam);
            if matches!(
                security_mechanism,
                codepoints::SECMEC_EUSRIDPWD | codepoints::SECMEC_USRENCPWD
            ) {
                ddm.add_code_point(codepoints::SECTKN, client_public);
                if encryption_algorithm == db2_proto::secmec9::EncryptionAlgorithm::Aes {
                    ddm.add_u16(codepoints::ENCALG, codepoints::ENCALG_AES);
                    ddm.add_u16(codepoints::ENCKEYLEN, codepoints::ENCKEYLEN_AES_256);
                }
            }
            Ok(ddm.build())
        }
        other => Err(Error::Auth(format!(
            "Unsupported DRDA security mechanism 0x{other:04X}"
        ))),
    }
}

fn build_secchk_for_mechanism(
    security_mechanism: u16,
    server_sectkn: Option<&[u8]>,
    client_private: &[u8],
    config: &Config,
    credential_options: AuthCredentialOptions,
    accsecrd_detail: &str,
) -> Result<Vec<u8>, Error> {
    match security_mechanism {
        codepoints::SECMEC_EUSRIDPWD => {
            let server_sectkn = server_sectkn.ok_or_else(|| {
                Error::Protocol(format!(
                    "ACCSECRD selected encrypted authentication but did not include SECTKN; {accsecrd_detail}"
                ))
            })?;
            db2_proto::commands::secchk::build_secchk_eusridpwd_with_algorithm_and_encoding(
                &config.database,
                &config.user,
                &config.password,
                server_sectkn,
                client_private,
                credential_options.credential_encoding,
                credential_options.encryption_algorithm,
            )
            .map_err(Error::from)
        }
        codepoints::SECMEC_USRENCPWD => {
            let server_sectkn = server_sectkn.ok_or_else(|| {
                Error::Protocol(format!(
                    "ACCSECRD selected encrypted password authentication but did not include SECTKN; {accsecrd_detail}"
                ))
            })?;
            db2_proto::commands::secchk::build_secchk_usencpwd_with_algorithm_and_encodings(
                &config.database,
                &config.user,
                &config.password,
                server_sectkn,
                client_private,
                db2_proto::commands::secchk::EncryptedPasswordCredentialEncodings {
                    user_id: credential_options.credential_encoding,
                    password: credential_options.encrypted_password_encoding,
                    password_token: credential_options.encrypted_password_token_encoding,
                },
                credential_options.encryption_algorithm,
            )
            .map_err(Error::from)
        }
        codepoints::SECMEC_USRIDPWD => Ok(
            db2_proto::commands::secchk::build_secchk_usridpwd_with_encoding(
                &config.database,
                &config.user,
                &config.password,
                credential_options.credential_encoding,
            ),
        ),
        codepoints::SECMEC_USRIDONL => {
            let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::SECCHK);
            ddm.add_u16(codepoints::SECMEC, codepoints::SECMEC_USRIDONL);
            let user_id = encode_credential(&config.user, credential_options.credential_encoding);
            ddm.add_code_point(codepoints::USRID, &user_id);
            Ok(ddm.build())
        }
        other => Err(Error::Auth(format!(
            "Server selected unsupported DRDA security mechanism 0x{other:04X}"
        ))),
    }
}

fn build_luw_legacy_secchk_for_mechanism(
    security_mechanism: u16,
    server_sectkn: Option<&[u8]>,
    client_private: &[u8],
    config: &Config,
    accsecrd_detail: &str,
) -> Result<Vec<u8>, Error> {
    match security_mechanism {
        codepoints::SECMEC_EUSRIDPWD => {
            let server_sectkn = server_sectkn.ok_or_else(|| {
                Error::Protocol(format!(
                    "ACCSECRD selected encrypted authentication but did not include SECTKN; {accsecrd_detail}"
                ))
            })?;
            db2_proto::commands::secchk::build_secchk_eusridpwd(
                &config.database,
                &config.user,
                &config.password,
                server_sectkn,
                client_private,
            )
            .map_err(Error::from)
        }
        codepoints::SECMEC_USRENCPWD => {
            let server_sectkn = server_sectkn.ok_or_else(|| {
                Error::Protocol(format!(
                    "ACCSECRD selected encrypted password authentication but did not include SECTKN; {accsecrd_detail}"
                ))
            })?;
            db2_proto::commands::secchk::build_secchk_usencpwd(
                &config.database,
                &config.user,
                &config.password,
                server_sectkn,
                client_private,
            )
            .map_err(Error::from)
        }
        codepoints::SECMEC_USRIDPWD => Ok(db2_proto::commands::secchk::build_secchk_usridpwd(
            &config.database,
            &config.user,
            &config.password,
        )),
        codepoints::SECMEC_USRIDONL => {
            let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::SECCHK);
            ddm.add_u16(codepoints::SECMEC, codepoints::SECMEC_USRIDONL);
            ddm.add_code_point(
                codepoints::USRID,
                &db2_proto::codepage::utf8_to_ebcdic037(&config.user),
            );
            Ok(ddm.build())
        }
        other => Err(Error::Auth(format!(
            "Unsupported DRDA security mechanism 0x{other:04X}"
        ))),
    }
}

fn effective_encrypted_password_encoding(
    config_value: EncryptedPasswordEncoding,
    credential_encoding: db2_proto::commands::secchk::CredentialEncoding,
) -> db2_proto::commands::secchk::CredentialEncoding {
    match config_value {
        EncryptedPasswordEncoding::SameAsCredential => credential_encoding,
        EncryptedPasswordEncoding::Ebcdic037 => {
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037
        }
        EncryptedPasswordEncoding::Utf8 => db2_proto::commands::secchk::CredentialEncoding::Utf8,
    }
}

fn jcc_compatible_encrypted_password_encodings(
    accepted_secmec: u16,
    encryption_algorithm: db2_proto::secmec9::EncryptionAlgorithm,
    password_encoding: db2_proto::commands::secchk::CredentialEncoding,
    token_encoding: db2_proto::commands::secchk::CredentialEncoding,
) -> (
    db2_proto::commands::secchk::CredentialEncoding,
    db2_proto::commands::secchk::CredentialEncoding,
) {
    if accepted_secmec == codepoints::SECMEC_USRENCPWD
        && encryption_algorithm == db2_proto::secmec9::EncryptionAlgorithm::Aes
    {
        // IBM JCC uses the source CCSID manager (CCSID 1208) for the password
        // plaintext in SECMEC 7 AES. The user-id-derived token is DES-only.
        (
            db2_proto::commands::secchk::CredentialEncoding::Utf8,
            db2_proto::commands::secchk::CredentialEncoding::Utf8,
        )
    } else {
        (password_encoding, token_encoding)
    }
}

fn effective_credential_encoding(
    config: &Config,
    server_info: &ServerInfo,
) -> db2_proto::commands::secchk::CredentialEncoding {
    match config.credential_encoding {
        CredentialEncoding::Ebcdic037 => db2_proto::commands::secchk::CredentialEncoding::Ebcdic037,
        CredentialEncoding::Utf8 => db2_proto::commands::secchk::CredentialEncoding::Utf8,
        CredentialEncoding::Auto => {
            if server_supports_utf8_credentials(server_info) {
                db2_proto::commands::secchk::CredentialEncoding::Utf8
            } else {
                db2_proto::commands::secchk::CredentialEncoding::Ebcdic037
            }
        }
    }
}

fn format_auth_encoding_detail(
    accepted_secmec: u16,
    credential_encoding: db2_proto::commands::secchk::CredentialEncoding,
    encrypted_password_encoding: db2_proto::commands::secchk::CredentialEncoding,
    encrypted_password_token_encoding: db2_proto::commands::secchk::CredentialEncoding,
    encryption_algorithm: db2_proto::secmec9::EncryptionAlgorithm,
    accepted_encryption_algorithm_code: Option<u16>,
    accepted_encryption_key_length: Option<u16>,
) -> String {
    let encryption_detail = format!(
        "encryption_algorithm={encryption_algorithm:?}, accepted_encalg={}, accepted_enckeylen={}",
        format_optional_u16(accepted_encryption_algorithm_code),
        format_optional_u16(accepted_encryption_key_length)
    );
    if accepted_secmec == codepoints::SECMEC_USRENCPWD {
        format!(
            "credential_encoding={credential_encoding:?}, encrypted_password_encoding={encrypted_password_encoding:?}, encrypted_password_token_encoding={encrypted_password_token_encoding:?}, {encryption_detail}"
        )
    } else {
        format!("credential_encoding={credential_encoding:?}, {encryption_detail}")
    }
}

fn format_optional_u16(value: Option<u16>) -> String {
    value
        .map(|value| format!("0x{value:04X}"))
        .unwrap_or_else(|| "none".into())
}

fn server_supports_utf8_credentials(server_info: &ServerInfo) -> bool {
    server_info
        .manager_levels
        .iter()
        .any(|(code_point, level)| *code_point == codepoints::UNICODEMGR && *level == 1208)
}

fn encode_credential(
    value: &str,
    credential_encoding: db2_proto::commands::secchk::CredentialEncoding,
) -> Vec<u8> {
    match credential_encoding {
        db2_proto::commands::secchk::CredentialEncoding::Ebcdic037 => {
            db2_proto::codepage::utf8_to_ebcdic037(value)
        }
        db2_proto::commands::secchk::CredentialEncoding::Utf8 => value.as_bytes().to_vec(),
    }
}

fn phase2_frames_complete(frames: &[DssFrame], min_success_frames: usize) -> Result<bool, Error> {
    if frames.len() >= min_success_frames {
        return Ok(true);
    }

    let mut saw_secchkrm_success = false;
    let mut saw_access_reply = false;

    for frame in frames {
        let (obj, _) =
            DdmObject::parse(&frame.payload).map_err(|e| Error::Protocol(e.to_string()))?;
        match obj.code_point {
            codepoints::SECCHKRM => {
                let reply = db2_proto::replies::secchkrm::parse_secchkrm(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if !reply.is_success() {
                    return Ok(true);
                }
                saw_secchkrm_success = true;
            }
            codepoints::ACCRDBRM => saw_access_reply = true,
            codepoints::RDBNACRM
            | 0x2211
            | codepoints::PRCCNVRM
            | codepoints::SYNTAXRM
            | codepoints::CMDNSPRM
            | codepoints::PRMNSPRM
            | codepoints::VALNSPRM
            | codepoints::CMDCHKRM
            | codepoints::OBJNSPRM
            | codepoints::SQLERRRM => return Ok(true),
            codepoints::SQLCARD => {
                let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if card.is_error() {
                    return Ok(true);
                }
                if card.is_success() {
                    saw_access_reply = true;
                }
            }
            _ => {}
        }
    }

    Ok(saw_secchkrm_success && saw_access_reply)
}

fn format_security_check_code(code: Option<u8>) -> String {
    match code {
        Some(0x00) => "0x00 (success)".into(),
        Some(0x01) => "0x01 (security mechanism not supported)".into(),
        Some(0x0A) => "0x0A (security service non-retryable error)".into(),
        Some(0x0B) => "0x0B (security token missing or invalid)".into(),
        Some(0x0E) => "0x0E (password expired)".into(),
        Some(0x0F) => "0x0F (user id or password invalid)".into(),
        Some(0x10) => "0x10 (password missing)".into(),
        Some(0x12) => "0x12 (user id missing)".into(),
        Some(0x13) => "0x13 (user id or password invalid)".into(),
        Some(0x14) => "0x14 (user id revoked)".into(),
        Some(0x15) => "0x15 (new password invalid)".into(),
        Some(other) => format!("0x{other:02X}"),
        None => "unknown".into(),
    }
}

fn format_code_points(code_points: &[u16]) -> String {
    if code_points.is_empty() {
        return "none".into();
    }

    code_points
        .iter()
        .map(|cp| format!("{}(0x{cp:04X})", code_point_name(*cp)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn format_reply_detail(obj: &DdmObject) -> String {
    let mut details = Vec::new();

    if let Some(severity) = obj.find_param(codepoints::SVRCOD).and_then(|p| p.as_u16()) {
        details.push(format!("severity={severity}"));
    }

    if let Some(code_point) = obj.find_param(codepoints::CODPNT).and_then(|p| p.as_u16()) {
        details.push(format!(
            "parameter={}(0x{code_point:04X})",
            code_point_name(code_point)
        ));
    }

    if details.is_empty() {
        format!("data_len={}", obj.data.len())
    } else {
        details.join(", ")
    }
}

fn code_point_name(code_point: u16) -> &'static str {
    match code_point {
        codepoints::EXCSAT => "EXCSAT",
        codepoints::EXSATRD => "EXSATRD",
        codepoints::ACCSEC => "ACCSEC",
        codepoints::ACCSECRD => "ACCSECRD",
        codepoints::SECCHK => "SECCHK",
        codepoints::SECCHKRM => "SECCHKRM",
        codepoints::ACCRDB => "ACCRDB",
        codepoints::ACCRDBRM => "ACCRDBRM",
        codepoints::CODPNT => "CODPNT",
        codepoints::SVRCOD => "SVRCOD",
        codepoints::SECMEC => "SECMEC",
        codepoints::SECTKN => "SECTKN",
        codepoints::ENCALG => "ENCALG",
        codepoints::ENCKEYLEN => "ENCKEYLEN",
        codepoints::USRID => "USRID",
        codepoints::PASSWORD => "PASSWORD",
        codepoints::RDBNAM => "RDBNAM",
        codepoints::PRDID => "PRDID",
        codepoints::TYPDEFNAM => "TYPDEFNAM",
        codepoints::TYPDEFOVR => "TYPDEFOVR",
        codepoints::CCSIDSBC => "CCSIDSBC",
        codepoints::CCSIDDBC => "CCSIDDBC",
        codepoints::CCSIDMBC => "CCSIDMBC",
        codepoints::RDBACCCL => "RDBACCCL",
        codepoints::CRRTKN => "CRRTKN",
        codepoints::SQLCARD => "SQLCARD",
        codepoints::RDBNACRM => "RDBNACRM",
        codepoints::PRCCNVRM => "PRCCNVRM",
        codepoints::SYNTAXRM => "SYNTAXRM",
        codepoints::CMDNSPRM => "CMDNSPRM",
        codepoints::PRMNSPRM => "PRMNSPRM",
        codepoints::VALNSPRM => "VALNSPRM",
        codepoints::CMDCHKRM => "CMDCHKRM",
        codepoints::OBJNSPRM => "OBJNSPRM",
        codepoints::SQLERRRM => "SQLERRRM",
        _ => "UNKNOWN",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db2_proto::dss::{DssFlags, DssHeader, DssType};

    fn reply_frame(payload: Vec<u8>) -> DssFrame {
        DssFrame {
            header: DssHeader {
                length: (payload.len() + 6) as u16,
                dss_type: DssType::Reply,
                flags: DssFlags::none(),
                correlation_id: 1,
            },
            payload,
        }
    }

    #[test]
    fn valnsprm_ends_phase2_read() {
        let mut builder = db2_proto::ddm::DdmBuilder::new(codepoints::VALNSPRM);
        builder.add_u16(codepoints::SVRCOD, codepoints::SRVCOD_ERROR);
        builder.add_u16(codepoints::CODPNT, codepoints::SECMEC);

        let frames = vec![reply_frame(builder.build())];

        assert!(phase2_frames_complete(&frames, 2).unwrap());
    }

    #[test]
    fn reply_detail_includes_rejected_parameter() {
        let mut builder = db2_proto::ddm::DdmBuilder::new(codepoints::VALNSPRM);
        builder.add_u16(codepoints::SVRCOD, codepoints::SRVCOD_ERROR);
        builder.add_u16(codepoints::CODPNT, codepoints::SECTKN);
        let bytes = builder.build();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();

        assert_eq!(
            format_reply_detail(&obj),
            "severity=8, parameter=SECTKN(0x11DC)"
        );
    }

    #[test]
    fn security_check_code_names_include_zos_auth_failures() {
        assert_eq!(
            format_security_check_code(Some(0x13)),
            "0x13 (user id or password invalid)"
        );
        assert_eq!(
            format_security_check_code(Some(0x14)),
            "0x14 (user id revoked)"
        );
        assert_eq!(
            format_security_check_code(Some(0x0B)),
            "0x0B (security token missing or invalid)"
        );
    }

    #[test]
    fn accsec_includes_sectkn_only_for_encrypted_auth() {
        let public_key = [0xAA; 32];
        let encrypted = build_accsec_for_mechanism(
            codepoints::SECMEC_EUSRIDPWD,
            "DSNDB04",
            &public_key,
            db2_proto::secmec9::EncryptionAlgorithm::Des,
            AccsecRdbnamMode::Trimmed,
        )
        .unwrap();
        let (encrypted_obj, _) = DdmObject::parse(&encrypted).unwrap();
        assert!(encrypted_obj.find_param(codepoints::SECTKN).is_some());

        let encrypted_password = build_accsec_for_mechanism(
            codepoints::SECMEC_USRENCPWD,
            "DSNDB04",
            &public_key,
            db2_proto::secmec9::EncryptionAlgorithm::Des,
            AccsecRdbnamMode::Trimmed,
        )
        .unwrap();
        let (encrypted_password_obj, _) = DdmObject::parse(&encrypted_password).unwrap();
        assert!(encrypted_password_obj
            .find_param(codepoints::SECTKN)
            .is_some());

        let clear = build_accsec_for_mechanism(
            codepoints::SECMEC_USRIDPWD,
            "DSNDB04",
            &public_key,
            db2_proto::secmec9::EncryptionAlgorithm::Des,
            AccsecRdbnamMode::Trimmed,
        )
        .unwrap();
        let (clear_obj, _) = DdmObject::parse(&clear).unwrap();
        assert!(clear_obj.find_param(codepoints::SECTKN).is_none());
    }

    #[test]
    fn accsec_includes_aes_encryption_parameters_when_requested() {
        let public_key = [0xAA; 32];
        let encrypted_password = build_accsec_for_mechanism(
            codepoints::SECMEC_USRENCPWD,
            "DSNDB04",
            &public_key,
            db2_proto::secmec9::EncryptionAlgorithm::Aes,
            AccsecRdbnamMode::Trimmed,
        )
        .unwrap();
        let (obj, _) = DdmObject::parse(&encrypted_password).unwrap();
        assert_eq!(
            obj.find_param(codepoints::ENCALG)
                .and_then(|param| param.as_u16()),
            Some(codepoints::ENCALG_AES)
        );
        assert_eq!(
            obj.find_param(codepoints::ENCKEYLEN)
                .and_then(|param| param.as_u16()),
            Some(codepoints::ENCKEYLEN_AES_256)
        );
    }

    #[test]
    fn auto_credential_encoding_uses_utf8_when_unicode_manager_is_negotiated() {
        let config = Config::default();
        let mut server_info = ServerInfo::default();
        server_info
            .manager_levels
            .push((codepoints::UNICODEMGR, 1208));

        assert_eq!(
            effective_credential_encoding(&config, &server_info),
            db2_proto::commands::secchk::CredentialEncoding::Utf8
        );
    }

    #[test]
    fn explicit_credential_encoding_overrides_server_managers() {
        let config = Config {
            credential_encoding: CredentialEncoding::Ebcdic037,
            ..Default::default()
        };
        let mut server_info = ServerInfo::default();
        server_info
            .manager_levels
            .push((codepoints::UNICODEMGR, 1208));

        assert_eq!(
            effective_credential_encoding(&config, &server_info),
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037
        );
    }

    #[test]
    fn secmec7_aes_forces_jcc_source_ccsid_password_encoding() {
        let (password_encoding, token_encoding) = jcc_compatible_encrypted_password_encodings(
            codepoints::SECMEC_USRENCPWD,
            db2_proto::secmec9::EncryptionAlgorithm::Aes,
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037,
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037,
        );

        assert_eq!(
            password_encoding,
            db2_proto::commands::secchk::CredentialEncoding::Utf8
        );
        assert_eq!(
            token_encoding,
            db2_proto::commands::secchk::CredentialEncoding::Utf8
        );

        let (password_encoding, token_encoding) = jcc_compatible_encrypted_password_encodings(
            codepoints::SECMEC_USRENCPWD,
            db2_proto::secmec9::EncryptionAlgorithm::Des,
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037,
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037,
        );

        assert_eq!(
            password_encoding,
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037
        );
        assert_eq!(
            token_encoding,
            db2_proto::commands::secchk::CredentialEncoding::Ebcdic037
        );
    }
}
