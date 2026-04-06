use std::collections::HashMap;

/// A single row from a DB2 query result set.
#[derive(Debug, Clone)]
pub struct Row {
    columns: Vec<String>,
    values: Vec<db2_proto::types::Db2Value>,
    column_map: HashMap<String, usize>,
}

impl Row {
    /// Create a new Row from column names and values.
    pub fn new(columns: Vec<String>, values: Vec<db2_proto::types::Db2Value>) -> Self {
        let column_map = columns
            .iter()
            .enumerate()
            .map(|(i, name)| (name.to_uppercase(), i))
            .collect();
        Row {
            columns,
            values,
            column_map,
        }
    }

    /// Get a value by column name, converting to the requested type.
    /// Column name matching is case-insensitive.
    pub fn get<T: FromDb2Value>(&self, column: &str) -> Option<T> {
        let idx = self.column_map.get(&column.to_uppercase())?;
        self.values.get(*idx).and_then(T::from_db2_value)
    }

    /// Get a value by column index, converting to the requested type.
    pub fn get_by_index<T: FromDb2Value>(&self, index: usize) -> Option<T> {
        self.values.get(index).and_then(T::from_db2_value)
    }

    /// Check if a column value is NULL. Column name matching is case-insensitive.
    pub fn is_null(&self, column: &str) -> bool {
        match self.column_map.get(&column.to_uppercase()) {
            Some(idx) => matches!(
                self.values.get(*idx),
                Some(db2_proto::types::Db2Value::Null)
            ),
            None => true,
        }
    }

    /// Check if a column value is NULL by index.
    pub fn is_null_by_index(&self, index: usize) -> bool {
        matches!(
            self.values.get(index),
            Some(db2_proto::types::Db2Value::Null) | None
        )
    }

    /// Return the column names for this row.
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// Return the raw values for this row.
    pub fn values(&self) -> &[db2_proto::types::Db2Value] {
        &self.values
    }

    /// Return the number of columns.
    pub fn len(&self) -> usize {
        self.columns.len()
    }

    /// Return whether this row has no columns.
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
}

/// Trait for converting from a DB2 protocol value to a Rust type.
pub trait FromDb2Value: Sized {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self>;
}

impl FromDb2Value for i16 {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::SmallInt(v) => Some(*v),
            db2_proto::types::Db2Value::Integer(v) => Some(*v as i16),
            db2_proto::types::Db2Value::BigInt(v) => Some(*v as i16),
            _ => None,
        }
    }
}

impl FromDb2Value for i32 {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::SmallInt(v) => Some(*v as i32),
            db2_proto::types::Db2Value::Integer(v) => Some(*v),
            db2_proto::types::Db2Value::BigInt(v) => Some(*v as i32),
            _ => None,
        }
    }
}

impl FromDb2Value for i64 {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::SmallInt(v) => Some(*v as i64),
            db2_proto::types::Db2Value::Integer(v) => Some(*v as i64),
            db2_proto::types::Db2Value::BigInt(v) => Some(*v),
            _ => None,
        }
    }
}

impl FromDb2Value for f32 {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::Real(v) => Some(*v),
            db2_proto::types::Db2Value::Double(v) => Some(*v as f32),
            db2_proto::types::Db2Value::SmallInt(v) => Some(*v as f32),
            db2_proto::types::Db2Value::Integer(v) => Some(*v as f32),
            _ => None,
        }
    }
}

impl FromDb2Value for f64 {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::Real(v) => Some(*v as f64),
            db2_proto::types::Db2Value::Double(v) => Some(*v),
            db2_proto::types::Db2Value::SmallInt(v) => Some(*v as f64),
            db2_proto::types::Db2Value::Integer(v) => Some(*v as f64),
            db2_proto::types::Db2Value::BigInt(v) => Some(*v as f64),
            _ => None,
        }
    }
}

impl FromDb2Value for String {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::VarChar(v) => Some(v.clone()),
            db2_proto::types::Db2Value::Char(v) => Some(v.clone()),
            db2_proto::types::Db2Value::Clob(v) => Some(v.clone()),
            db2_proto::types::Db2Value::Date(v) => Some(v.clone()),
            db2_proto::types::Db2Value::Time(v) => Some(v.clone()),
            db2_proto::types::Db2Value::Timestamp(v) => Some(v.clone()),
            db2_proto::types::Db2Value::Xml(v) => Some(v.clone()),
            db2_proto::types::Db2Value::SmallInt(v) => Some(v.to_string()),
            db2_proto::types::Db2Value::Integer(v) => Some(v.to_string()),
            db2_proto::types::Db2Value::BigInt(v) => Some(v.to_string()),
            db2_proto::types::Db2Value::Real(v) => Some(v.to_string()),
            db2_proto::types::Db2Value::Double(v) => Some(v.to_string()),
            db2_proto::types::Db2Value::Decimal(v) => Some(v.clone()),
            _ => None,
        }
    }
}

impl FromDb2Value for bool {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::Boolean(v) => Some(*v),
            db2_proto::types::Db2Value::SmallInt(v) => Some(*v != 0),
            db2_proto::types::Db2Value::Integer(v) => Some(*v != 0),
            _ => None,
        }
    }
}

impl FromDb2Value for Vec<u8> {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::Blob(v) => Some(v.clone()),
            _ => None,
        }
    }
}

impl<T: FromDb2Value> FromDb2Value for Option<T> {
    fn from_db2_value(value: &db2_proto::types::Db2Value) -> Option<Self> {
        match value {
            db2_proto::types::Db2Value::Null => Some(None),
            other => Some(T::from_db2_value(other)),
        }
    }
}
