use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("Connection failed: {0}")]
    Connection(String),

    #[error("Authentication failed: {0}")]
    Auth(String),

    #[error("SQL error (SQLSTATE={sqlstate}, SQLCODE={sqlcode}): {message}")]
    Sql {
        sqlstate: String,
        sqlcode: i32,
        message: String,
    },

    #[error("Protocol error: {0}")]
    ParameterCount(String),

    #[error("{0}")]
    ParameterType(String),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Pool error: {0}")]
    Pool(String),

    #[error("TLS error: {0}")]
    Tls(String),

    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Stable client-side classification, independent of the human-readable message.
    /// Server SQL errors retain their SQLSTATE/SQLCODE and have no driver code.
    pub fn driver_code(&self) -> Option<&'static str> {
        match self {
            Error::ParameterCount(_) => Some("DB2_PARAMETER_COUNT"),
            Error::ParameterType(_) => Some("DB2_PARAMETER_TYPE"),
            Error::Protocol(_) => Some("DB2_PROTOCOL"),
            _ => None,
        }
    }

    pub fn is_auth_error(&self) -> bool {
        matches!(self, Error::Auth(_))
    }

    pub fn sqlstate(&self) -> Option<&str> {
        match self {
            Error::Sql { sqlstate, .. } => Some(sqlstate),
            _ => None,
        }
    }

    pub fn sqlcode(&self) -> Option<i32> {
        match self {
            Error::Sql { sqlcode, .. } => Some(*sqlcode),
            _ => None,
        }
    }
}

impl From<db2_proto::ProtoError> for Error {
    fn from(e: db2_proto::ProtoError) -> Self {
        Error::Protocol(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_classification_does_not_replace_server_diagnostics() {
        for (error, code) in [
            (
                Error::ParameterCount("wrong count".into()),
                "DB2_PARAMETER_COUNT",
            ),
            (
                Error::ParameterType("wrong type".into()),
                "DB2_PARAMETER_TYPE",
            ),
            (Error::Protocol("bad frame".into()), "DB2_PROTOCOL"),
        ] {
            assert_eq!(error.driver_code(), Some(code));
            assert_eq!(error.sqlstate(), None);
            assert_eq!(error.sqlcode(), None);
        }
        let error = Error::Sql {
            sqlstate: "07001".into(),
            sqlcode: -313,
            message: "server error".into(),
        };
        assert_eq!(error.driver_code(), None);
        assert_eq!(error.sqlstate(), Some("07001"));
        assert_eq!(error.sqlcode(), Some(-313));
    }
}
