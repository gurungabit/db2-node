use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::js_connection::JsQueryResult;
use crate::js_types::{client_error_to_napi, js_params_to_db2, query_result_to_js};

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
    #[napi]
    pub async fn query(
        &self,
        sql: String,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<JsQueryResult> {
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
            .map_err(client_error_to_napi)?;

        Ok(query_result_to_js(result))
    }

    /// Prepare a SQL statement within this transaction.
    #[napi]
    pub async fn prepare(&self, sql: String) -> Result<crate::js_statement::JsPreparedStatement> {
        let mut guard = self.inner.lock().await;
        let txn = guard.as_mut().ok_or_else(|| {
            napi::Error::from_reason("Transaction is already committed or rolled back")
        })?;

        let stmt = txn.prepare(&sql).await.map_err(client_error_to_napi)?;
        Ok(crate::js_statement::JsPreparedStatement::from_inner(stmt))
    }

    #[napi]
    pub async fn commit(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let txn = guard.take().ok_or_else(|| {
            napi::Error::from_reason("Transaction is already committed or rolled back")
        })?;
        txn.commit().await.map_err(client_error_to_napi)?;
        Ok(())
    }

    #[napi]
    pub async fn rollback(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        let txn = guard.take().ok_or_else(|| {
            napi::Error::from_reason("Transaction is already committed or rolled back")
        })?;
        txn.rollback().await.map_err(client_error_to_napi)?;
        Ok(())
    }
}
