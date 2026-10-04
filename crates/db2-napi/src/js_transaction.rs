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
    #[napi(
        ts_generic_types = "M extends import('./types').RowMode = 'object'",
        ts_return_type = "Promise<import('./types').QueryResult<M>>"
    )]
    pub async fn query(
        &self,
        sql: String,
        #[napi(
            ts_arg_type = "Array<string | number | bigint | boolean | Date | null | Uint8Array | ArrayBuffer | number[]> | undefined | null"
        )]
        params: Option<Vec<JsParameter>>,
        #[napi(ts_arg_type = "import('./types').QueryOptions<M> | undefined | null")]
        options: Option<JsQueryOptions>,
    ) -> JsOutcome<JsQueryResult> {
        JsOutcome(
            async {
                let row_mode = RowMode::from_options(options)?;
                let mut guard = self.inner.lock().await;
                let txn = guard.as_mut().ok_or_else(|| {
                    napi::Error::from_reason("Transaction is already committed or rolled back")
                })?;

                let db2_params = match &params {
                    Some(p) => js_params_to_db2(p),
                    None => Vec::new(),
                };

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
