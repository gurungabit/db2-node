//! Parse ACCSECRD (Access Security Reply Data).
use crate::codepoints::*;
use crate::ddm::DdmObject;
use crate::{ProtoError, Result};

/// Access security reply data.
#[derive(Debug, Clone)]
pub struct AccSecReply {
    pub security_mechanism: u16,
    pub security_token: Option<Vec<u8>>,
    pub encryption_algorithm: Option<u16>,
    pub encryption_key_length: Option<u16>,
    pub security_check_code: Option<u8>,
}

/// Parse an ACCSECRD DDM object.
pub fn parse_accsecrd(obj: &DdmObject) -> Result<AccSecReply> {
    if obj.code_point != ACCSECRD {
        return Err(ProtoError::UnexpectedReply {
            expected: ACCSECRD,
            actual: obj.code_point,
        });
    }

    let params = obj.parameters();
    let mut secmec: Option<u16> = None;
    let mut sectkn: Option<Vec<u8>> = None;
    let mut encalg: Option<u16> = None;
    let mut enckeylen: Option<u16> = None;
    let mut check_code = None;

    for param in &params {
        match param.code_point {
            SECMEC => {
                secmec = param.as_u16();
            }
            SECTKN => {
                sectkn = Some(param.data.clone());
            }
            ENCALG => {
                encalg = param.as_u16();
            }
            ENCKEYLEN => {
                enckeylen = param.as_u16();
            }
            SECCHKCD => check_code = param.data.first().copied(),
            _ => {}
        }
    }

    Ok(AccSecReply {
        security_mechanism: secmec.unwrap_or(0),
        security_token: sectkn,
        encryption_algorithm: encalg,
        encryption_key_length: enckeylen,
        security_check_code: check_code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_accsecrd() {
        let mut builder = crate::ddm::DdmBuilder::new(ACCSECRD);
        builder.add_u16(SECMEC, SECMEC_USRIDPWD);
        let bytes = builder.build();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        let reply = parse_accsecrd(&obj).unwrap();
        assert_eq!(reply.security_mechanism, SECMEC_USRIDPWD);
        assert!(reply.security_token.is_none());
        assert_eq!(reply.encryption_algorithm, None);
        assert_eq!(reply.encryption_key_length, None);
    }

    #[test]
    fn test_parse_accsecrd_encryption_parameters() {
        let mut builder = crate::ddm::DdmBuilder::new(ACCSECRD);
        builder.add_u16(SECMEC, SECMEC_USRENCPWD);
        builder.add_u16(ENCALG, ENCALG_AES);
        builder.add_u16(ENCKEYLEN, ENCKEYLEN_AES_256);
        builder.add_code_point(SECCHKCD, &[0x1B]);
        let bytes = builder.build();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        let reply = parse_accsecrd(&obj).unwrap();
        assert_eq!(reply.security_mechanism, SECMEC_USRENCPWD);
        assert_eq!(reply.encryption_algorithm, Some(ENCALG_AES));
        assert_eq!(reply.encryption_key_length, Some(ENCKEYLEN_AES_256));
        assert_eq!(reply.security_check_code, Some(0x1B));
    }
}
