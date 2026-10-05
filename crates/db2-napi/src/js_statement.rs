use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::js_connection::JsQueryResult;
use crate::js_types::{
    client_error_to_js, js_params_to_db2, query_result_to_js, JsOutcome, JsParameter,
    JsQueryOptions, RowMode,
};

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
    // napi-rs emits one signature; append fixed overloads in its TS override so
    // rebuilding cannot let return-type context infer an absent array option.
    #[napi(
        ts_args_type = "params?: import('./types').QueryParameters | null, options?: import('./types').QueryOptions<'object'> | null",
        ts_return_type = "Promise<import('./types').QueryResult>;\n  execute(params: import('./types').QueryParameters | null | undefined, options: import('./types').QueryOptions<'array'>): Promise<import('./types').QueryResult<'array'>>;\n  execute(params: import('./types').QueryParameters | null | undefined, options: import('./types').QueryOptions | null | undefined): Promise<import('./types').QueryResult<import('./types').RowMode>>"
    )]
    pub async fn execute(
        &self,
        params: Option<Vec<JsParameter>>,
        options: Option<JsQueryOptions>,
    ) -> JsOutcome<JsQueryResult> {
        JsOutcome(
            async {
                let row_mode = RowMode::from_options(options)?;
                let mut guard = self.inner.lock().await;
                let stmt = guard
                    .as_mut()
                    .ok_or_else(|| napi::Error::from_reason("PreparedStatement is closed"))?;

                let db2_params = params.map(js_params_to_db2).unwrap_or_default();

                let param_refs: Vec<&dyn db2_client::ToSql> = db2_params
                    .iter()
                    .map(|p| p as &dyn db2_client::ToSql)
                    .collect();

                let result = stmt
                    .execute(&param_refs)
                    .await
                    .map_err(client_error_to_js)?;

                Ok(query_result_to_js(result, row_mode))
            }
            .await,
        )
    }

    /// Execute the prepared statement as a batch with multiple rows of parameters.
    /// Each element of `param_rows` is an array of parameter values for one row.
    #[napi(
        js_name = "executeBatch",
        ts_args_type = "paramRows: import('./types').QueryParameters[], options?: import('./types').QueryOptions<'object'> | null",
        ts_return_type = "Promise<import('./types').QueryResult>;\n  executeBatch(paramRows: import('./types').QueryParameters[], options: import('./types').QueryOptions<'array'>): Promise<import('./types').QueryResult<'array'>>;\n  executeBatch(paramRows: import('./types').QueryParameters[], options: import('./types').QueryOptions | null | undefined): Promise<import('./types').QueryResult<import('./types').RowMode>>"
    )]
    pub async fn execute_batch(
        &self,
        param_rows: Vec<Vec<JsParameter>>,
        options: Option<JsQueryOptions>,
    ) -> JsOutcome<JsQueryResult> {
        JsOutcome(
            async {
                let row_mode = RowMode::from_options(options)?;
                let guard = self.inner.lock().await;
                let stmt = guard
                    .as_ref()
                    .ok_or_else(|| napi::Error::from_reason("PreparedStatement is closed"))?;

                // Convert all rows from JSON to Db2Value
                let db2_rows: Vec<Vec<db2_proto::types::Db2Value>> =
                    param_rows.into_iter().map(js_params_to_db2).collect();

                // Build references for each row
                let param_ref_rows: Vec<Vec<&dyn db2_client::ToSql>> = db2_rows
                    .iter()
                    .map(|row| row.iter().map(|p| p as &dyn db2_client::ToSql).collect())
                    .collect();

                let result = stmt
                    .execute_batch(&param_ref_rows)
                    .await
                    .map_err(client_error_to_js)?;

                Ok(query_result_to_js(result, row_mode))
            }
            .await,
        )
    }

    #[napi(ts_return_type = "Promise<void>")]
    pub async fn close(&self) -> JsOutcome<()> {
        JsOutcome(
            async {
                let mut guard = self.inner.lock().await;
                if let Some(stmt) = guard.take() {
                    stmt.close().await.map_err(client_error_to_js)?;
                }
                Ok(())
            }
            .await,
        )
    }
}
