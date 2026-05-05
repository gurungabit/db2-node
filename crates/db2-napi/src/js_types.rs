use crate::js_connection::{JsColumnInfo, JsQueryResult};

/// Convert a db2_client::Config from our JS-facing connection config.
#[allow(clippy::too_many_arguments)]
pub fn config_from_js(
    host: &str,
    port: Option<u32>,
    database: &str,
    user: &str,
    password: &str,
    ssl: Option<bool>,
    reject_unauthorized: Option<bool>,
    ca_cert: Option<String>,
    security_mechanism: Option<String>,
    encryption_algorithm: Option<String>,
    credential_encoding: Option<String>,
    encrypted_password_encoding: Option<String>,
    encrypted_password_token_encoding: Option<String>,
    connect_timeout: Option<u32>,
    query_timeout: Option<u32>,
    frame_drain_timeout: Option<u32>,
    current_schema: Option<String>,
    type_definition_name: Option<String>,
    fetch_size: Option<u32>,
) -> napi::Result<db2_client::Config> {
    let mut config = db2_client::Config {
        host: host.to_string(),
        port: port.unwrap_or(50000) as u16,
        database: database.to_string(),
        user: user.to_string(),
        password: password.to_string(),
        security_mechanism: parse_security_mechanism(security_mechanism)?,
        encryption_algorithm: parse_encryption_algorithm(encryption_algorithm)?,
        credential_encoding: parse_credential_encoding(credential_encoding)?,
        encrypted_password_encoding: parse_encrypted_password_encoding(
            encrypted_password_encoding,
        )?,
        encrypted_password_token_encoding: parse_encrypted_password_encoding(
            encrypted_password_token_encoding,
        )?,
        ..db2_client::Config::default()
    };
    let use_ssl = ssl.unwrap_or(false);
    config.ssl = use_ssl;
    if use_ssl {
        config.ssl_config = Some(db2_client::SslConfig {
            reject_unauthorized: reject_unauthorized.unwrap_or(true),
            ca_cert,
            ..Default::default()
        });
    }
    if let Some(ct) = connect_timeout {
        config.connect_timeout = std::time::Duration::from_millis(ct as u64);
    }
    if let Some(qt) = query_timeout {
        config.query_timeout = std::time::Duration::from_millis(qt as u64);
    }
    if let Some(fd) = frame_drain_timeout {
        config.frame_drain_timeout = std::time::Duration::from_millis(fd as u64);
    }
    config.current_schema = current_schema;
    config.type_definition_name = parse_type_definition_name(type_definition_name)?;
    if let Some(fs) = fetch_size {
        config.fetch_size = fs;
    }
    Ok(config)
}

fn parse_security_mechanism(value: Option<String>) -> napi::Result<db2_client::SecurityMechanism> {
    let Some(value) = value else {
        return Ok(db2_client::SecurityMechanism::EncryptedUserPassword);
    };

    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| *c != '_' && *c != '-' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();

    match normalized.as_str() {
        ""
        | "9"
        | "secmec9"
        | "encrypted"
        | "eusridpwd"
        | "encrypteduserpassword"
        | "encrypteduseridpassword" => Ok(db2_client::SecurityMechanism::EncryptedUserPassword),
        "7" | "secmec7" | "usencpwd" | "usrencryptedpassword" | "encryptedpassword"
        | "encryptedpasswordonly" => Ok(db2_client::SecurityMechanism::EncryptedPassword),
        "3" | "secmec3" | "clear" | "usridpwd" | "userpassword" | "useridpassword" => {
            Ok(db2_client::SecurityMechanism::UserPassword)
        }
        "4" | "secmec4" | "usridonly" | "useronly" | "useridonly" => {
            Ok(db2_client::SecurityMechanism::UserOnly)
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported securityMechanism '{}'. Use 'encrypted', 'encryptedPassword', 'userPassword', or 'userOnly'.",
            value
        ))),
    }
}

fn parse_encryption_algorithm(
    value: Option<String>,
) -> napi::Result<db2_client::EncryptionAlgorithm> {
    let Some(value) = value else {
        return Ok(db2_client::EncryptionAlgorithm::Aes);
    };

    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| *c != '_' && *c != '-' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();

    match normalized.as_str() {
        "" | "des" | "des56" | "1" | "encalg1" => Ok(db2_client::EncryptionAlgorithm::Des),
        "aes" | "aes256" | "2" | "encalg2" => Ok(db2_client::EncryptionAlgorithm::Aes),
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported encryptionAlgorithm '{}'. Use 'des' or 'aes'.",
            value
        ))),
    }
}

fn parse_credential_encoding(
    value: Option<String>,
) -> napi::Result<db2_client::CredentialEncoding> {
    let Some(value) = value else {
        return Ok(db2_client::CredentialEncoding::Auto);
    };

    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| *c != '_' && *c != '-' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();

    match normalized.as_str() {
        "" | "auto" | "negotiated" => Ok(db2_client::CredentialEncoding::Auto),
        "utf8" | "unicode" | "unicode1208" | "ccsid1208" => {
            Ok(db2_client::CredentialEncoding::Utf8)
        }
        "ebcdic" | "ebcdic037" | "cp037" | "ibm037" | "ccsid37" | "ccsid037" => {
            Ok(db2_client::CredentialEncoding::Ebcdic037)
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported credentialEncoding '{}'. Use 'auto', 'utf8', or 'ebcdic'.",
            value
        ))),
    }
}

fn parse_encrypted_password_encoding(
    value: Option<String>,
) -> napi::Result<db2_client::EncryptedPasswordEncoding> {
    let Some(value) = value else {
        return Ok(db2_client::EncryptedPasswordEncoding::SameAsCredential);
    };

    let normalized: String = value
        .trim()
        .chars()
        .filter(|c| *c != '_' && *c != '-' && !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();

    match normalized.as_str() {
        "" | "same" | "auto" | "credential" | "credentials" | "credentialencoding"
        | "sameascredential" | "sameascredentials" => {
            Ok(db2_client::EncryptedPasswordEncoding::SameAsCredential)
        }
        "utf8" | "unicode" | "unicode1208" | "ccsid1208" => {
            Ok(db2_client::EncryptedPasswordEncoding::Utf8)
        }
        "ebcdic" | "ebcdic037" | "cp037" | "ibm037" | "ccsid37" | "ccsid037" => {
            Ok(db2_client::EncryptedPasswordEncoding::Ebcdic037)
        }
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported encrypted password encoding '{}'. Use 'same', 'utf8', or 'ebcdic'.",
            value
        ))),
    }
}

fn parse_type_definition_name(value: Option<String>) -> napi::Result<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };

    let normalized: String = value.trim().to_ascii_uppercase();
    match normalized.as_str() {
        "" => Ok(None),
        "NONE" | "OMIT" | "OMITTED" | "DISABLE" | "DISABLED" => Ok(Some(String::new())),
        "QTDSQL370" | "QTDSQLASC" | "QTDSQLX86" | "QTDSQL400" => Ok(Some(normalized)),
        _ => Err(napi::Error::from_reason(format!(
            "Unsupported typeDefinitionName '{}'. Use 'none', 'QTDSQL370', 'QTDSQLASC', 'QTDSQLX86', or 'QTDSQL400'.",
            value
        ))),
    }
}

/// Convert a db2_client::QueryResult into our JS-facing QueryResult struct.
pub fn query_result_to_js(result: db2_client::types::QueryResult) -> JsQueryResult {
    let columns: Vec<JsColumnInfo> = result
        .columns
        .iter()
        .map(|col| {
            let type_name = public_type_name(&col.type_name);
            JsColumnInfo {
                name: col.name.clone(),
                db2_type_name: raw_db2_type_name(&col.type_name, &type_name),
                type_name,
                nullable: col.nullable,
                precision: col.precision.map(|p| p as u32),
                scale: col.scale.map(|s| s as u32),
            }
        })
        .collect();

    let col_names: Vec<String> = result.columns.iter().map(|c| c.name.clone()).collect();
    let rows: Vec<serde_json::Value> = result
        .rows
        .iter()
        .map(|row| row_to_json(row, &col_names))
        .collect();

    JsQueryResult {
        rows,
        row_count: result.row_count,
        columns,
        diagnostics: result.diagnostics,
    }
}

fn public_type_name(type_name: &str) -> String {
    if let Some(len) = parse_enum_length(type_name, "Graphic") {
        return format!("CHAR({len})");
    }
    if let Some(len) = parse_enum_length(type_name, "VarGraphic") {
        return format!("VARCHAR({len})");
    }
    if let Some(len) = parse_enum_length(type_name, "RowId") {
        return format!("ROWID({len})");
    }
    match type_name {
        "Blob" => "BLOB".to_string(),
        "Clob" => "CLOB".to_string(),
        "DbClob" => "DBCLOB".to_string(),
        _ => type_name.to_string(),
    }
}

fn raw_db2_type_name(raw: &str, public: &str) -> Option<String> {
    let db2_type = if let Some(len) = parse_enum_length(raw, "Graphic") {
        format!("GRAPHIC({len})")
    } else if let Some(len) = parse_enum_length(raw, "VarGraphic") {
        format!("VARGRAPHIC({len})")
    } else if let Some(len) = parse_enum_length(raw, "RowId") {
        format!("ROWID({len})")
    } else if raw == "Blob" {
        "BLOB".to_string()
    } else if raw == "Clob" {
        "CLOB".to_string()
    } else if raw == "DbClob" {
        "DBCLOB".to_string()
    } else {
        raw.to_string()
    };

    (db2_type != public).then_some(db2_type)
}

fn parse_enum_length(type_name: &str, variant: &str) -> Option<u16> {
    let inner = type_name
        .strip_prefix(variant)?
        .strip_prefix('(')?
        .strip_suffix(')')?;
    inner.parse::<u16>().ok()
}

/// Convert a single Row to a JSON object using column names as keys.
///
/// Type resolution order matters: numeric types are tried before String
/// because `FromDb2Value for String` can convert numeric values to strings,
/// which would cause integers and floats to be returned as JSON strings
/// instead of JSON numbers.
fn row_to_json(row: &db2_client::Row, col_names: &[String]) -> serde_json::Value {
    let mut map = serde_json::Map::with_capacity(col_names.len());
    for (index, name) in col_names.iter().enumerate() {
        let value = row
            .values()
            .get(index)
            .map(db2_value_to_json)
            .unwrap_or(serde_json::Value::Null);
        map.insert(name.clone(), value);
    }
    serde_json::Value::Object(map)
}

fn db2_value_to_json(value: &db2_proto::types::Db2Value) -> serde_json::Value {
    use db2_proto::types::Db2Value;

    match value {
        Db2Value::Null => serde_json::Value::Null,
        Db2Value::SmallInt(v) => serde_json::Value::Number((*v as i64).into()),
        Db2Value::Integer(v) => serde_json::Value::Number((*v as i64).into()),
        Db2Value::BigInt(v) => serde_json::Value::Number((*v).into()),
        Db2Value::Real(v) => serde_json::Number::from_f64(*v as f64)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::Number(0.into())),
        Db2Value::Double(v) => serde_json::Number::from_f64(*v)
            .map(serde_json::Value::Number)
            .unwrap_or_else(|| serde_json::Value::Number(0.into())),
        Db2Value::Boolean(v) => serde_json::Value::Bool(*v),
        Db2Value::Binary(v) | Db2Value::Blob(v) => serde_json::Value::Array(
            v.iter()
                .map(|b| serde_json::Value::Number((*b).into()))
                .collect(),
        ),
        Db2Value::Decimal(v)
        | Db2Value::Char(v)
        | Db2Value::VarChar(v)
        | Db2Value::Clob(v)
        | Db2Value::RowId(v)
        | Db2Value::Date(v)
        | Db2Value::Time(v)
        | Db2Value::Timestamp(v)
        | Db2Value::Xml(v) => serde_json::Value::String(v.clone()),
    }
}

/// Convert JavaScript parameter values (passed as serde_json::Value) to Vec<Db2Value>.
pub fn js_params_to_db2(params: &[serde_json::Value]) -> Vec<db2_proto::types::Db2Value> {
    params.iter().map(json_to_db2_value).collect()
}

/// Convert a single JSON value to a Db2Value.
fn json_to_db2_value(val: &serde_json::Value) -> db2_proto::types::Db2Value {
    use db2_proto::types::Db2Value;
    match val {
        serde_json::Value::Null => Db2Value::Null,
        serde_json::Value::Bool(b) => Db2Value::Boolean(*b),
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                if i >= i32::MIN as i64 && i <= i32::MAX as i64 {
                    Db2Value::Integer(i as i32)
                } else {
                    Db2Value::BigInt(i)
                }
            } else if let Some(f) = n.as_f64() {
                Db2Value::Double(f)
            } else {
                Db2Value::Null
            }
        }
        serde_json::Value::String(s) => Db2Value::VarChar(s.clone()),
        serde_json::Value::Array(arr) => {
            // Treat arrays of numbers as binary data
            let bytes: Vec<u8> = arr
                .iter()
                .filter_map(|v| v.as_u64().map(|n| n as u8))
                .collect();
            Db2Value::Binary(bytes)
        }
        serde_json::Value::Object(_) => {
            // Objects are not directly supported; convert to string representation
            Db2Value::VarChar(val.to_string())
        }
    }
}

/// Convert a db2_client::Error into a napi::Error with descriptive message.
pub fn client_error_to_napi(err: db2_client::Error) -> napi::Error {
    let message = match &err {
        db2_client::Error::Sql {
            sqlstate,
            sqlcode,
            message,
        } => format!(
            "SQL Error [SQLSTATE={}, SQLCODE={}]: {}",
            sqlstate, sqlcode, message
        ),
        other => other.to_string(),
    };
    napi::Error::from_reason(message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn security_mechanism_parser_accepts_public_aliases() {
        assert_eq!(
            parse_security_mechanism(Some("encrypted".into())).unwrap(),
            db2_client::SecurityMechanism::EncryptedUserPassword
        );
        assert_eq!(
            parse_security_mechanism(Some("SECMEC-3".into())).unwrap(),
            db2_client::SecurityMechanism::UserPassword
        );
        assert_eq!(
            parse_security_mechanism(Some("encryptedPassword".into())).unwrap(),
            db2_client::SecurityMechanism::EncryptedPassword
        );
        assert!(parse_security_mechanism(Some("unsupported".into())).is_err());
    }

    #[test]
    fn credential_encoding_parser_accepts_public_aliases() {
        assert_eq!(
            parse_credential_encoding(None).unwrap(),
            db2_client::CredentialEncoding::Auto
        );
        assert_eq!(
            parse_credential_encoding(Some("utf-8".into())).unwrap(),
            db2_client::CredentialEncoding::Utf8
        );
        assert_eq!(
            parse_credential_encoding(Some("CP037".into())).unwrap(),
            db2_client::CredentialEncoding::Ebcdic037
        );
        assert!(parse_credential_encoding(Some("unsupported".into())).is_err());
    }

    #[test]
    fn encryption_algorithm_parser_accepts_public_aliases() {
        assert_eq!(
            parse_encryption_algorithm(None).unwrap(),
            db2_client::EncryptionAlgorithm::Aes
        );
        assert_eq!(
            parse_encryption_algorithm(Some("AES-256".into())).unwrap(),
            db2_client::EncryptionAlgorithm::Aes
        );
        assert_eq!(
            parse_encryption_algorithm(Some("DES".into())).unwrap(),
            db2_client::EncryptionAlgorithm::Des
        );
        assert!(parse_encryption_algorithm(Some("unsupported".into())).is_err());
    }

    #[test]
    fn encrypted_password_encoding_parser_accepts_public_aliases() {
        assert_eq!(
            parse_encrypted_password_encoding(None).unwrap(),
            db2_client::EncryptedPasswordEncoding::SameAsCredential
        );
        assert_eq!(
            parse_encrypted_password_encoding(Some("same-as-credential".into())).unwrap(),
            db2_client::EncryptedPasswordEncoding::SameAsCredential
        );
        assert_eq!(
            parse_encrypted_password_encoding(Some("utf-8".into())).unwrap(),
            db2_client::EncryptedPasswordEncoding::Utf8
        );
        assert_eq!(
            parse_encrypted_password_encoding(Some("CP037".into())).unwrap(),
            db2_client::EncryptedPasswordEncoding::Ebcdic037
        );
        assert!(parse_encrypted_password_encoding(Some("unsupported".into())).is_err());
    }

    #[test]
    fn type_definition_name_parser_accepts_supported_names() {
        assert_eq!(parse_type_definition_name(None).unwrap(), None);
        assert_eq!(
            parse_type_definition_name(Some(" qtdsqlasc ".into())).unwrap(),
            Some("QTDSQLASC".into())
        );
        assert_eq!(
            parse_type_definition_name(Some("QTDSQL370".into())).unwrap(),
            Some("QTDSQL370".into())
        );
        assert_eq!(
            parse_type_definition_name(Some("none".into())).unwrap(),
            Some(String::new())
        );
        assert!(parse_type_definition_name(Some("unsupported".into())).is_err());
    }

    #[test]
    fn graphic_column_types_are_publicly_shown_as_char_types() {
        assert_eq!(public_type_name("Graphic(1)"), "CHAR(1)");
        assert_eq!(public_type_name("VarGraphic(45)"), "VARCHAR(45)");
        assert_eq!(
            raw_db2_type_name("Graphic(1)", "CHAR(1)"),
            Some("GRAPHIC(1)".to_string())
        );
        assert_eq!(
            raw_db2_type_name("VarGraphic(45)", "VARCHAR(45)"),
            Some("VARGRAPHIC(45)".to_string())
        );
        assert_eq!(public_type_name("Clob"), "CLOB");
        assert_eq!(public_type_name("RowId(40)"), "ROWID(40)");
        assert_eq!(raw_db2_type_name("RowId(40)", "ROWID(40)"), None);
        assert_eq!(raw_db2_type_name("Integer", "Integer"), None);
    }
}
