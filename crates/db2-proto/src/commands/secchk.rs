//! Build SECCHK (Security Check) command.
use crate::codepage::{pad_rdbnam, utf8_to_ebcdic037, utf8_to_ebcdic500, utf8_to_ebcdic500_luw};
use crate::codepoints::*;
use crate::ddm::DdmBuilder;
use crate::secmec9::EncryptionAlgorithm;
use crate::{ProtoError, Result};

/// Encoding for credential string bytes sent in SECCHK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialEncoding {
    /// EBCDIC code page 500, DRDA's default character parameter CCSID.
    Ebcdic500,
    /// Db2 LUW's ASCII-compatible CCSID 500 credential conversion.
    Ebcdic500Luw,
    /// EBCDIC code page 037.
    Ebcdic037,
    /// UTF-8.
    Utf8,
}

impl CredentialEncoding {
    /// Encode credential plaintext before framing or DRDA encryption.
    pub fn encode(self, value: &str) -> Vec<u8> {
        match self {
            CredentialEncoding::Ebcdic500 => utf8_to_ebcdic500(value),
            CredentialEncoding::Ebcdic500Luw => utf8_to_ebcdic500_luw(value),
            CredentialEncoding::Ebcdic037 => utf8_to_ebcdic037(value),
            CredentialEncoding::Utf8 => value.as_bytes().to_vec(),
        }
    }
}

/// Encodings used by SECMEC 7 encrypted password authentication.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncryptedPasswordCredentialEncodings {
    /// Encoding for the clear USRID parameter.
    pub user_id: CredentialEncoding,
    /// Encoding for the password plaintext before encryption.
    pub password: CredentialEncoding,
    /// Encoding for the user-ID-derived password IV/token.
    pub password_token: CredentialEncoding,
}

impl EncryptedPasswordCredentialEncodings {
    pub fn same(credential_encoding: CredentialEncoding) -> Self {
        Self {
            user_id: credential_encoding,
            password: credential_encoding,
            password_token: credential_encoding,
        }
    }
}

/// Build a SECCHK DDM command with user ID, password, and database name.
///
/// Parameters:
///   - security_mechanism: Security mechanism code
///   - rdbnam: Database name (included for DB2 LUW compatibility)
///   - user_id: User ID (will be EBCDIC-encoded)
///   - password: Password (will be EBCDIC-encoded)
pub fn build_secchk(
    security_mechanism: u16,
    rdbnam: &str,
    user_id: &str,
    password: &str,
) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(SECCHK);
    ddm.add_u16(SECMEC, security_mechanism);
    ddm.add_code_point(RDBNAM, &pad_rdbnam(rdbnam));
    ddm.add_code_point(USRID, &utf8_to_ebcdic037(user_id));
    ddm.add_code_point(PASSWORD, &utf8_to_ebcdic037(password));
    ddm.build()
}

/// Build a SECCHK DDM command with user ID and password credentials.
///
/// The database name is sent in ACCSEC/ACCRDB. Some DB2 z/OS servers reject
/// RDBNAM when it is repeated inside SECCHK, so package code should prefer
/// this builder unless it explicitly needs the legacy framing above.
pub fn build_secchk_without_rdbnam(
    security_mechanism: u16,
    user_id: &str,
    password: &str,
) -> Vec<u8> {
    build_secchk_without_rdbnam_with_encoding(
        security_mechanism,
        user_id,
        password,
        CredentialEncoding::Ebcdic037,
    )
}

/// Build a SECCHK DDM command with encoded user ID and password credentials.
pub fn build_secchk_without_rdbnam_with_encoding(
    security_mechanism: u16,
    user_id: &str,
    password: &str,
    credential_encoding: CredentialEncoding,
) -> Vec<u8> {
    let mut ddm = DdmBuilder::new(SECCHK);
    ddm.add_u16(SECMEC, security_mechanism);
    ddm.add_code_point(USRID, &credential_encoding.encode(user_id));
    ddm.add_code_point(PASSWORD, &credential_encoding.encode(password));
    ddm.build()
}

/// Build SECCHK for user ID + password authentication.
///
/// The `rdbnam` argument is retained for API compatibility; it is not encoded
/// in SECCHK because the database name is already sent in ACCSEC/ACCRDB.
pub fn build_secchk_usridpwd(_rdbnam: &str, user_id: &str, password: &str) -> Vec<u8> {
    build_secchk_without_rdbnam(SECMEC_USRIDPWD, user_id, password)
}

/// Build SECCHK for user ID + password authentication with encoded credentials.
pub fn build_secchk_usridpwd_with_encoding(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    credential_encoding: CredentialEncoding,
) -> Vec<u8> {
    build_secchk_without_rdbnam_with_encoding(
        SECMEC_USRIDPWD,
        user_id,
        password,
        credential_encoding,
    )
}

/// Build SECCHK for encrypted user ID + password authentication (SECMEC 0x0009).
///
/// The user ID and password are encrypted with the Diffie-Hellman session key
/// negotiated through ACCSEC/ACCSECRD, then sent as two SECTKN parameters.
/// The `rdbnam` argument is retained for API compatibility; it is not encoded
/// in SECCHK because the database name is already sent in ACCSEC/ACCRDB.
pub fn build_secchk_eusridpwd(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
) -> Result<Vec<u8>> {
    build_secchk_eusridpwd_with_encoding(
        _rdbnam,
        user_id,
        password,
        server_sectkn,
        client_private,
        CredentialEncoding::Ebcdic037,
    )
}

/// Build SECCHK for encrypted user ID + password authentication with encoded credentials.
pub fn build_secchk_eusridpwd_with_encoding(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
    credential_encoding: CredentialEncoding,
) -> Result<Vec<u8>> {
    build_secchk_eusridpwd_with_algorithm_and_encoding(
        _rdbnam,
        user_id,
        password,
        server_sectkn,
        client_private,
        credential_encoding,
        EncryptionAlgorithm::Des,
    )
}

/// Build SECCHK for encrypted user ID + password authentication with encoded
/// credentials and the selected DRDA encryption algorithm.
pub fn build_secchk_eusridpwd_with_algorithm_and_encoding(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
    credential_encoding: CredentialEncoding,
    encryption_algorithm: EncryptionAlgorithm,
) -> Result<Vec<u8>> {
    let expected_sectkn_len = sectkn_len_for_algorithm(encryption_algorithm);
    if !valid_sectkn_len(server_sectkn, expected_sectkn_len) {
        return Err(ProtoError::Other(format!(
            "ACCSECRD returned an invalid SECTKN for encrypted authentication: expected {} bytes, got {}",
            expected_sectkn_len,
            server_sectkn.len()
        )));
    }

    let session_key = crate::secmec9::calculate_session_key_with_algorithm(
        server_sectkn,
        client_private,
        encryption_algorithm,
    );
    let encoded_user_id = credential_encoding.encode(user_id);
    let encoded_password = credential_encoding.encode(password);
    let encrypted_user_id = crate::secmec9::encrypt_userid_bytes_with_algorithm(
        &session_key,
        server_sectkn,
        &encoded_user_id,
        encryption_algorithm,
    );
    let encrypted_password = crate::secmec9::encrypt_password_bytes_with_algorithm(
        &session_key,
        server_sectkn,
        &encoded_password,
        encryption_algorithm,
    );

    let mut ddm = DdmBuilder::new(SECCHK);
    ddm.add_u16(SECMEC, SECMEC_EUSRIDPWD);
    ddm.add_code_point(SECTKN, &encrypted_user_id);
    ddm.add_code_point(SECTKN, &encrypted_password);
    Ok(ddm.build())
}

/// Build SECCHK for user ID + encrypted password authentication (SECMEC 0x0007).
///
/// The user ID is sent as a clear USRID parameter. The password is encrypted
/// with the Diffie-Hellman session key negotiated through ACCSEC/ACCSECRD and
/// sent as a SECTKN parameter.
pub fn build_secchk_usencpwd(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
) -> Result<Vec<u8>> {
    build_secchk_usencpwd_with_encoding(
        _rdbnam,
        user_id,
        password,
        server_sectkn,
        client_private,
        CredentialEncoding::Ebcdic037,
    )
}

/// Build SECCHK for user ID + encrypted password authentication with encoded credentials.
pub fn build_secchk_usencpwd_with_encoding(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
    credential_encoding: CredentialEncoding,
) -> Result<Vec<u8>> {
    build_secchk_usencpwd_with_encodings(
        _rdbnam,
        user_id,
        password,
        server_sectkn,
        client_private,
        EncryptedPasswordCredentialEncodings::same(credential_encoding),
    )
}

/// Build SECCHK for user ID + encrypted password authentication with separate
/// encodings for the clear user ID, encrypted password plaintext, and password IV token.
pub fn build_secchk_usencpwd_with_encodings(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
    credential_encodings: EncryptedPasswordCredentialEncodings,
) -> Result<Vec<u8>> {
    build_secchk_usencpwd_with_algorithm_and_encodings(
        _rdbnam,
        user_id,
        password,
        server_sectkn,
        client_private,
        credential_encodings,
        EncryptionAlgorithm::Des,
    )
}

/// Build SECCHK for user ID + encrypted password authentication with separate
/// encodings and the selected DRDA encryption algorithm.
pub fn build_secchk_usencpwd_with_algorithm_and_encodings(
    _rdbnam: &str,
    user_id: &str,
    password: &str,
    server_sectkn: &[u8],
    client_private: &[u8],
    credential_encodings: EncryptedPasswordCredentialEncodings,
    encryption_algorithm: EncryptionAlgorithm,
) -> Result<Vec<u8>> {
    let expected_sectkn_len = sectkn_len_for_algorithm(encryption_algorithm);
    if !valid_sectkn_len(server_sectkn, expected_sectkn_len) {
        return Err(ProtoError::Other(format!(
            "ACCSECRD returned an invalid SECTKN for encrypted password authentication: expected {} bytes, got {}",
            expected_sectkn_len,
            server_sectkn.len()
        )));
    }

    let session_key = crate::secmec9::calculate_session_key_with_algorithm(
        server_sectkn,
        client_private,
        encryption_algorithm,
    );
    let encoded_user_id = credential_encodings.user_id.encode(user_id);
    let encoded_password = credential_encodings.password.encode(password);
    let password_token = credential_encodings.password_token.encode(user_id);
    let encrypted_password = crate::secmec9::encrypt_password_with_userid_iv_bytes_with_algorithm(
        &session_key,
        server_sectkn,
        &password_token,
        &encoded_password,
        encryption_algorithm,
    );

    let mut ddm = DdmBuilder::new(SECCHK);
    ddm.add_u16(SECMEC, SECMEC_USRENCPWD);
    ddm.add_code_point(USRID, &encoded_user_id);
    ddm.add_code_point(SECTKN, &encrypted_password);
    Ok(ddm.build())
}

fn sectkn_len_for_algorithm(encryption_algorithm: EncryptionAlgorithm) -> usize {
    match encryption_algorithm {
        EncryptionAlgorithm::Des => 32,
        EncryptionAlgorithm::Aes => 64,
    }
}

fn valid_sectkn_len(server_sectkn: &[u8], expected_len: usize) -> bool {
    server_sectkn.len() == expected_len
        || (server_sectkn.len() == expected_len + 1 && server_sectkn[0] == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddm::DdmObject;

    fn unhex(value: &str) -> Vec<u8> {
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }

    #[test]
    fn secchk_500_preserves_all_variant_characters_in_both_credentials() {
        let bytes = build_secchk_usridpwd_with_encoding(
            "TESTDB",
            "u!^[]|¢¬",
            "Abc123!^[]|¢¬x",
            CredentialEncoding::Ebcdic500,
        );
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(
            obj.find_param(USRID).unwrap().data,
            unhex("a44f5f4a5abbb0ba")
        );
        assert_eq!(
            obj.find_param(PASSWORD).unwrap().data,
            unhex("c18283f1f2f34f5f4a5abbb0baa7")
        );
        assert!(obj.find_param(RDBNAM).is_none());
    }

    #[test]
    fn encrypted_500_credentials_match_independent_des_and_aes_vectors() {
        // Independent standard cp500 / IBM CLI plaintext reference vectors,
        // modular DH and Node/OpenSSL crypto (des-ede3-cbc with three identical
        // keys = DES, aes-256-cbc). Fixed private keys are test data only.
        for (encoding, algorithm, key_len, user9, password9, password7, encoded_user) in [
            (
                CredentialEncoding::Ebcdic500,
                EncryptionAlgorithm::Des,
                32,
                "a15468ed2fb7ff1c",
                "82565ce4f25c1c741967c6d8af223ba7",
                "2a8db25793fb98d0e43332c6696959bc",
                "a44f5f4a5abb",
            ),
            (
                CredentialEncoding::Ebcdic500,
                EncryptionAlgorithm::Aes,
                64,
                "f9c8281a22ff3be0dbc843d9973bbfc2",
                "54ad7e1e86ee34bb86eb9e2d7ef25ad3",
                "54ad7e1e86ee34bb86eb9e2d7ef25ad3",
                "a44f5f4a5abb",
            ),
            (
                CredentialEncoding::Ebcdic500Luw,
                EncryptionAlgorithm::Des,
                32,
                "942347efbcebe022",
                "82565ce4f25c1c748d00a593eba4fb51",
                "0fd8c7df52c09b8b20e78ef5af36d293",
                "a44f5f4a5a6a",
            ),
            (
                CredentialEncoding::Ebcdic500Luw,
                EncryptionAlgorithm::Aes,
                64,
                "4bbe6e44d94e515ae62b6c3dc0a0f7cb",
                "d476eca0c62e0331bdb054c638fc7678",
                "d476eca0c62e0331bdb054c638fc7678",
                "a44f5f4a5a6a",
            ),
        ] {
            let private = vec![0x11; key_len];
            let server_public = crate::secmec9::calculate_public_key_with_algorithm(
                &vec![0x22; key_len],
                algorithm,
            );
            let bytes = build_secchk_eusridpwd_with_algorithm_and_encoding(
                "TESTDB",
                "u!^[]|",
                "Abc123!^[]|¢¬x",
                &server_public,
                &private,
                encoding,
                algorithm,
            )
            .unwrap();
            let (obj, _) = DdmObject::parse(&bytes).unwrap();
            let tokens: Vec<_> = obj
                .parameters()
                .into_iter()
                .filter(|p| p.code_point == SECTKN)
                .collect();
            assert_eq!(tokens.len(), 2);
            assert_eq!(
                tokens[0].data,
                unhex(user9),
                "SECMEC 9 {algorithm:?} user ID"
            );
            assert_eq!(
                tokens[1].data,
                unhex(password9),
                "SECMEC 9 {algorithm:?} password"
            );
            assert!(obj.find_param(USRID).is_none());
            assert!(obj.find_param(PASSWORD).is_none());

            let bytes = build_secchk_usencpwd_with_algorithm_and_encodings(
                "TESTDB",
                "u!^[]|",
                "Abc123!^[]|¢¬x",
                &server_public,
                &private,
                EncryptedPasswordCredentialEncodings::same(encoding),
                algorithm,
            )
            .unwrap();
            let (obj, _) = DdmObject::parse(&bytes).unwrap();
            assert_eq!(obj.find_param(USRID).unwrap().data, unhex(encoded_user));
            assert_eq!(
                obj.find_param(SECTKN).unwrap().data,
                unhex(password7),
                "SECMEC 7 {algorithm:?} password"
            );
            assert!(obj.find_param(PASSWORD).is_none());
        }
    }

    #[test]
    fn test_build_secchk() {
        let bytes = build_secchk_usridpwd("testdb", "db2inst1", "password123");
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SECCHK);
        let params = obj.parameters();
        assert!(!params.iter().any(|p| p.code_point == RDBNAM));
        assert!(params.iter().any(|p| p.code_point == USRID));
        assert!(params.iter().any(|p| p.code_point == PASSWORD));
    }

    #[test]
    fn test_build_legacy_secchk_with_rdbnam() {
        let bytes = build_secchk(SECMEC_USRIDPWD, "testdb", "db2inst1", "password123");
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SECCHK);
        let params = obj.parameters();
        assert!(params.iter().any(|p| p.code_point == RDBNAM));
        assert!(params.iter().any(|p| p.code_point == USRID));
        assert!(params.iter().any(|p| p.code_point == PASSWORD));
    }

    #[test]
    fn test_build_secchk_eusridpwd() {
        let client_private = crate::secmec9::generate_private_key();
        let server_private = crate::secmec9::generate_private_key();
        let server_public = crate::secmec9::calculate_public_key(&server_private);

        let bytes = build_secchk_eusridpwd(
            "testdb",
            "db2inst1",
            "password123",
            &server_public,
            &client_private,
        )
        .unwrap();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SECCHK);
        let params = obj.parameters();
        assert!(!params.iter().any(|p| p.code_point == RDBNAM));
        assert_eq!(params.iter().filter(|p| p.code_point == SECTKN).count(), 2);
        assert!(!params.iter().any(|p| p.code_point == USRID));
        assert!(!params.iter().any(|p| p.code_point == PASSWORD));
    }

    #[test]
    fn test_build_secchk_usencpwd() {
        let client_private = crate::secmec9::generate_private_key();
        let server_private = crate::secmec9::generate_private_key();
        let server_public = crate::secmec9::calculate_public_key(&server_private);

        let bytes = build_secchk_usencpwd(
            "testdb",
            "db2inst1",
            "password123",
            &server_public,
            &client_private,
        )
        .unwrap();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(obj.code_point, SECCHK);
        let params = obj.parameters();
        assert!(!params.iter().any(|p| p.code_point == RDBNAM));
        assert!(params.iter().any(|p| p.code_point == USRID));
        assert_eq!(params.iter().filter(|p| p.code_point == SECTKN).count(), 1);
        assert!(!params.iter().any(|p| p.code_point == PASSWORD));
    }

    #[test]
    fn test_build_secchk_usencpwd_utf8_credentials() {
        let client_private = crate::secmec9::generate_private_key();
        let server_private = crate::secmec9::generate_private_key();
        let server_public = crate::secmec9::calculate_public_key(&server_private);

        let bytes = build_secchk_usencpwd_with_encoding(
            "testdb",
            "db2inst1",
            "password123",
            &server_public,
            &client_private,
            CredentialEncoding::Utf8,
        )
        .unwrap();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        let user = obj.find_param(USRID).expect("USRID should be present");

        assert_eq!(user.data, b"db2inst1");
        assert_eq!(
            obj.parameters()
                .iter()
                .filter(|p| p.code_point == SECTKN)
                .count(),
            1
        );
    }

    #[test]
    fn test_build_secchk_usencpwd_can_mix_password_encodings() {
        let client_private = crate::secmec9::generate_private_key();
        let server_private = crate::secmec9::generate_private_key();
        let server_public = crate::secmec9::calculate_public_key(&server_private);

        let bytes = build_secchk_usencpwd_with_encodings(
            "testdb",
            "db2inst1",
            "password123",
            &server_public,
            &client_private,
            EncryptedPasswordCredentialEncodings {
                user_id: CredentialEncoding::Utf8,
                password: CredentialEncoding::Ebcdic037,
                password_token: CredentialEncoding::Ebcdic037,
            },
        )
        .unwrap();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        let user = obj.find_param(USRID).expect("USRID should be present");

        assert_eq!(user.data, b"db2inst1");
        assert_eq!(
            obj.parameters()
                .iter()
                .filter(|p| p.code_point == SECTKN)
                .count(),
            1
        );
    }
}
