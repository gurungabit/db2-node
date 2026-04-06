use crate::column::ColumnInfo;
use crate::row::Row;
use db2_proto::types::{Db2Type, Db2Value};

/// Result of executing a SQL query or statement.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Rows returned by a SELECT query. Empty for non-SELECT statements.
    pub rows: Vec<Row>,
    /// Number of rows affected (for INSERT/UPDATE/DELETE) or fetched (for SELECT).
    pub row_count: i64,
    /// Column metadata for the result set.
    pub columns: Vec<ColumnInfo>,
}

impl QueryResult {
    /// Create an empty QueryResult for statements that return no rows.
    pub fn empty(row_count: i64) -> Self {
        QueryResult {
            rows: Vec::new(),
            row_count,
            columns: Vec::new(),
        }
    }

    /// Create a QueryResult with rows and column info.
    pub fn with_rows(rows: Vec<Row>, columns: Vec<ColumnInfo>) -> Self {
        let row_count = rows.len() as i64;
        QueryResult {
            rows,
            row_count,
            columns,
        }
    }
}

/// Trait for converting Rust types to DB2 protocol values.
pub trait ToSql: Send + Sync {
    /// Convert this value to a Db2Value for use in parameterized queries.
    fn to_db2_value(&self) -> Db2Value;

    /// Return the DB2 type for this value.
    fn db2_type(&self) -> Db2Type;
}

impl ToSql for i16 {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::SmallInt(*self)
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::SmallInt
    }
}

impl ToSql for i32 {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::Integer(*self)
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::Integer
    }
}

impl ToSql for i64 {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::BigInt(*self)
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::BigInt
    }
}

impl ToSql for f32 {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::Real(*self)
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::Real
    }
}

impl ToSql for f64 {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::Double(*self)
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::Double
    }
}

impl ToSql for &str {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::VarChar(self.to_string())
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::VarChar(self.len() as u16)
    }
}

impl ToSql for String {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::VarChar(self.clone())
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::VarChar(self.len() as u16)
    }
}

impl ToSql for bool {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::Boolean(*self)
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::Boolean
    }
}

impl ToSql for Vec<u8> {
    fn to_db2_value(&self) -> Db2Value {
        Db2Value::Blob(self.clone())
    }
    fn db2_type(&self) -> Db2Type {
        Db2Type::Blob
    }
}

impl ToSql for Db2Value {
    fn to_db2_value(&self) -> Db2Value {
        self.clone()
    }
    fn db2_type(&self) -> Db2Type {
        match self {
            Db2Value::Null => Db2Type::Null,
            Db2Value::SmallInt(_) => Db2Type::SmallInt,
            Db2Value::Integer(_) => Db2Type::Integer,
            Db2Value::BigInt(_) => Db2Type::BigInt,
            Db2Value::Real(_) => Db2Type::Real,
            Db2Value::Double(_) => Db2Type::Double,
            Db2Value::Decimal(_) => Db2Type::Decimal {
                precision: 15,
                scale: 2,
            },
            Db2Value::Char(s) => Db2Type::Char(s.len() as u16),
            Db2Value::VarChar(s) => Db2Type::VarChar(s.len() as u16),
            Db2Value::Binary(b) => Db2Type::VarBinary(b.len() as u16),
            Db2Value::Blob(_) => Db2Type::Blob,
            Db2Value::Clob(_) => Db2Type::Clob,
            Db2Value::Date(_) => Db2Type::Date,
            Db2Value::Time(_) => Db2Type::Time,
            Db2Value::Timestamp(_) => Db2Type::Timestamp,
            Db2Value::Boolean(_) => Db2Type::Boolean,
            Db2Value::Xml(_) => Db2Type::Xml,
        }
    }
}

impl<T: ToSql> ToSql for Option<T> {
    fn to_db2_value(&self) -> Db2Value {
        match self {
            Some(v) => v.to_db2_value(),
            None => Db2Value::Null,
        }
    }
    fn db2_type(&self) -> Db2Type {
        match self {
            Some(v) => v.db2_type(),
            None => Db2Type::VarChar(0),
        }
    }
}

/// Encode a Db2Value to its wire format bytes.
///
/// This is used when building SQLDTA for parameterized queries.
#[allow(dead_code)]
pub(crate) fn encode_db2_value(value: &Db2Value) -> Vec<u8> {
    match value {
        Db2Value::Null => vec![0xFF], // null indicator
        // Under QTDSQLX86, fixed-width numeric parameter data is little-endian.
        Db2Value::SmallInt(v) => v.to_le_bytes().to_vec(),
        Db2Value::Integer(v) => v.to_le_bytes().to_vec(),
        Db2Value::BigInt(v) => v.to_le_bytes().to_vec(),
        Db2Value::Real(v) => v.to_le_bytes().to_vec(),
        Db2Value::Double(v) => v.to_le_bytes().to_vec(),
        Db2Value::Decimal(s) => {
            // Encode as varchar for simplicity; a full impl would use packed BCD
            encode_varchar(s.as_bytes())
        }
        Db2Value::Char(s) | Db2Value::VarChar(s) | Db2Value::Clob(s) => {
            encode_varchar(s.as_bytes())
        }
        Db2Value::Date(s) | Db2Value::Time(s) | Db2Value::Timestamp(s) => {
            encode_varchar(s.as_bytes())
        }
        Db2Value::Binary(b) | Db2Value::Blob(b) => {
            let mut out = Vec::with_capacity(2 + b.len());
            out.extend_from_slice(&(b.len() as u16).to_be_bytes());
            out.extend_from_slice(b);
            out
        }
        Db2Value::Boolean(v) => vec![if *v { 1 } else { 0 }],
        Db2Value::Xml(s) => encode_varchar(s.as_bytes()),
    }
}

/// Encode bytes as a variable-length field (2-byte length prefix + data).
#[allow(dead_code)]
fn encode_varchar(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(2 + data.len());
    out.extend_from_slice(&(data.len() as u16).to_be_bytes());
    out.extend_from_slice(data);
    out
}
