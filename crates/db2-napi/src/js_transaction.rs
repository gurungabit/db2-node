use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::js_connection::JsQueryResult;
use crate::js_types::{
    client_error_to_js, js_params_to_db2, query_result_to_js, JsOutcome, JsParameter,
    JsQueryOptions, RowMode,
};

#[napi]
pub struct JsTransaction {
    inner: Arc<Mutex<Option<db2_client::Transaction>>>,
}

impl JsTransaction {
    /// Create a JsTransaction wrapping an active transaction.
    pub(crate) fn from_inner(txn: db2_client::Transaction) -> Self {
        JsTransaction {
            inner: Arc::new(Mutex::new(Some(txn))),
        }
    }
}

#[napi]
impl JsTransaction {
    // napi-rs emits one signature; append fixed overloads in its TS override so
    // rebuilding cannot let return-type context infer an absent array option.
    #[napi(
        ts_args_type = "sql: string, params?: import('./types').QueryParameters | null, options?: import('./types').QueryOptions<'object'> | null",
        ts_return_type = "Promise<import('./types').QueryResult>;\n  query(sql: string, params: import('./types').QueryParameters | null | undefined, options: import('./types').QueryOptions<'array'>): Promise<import('./types').QueryResult<'array'>>;\n  query(sql: string, params: import('./types').QueryParameters | null | undefined, options: import('./types').QueryOptions | null | undefined): Promise<import('./types').QueryResult<import('./types').RowMode>>"
    )]
    pub async fn query(
        &self,
        sql: String,
        params: Option<Vec<JsParameter>>,
        options: Option<JsQueryOptions>,
    ) -> JsOutcome<JsQueryResult> {
        JsOutcome(
            async {
                let row_mode = RowMode::from_options(options)?;
                let mut guard = self.inner.lock().await;
                let txn = guard.as_mut().ok_or_else(|| {
                    napi::Error::from_reason("Transaction is already committed or rolled back")
                })?;

                let db2_params = params.map(js_params_to_db2).unwrap_or_default();

                let param_refs: Vec<&dyn db2_client::ToSql> = db2_params
                    .iter()
                    .map(|p| p as &dyn db2_client::ToSql)
                    .collect();

                let result = txn
                    .query(&sql, &param_refs)
                    .await
                    .map_err(client_error_to_js)?;

                Ok(query_result_to_js(result, row_mode))
            }
            .await,
        )
    }

    /// Prepare a SQL statement within this transaction.
    #[napi(ts_return_type = "Promise<JsPreparedStatement>")]
    pub async fn prepare(
        &self,
        sql: String,
    ) -> JsOutcome<crate::js_statement::JsPreparedStatement> {
        JsOutcome(
            async {
                let mut guard = self.inner.lock().await;
                let txn = guard.as_mut().ok_or_else(|| {
                    napi::Error::from_reason("Transaction is already committed or rolled back")
                })?;

                let stmt = txn.prepare(&sql).await.map_err(client_error_to_js)?;
                Ok(crate::js_statement::JsPreparedStatement::from_inner(stmt))
            }
            .await,
        )
    }

    #[napi(ts_return_type = "Promise<void>")]
    pub async fn commit(&self) -> JsOutcome<()> {
        JsOutcome(
            async {
                let mut guard = self.inner.lock().await;
                let txn = guard.take().ok_or_else(|| {
                    napi::Error::from_reason("Transaction is already committed or rolled back")
                })?;
                txn.commit().await.map_err(client_error_to_js)?;
                Ok(())
            }
            .await,
        )
    }

    #[napi(ts_return_type = "Promise<void>")]
    pub async fn rollback(&self) -> JsOutcome<()> {
        JsOutcome(
            async {
                let mut guard = self.inner.lock().await;
                let txn = guard.take().ok_or_else(|| {
                    napi::Error::from_reason("Transaction is already committed or rolled back")
                })?;
                txn.rollback().await.map_err(client_error_to_js)?;
                Ok(())
            }
            .await,
        )
    }
}
