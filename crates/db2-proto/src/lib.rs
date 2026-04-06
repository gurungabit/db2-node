pub mod codepage;
pub mod codepoints;
pub mod commands;
pub mod ddm;
pub mod dss;
pub mod fdoca;
pub mod replies;
pub mod secmec9;
pub mod types;

#[derive(Debug, Clone, PartialEq)]
pub enum ProtoError {
    InvalidMagic(u8),
    BufferTooShort { expected: usize, actual: usize },
    InvalidDssType(u8),
    InvalidCodePoint(u16),
    UnexpectedReply { expected: u16, actual: u16 },
    InvalidSqlcard(String),
    EbcdicConversion(String),
    Utf8Error(std::string::FromUtf8Error),
    Other(String),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::InvalidMagic(m) => write!(f, "invalid DSS magic byte: 0x{:02X}", m),
            ProtoError::BufferTooShort { expected, actual } => {
                write!(
                    f,
                    "buffer too short: expected {} bytes, got {}",
                    expected, actual
                )
            }
            ProtoError::InvalidDssType(t) => write!(f, "invalid DSS type: 0x{:02X}", t),
            ProtoError::InvalidCodePoint(cp) => write!(f, "invalid code point: 0x{:04X}", cp),
            ProtoError::UnexpectedReply { expected, actual } => {
                write!(
                    f,
                    "unexpected reply: expected 0x{:04X}, got 0x{:04X}",
                    expected, actual
                )
            }
            ProtoError::InvalidSqlcard(msg) => write!(f, "invalid SQLCARD: {}", msg),
            ProtoError::EbcdicConversion(msg) => write!(f, "EBCDIC conversion error: {}", msg),
            ProtoError::Utf8Error(e) => write!(f, "UTF-8 error: {}", e),
            ProtoError::Other(msg) => write!(f, "{}", msg),
        }
    }
}

impl std::error::Error for ProtoError {}

impl From<std::string::FromUtf8Error> for ProtoError {
    fn from(e: std::string::FromUtf8Error) -> Self {
        ProtoError::Utf8Error(e)
    }
}

pub type Result<T> = std::result::Result<T, ProtoError>;
