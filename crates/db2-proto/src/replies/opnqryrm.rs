//! Parse OPNQRYRM (Open Query Complete Reply Message).
use crate::codepoints::*;
use crate::ddm::DdmObject;
use crate::{ProtoError, Result};

/// Open query reply data.
#[derive(Debug, Clone)]
pub struct OpenQueryReply {
    /// Severity code.
    pub severity_code: u16,
    /// Query protocol type actually used by the server.
    pub query_protocol_type: Option<u16>,
    /// Query instance identifier for follow-up CNTQRY/CLSQRY flows.
    pub query_instance_id: Option<Vec<u8>>,
    /// Other parameters from the reply.
    pub parameters: Vec<(u16, Vec<u8>)>,
}

impl OpenQueryReply {
    /// Check if the query opened successfully.
    pub fn is_success(&self) -> bool {
        self.severity_code == SRVCOD_INFO || self.severity_code == SRVCOD_WARNING
    }
}

/// Parse an OPNQRYRM DDM object.
pub fn parse_opnqryrm(obj: &DdmObject) -> Result<OpenQueryReply> {
    if obj.code_point != OPNQRYRM {
        return Err(ProtoError::UnexpectedReply {
            expected: OPNQRYRM,
            actual: obj.code_point,
        });
    }

    let params = obj.parameters();
    let mut severity_code: u16 = 0;
    let mut query_protocol_type: Option<u16> = None;
    let mut query_instance_id: Option<Vec<u8>> = None;
    let mut raw_params = Vec::new();

    for param in &params {
        match param.code_point {
            SVRCOD => {
                severity_code = param.as_u16().unwrap_or(0);
            }
            QRYPRCTYP => {
                query_protocol_type = param.as_u16();
            }
            QRYINSID => {
                query_instance_id = Some(param.data.clone());
            }
            _ => {
                raw_params.push((param.code_point, param.data.clone()));
            }
        }
    }

    Ok(OpenQueryReply {
        severity_code,
        query_protocol_type,
        query_instance_id,
        parameters: raw_params,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_opnqryrm() {
        let mut builder = crate::ddm::DdmBuilder::new(OPNQRYRM);
        builder.add_u16(SVRCOD, SRVCOD_INFO);
        builder.add_u16(QRYPRCTYP, QRYPRCTYP_LMTBLKPRC);
        let bytes = builder.build();
        let (obj, _) = DdmObject::parse(&bytes).unwrap();
        let reply = parse_opnqryrm(&obj).unwrap();
        assert!(reply.is_success());
        assert_eq!(reply.query_protocol_type, Some(QRYPRCTYP_LMTBLKPRC));
    }
}
