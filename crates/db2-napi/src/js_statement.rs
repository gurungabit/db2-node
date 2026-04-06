use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::js_connection::JsQueryResult;
use crate::js_types::{client_error_to_napi, js_params_to_db2, query_result_to_js};

#[napi]
pub struct JsPreparedStatement {
    inner: Arc<Mutex<Option<db2_client::PreparedStatement>>>,
}

impl JsPreparedStatement {
    /// Create a JsPreparedStatement wrapping an already-prepared statement.
    pub(crate) fn from_inner(stmt: db2_client::PreparedStatement) -> Self {
        JsPreparedStatement {
            inner: Arc::new(Mutex::new(Some(stmt))),
        }
    }
}

#[napi]
impl JsPreparedStatement {
    #[napi]
    pub async fn execute(&self, params: Option<Vec<serde_json::Value>>) -> Result<JsQueryResult> {
        let mut guard = self.inner.lock().await;
        let stmt = guard
            .as_mut()
            .ok_or_else(|| napi::Error::from_reason("PreparedStatement is closed"))?;

        let db2_params = match &params {
            Some(p) => js_params_to_db2(p),
            None => Vec::new(),
        };

        let param_refs: Vec<&dyn db2_client::ToSql> = db2_params
            .iter()
            .map(|p| p as &dyn db2_client::ToSql)
            .collect();

        let result = stmt
            .execute(&param_refs)
            .await
            .map_err(client_error_to_napi)?;

        Ok(query_result_to_js(result))
    }

    /// Execute the prepared statement as a batch with multiple rows of parameters.
    /// Each element of `param_rows` is an array of parameter values for one row.
    #[napi(js_name = "executeBatch")]
    pub async fn execute_batch(
        &self,
        param_rows: Vec<Vec<serde_json::Value>>,
    ) -> Result<JsQueryResult> {
        let guard = self.inner.lock().await;
        let stmt = guard
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("PreparedStatement is closed"))?;

        // Convert all rows from JSON to Db2Value
        let db2_rows: Vec<Vec<db2_proto::types::Db2Value>> =
            param_rows.iter().map(|row| js_params_to_db2(row)).collect();

        // Build references for each row
        let param_ref_rows: Vec<Vec<&dyn db2_client::ToSql>> = db2_rows
            .iter()
            .map(|row| row.iter().map(|p| p as &dyn db2_client::ToSql).collect())
            .collect();

        let result = stmt
            .execute_batch(&param_ref_rows)
            .await
            .map_err(client_error_to_napi)?;

        Ok(query_result_to_js(result))
    }

    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if let Some(stmt) = guard.take() {
            stmt.close().await.map_err(client_error_to_napi)?;
        }
        Ok(())
    }
}
