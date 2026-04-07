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
    connect_timeout: Option<u32>,
    query_timeout: Option<u32>,
    frame_drain_timeout: Option<u32>,
    current_schema: Option<String>,
    fetch_size: Option<u32>,
) -> db2_client::Config {
    let mut config = db2_client::Config {
        host: host.to_string(),
        port: port.unwrap_or(50000) as u16,
        database: database.to_string(),
        user: user.to_string(),
        password: password.to_string(),
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
    if let Some(fs) = fetch_size {
        config.fetch_size = fs;
    }
    config
}

/// Convert a db2_client::QueryResult into our JS-facing QueryResult struct.
pub fn query_result_to_js(result: db2_client::types::QueryResult) -> JsQueryResult {
    let columns: Vec<JsColumnInfo> = result
        .columns
        .iter()
        .map(|col| JsColumnInfo {
            name: col.name.clone(),
            type_name: col.type_name.clone(),
            nullable: col.nullable,
            precision: col.precision.map(|p| p as u32),
            scale: col.scale.map(|s| s as u32),
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
    }
}

/// Convert a single Row to a JSON object using column names as keys.
///
/// Type resolution order matters: numeric types are tried before String
/// because `FromDb2Value for String` can convert numeric values to strings,
/// which would cause integers and floats to be returned as JSON strings
/// instead of JSON numbers.
fn row_to_json(row: &db2_client::Row, col_names: &[String]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for name in col_names {
        let is_null = row.is_null(name);
        if is_null {
            map.insert(name.clone(), serde_json::Value::Null);
        } else if let Some(v) = row.get::<i64>(name) {
            map.insert(name.clone(), serde_json::Value::Number(v.into()));
        } else if let Some(v) = row.get::<f64>(name) {
            let num =
                serde_json::Number::from_f64(v).unwrap_or_else(|| serde_json::Number::from(0));
            map.insert(name.clone(), serde_json::Value::Number(num));
        } else if let Some(v) = row.get::<bool>(name) {
            map.insert(name.clone(), serde_json::Value::Bool(v));
        } else if let Some(v) = row.get::<Vec<u8>>(name) {
            let arr: Vec<serde_json::Value> = v
                .into_iter()
                .map(|b| serde_json::Value::Number(b.into()))
                .collect();
            map.insert(name.clone(), serde_json::Value::Array(arr));
        } else if let Some(v) = row.get::<String>(name) {
            map.insert(name.clone(), serde_json::Value::String(v));
        } else {
            map.insert(name.clone(), serde_json::Value::Null);
        }
    }
    serde_json::Value::Object(map)
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
