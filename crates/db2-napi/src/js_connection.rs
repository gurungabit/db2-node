use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::js_types::{client_error_to_napi, config_from_js, js_params_to_db2, query_result_to_js};

#[napi(object)]
pub struct JsConnectionConfig {
    pub host: String,
    pub port: Option<u32>,
    pub database: String,
    pub user: String,
    pub password: String,
    pub ssl: Option<bool>,
    pub reject_unauthorized: Option<bool>,
    pub ca_cert: Option<String>,
    pub connect_timeout: Option<u32>,
    pub query_timeout: Option<u32>,
    pub frame_drain_timeout: Option<u32>,
    pub current_schema: Option<String>,
    pub fetch_size: Option<u32>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsQueryResult {
    pub rows: Vec<serde_json::Value>,
    pub row_count: i64,
    pub columns: Vec<JsColumnInfo>,
}

#[napi(object)]
#[derive(Clone)]
pub struct JsColumnInfo {
    pub name: String,
    pub type_name: String,
    pub nullable: bool,
    pub precision: Option<u32>,
    pub scale: Option<u32>,
}

#[napi(object)]
pub struct JsServerInfo {
    pub product_name: String,
    pub server_release: String,
}

#[napi]
pub struct JsClient {
    pub(crate) inner: Arc<Mutex<Option<db2_client::Client>>>,
    config: db2_client::Config,
}

#[napi]
impl JsClient {
    #[napi(constructor)]
    pub fn new(config: JsConnectionConfig) -> Result<Self> {
        let client_config = config_from_js(
            &config.host,
            config.port,
            &config.database,
            &config.user,
            &config.password,
            config.ssl,
            config.reject_unauthorized,
            config.ca_cert,
            config.connect_timeout,
            config.query_timeout,
            config.frame_drain_timeout,
            config.current_schema,
            config.fetch_size,
        );
        Ok(JsClient {
            inner: Arc::new(Mutex::new(None)),
            config: client_config,
        })
    }

    /// Create a JsClient wrapping an already-connected db2_client::Client.
    pub(crate) fn from_inner(client: db2_client::Client, config: db2_client::Config) -> Self {
        JsClient {
            inner: Arc::new(Mutex::new(Some(client))),
            config,
        }
    }

    #[napi]
    pub async fn connect(&self) -> Result<()> {
        let mut client = db2_client::Client::new(self.config.clone());
        client.connect().await.map_err(client_error_to_napi)?;
        let mut guard = self.inner.lock().await;
        *guard = Some(client);
        Ok(())
    }

    #[napi]
    pub async fn query(
        &self,
        sql: String,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<JsQueryResult> {
        let mut guard = self.inner.lock().await;
        let client = guard
            .as_mut()
            .ok_or_else(|| napi::Error::from_reason("Client is not connected"))?;

        let db2_params = match &params {
            Some(p) => js_params_to_db2(p),
            None => Vec::new(),
        };

        let param_refs: Vec<&dyn db2_client::ToSql> = db2_params
            .iter()
            .map(|p| p as &dyn db2_client::ToSql)
            .collect();

        let result = client
            .query(&sql, &param_refs)
            .await
            .map_err(client_error_to_napi)?;

        Ok(query_result_to_js(result))
    }

    #[napi]
    pub async fn prepare(&self, sql: String) -> Result<crate::js_statement::JsPreparedStatement> {
        let mut guard = self.inner.lock().await;
        let client = guard
            .as_mut()
            .ok_or_else(|| napi::Error::from_reason("Client is not connected"))?;

        let stmt = client.prepare(&sql).await.map_err(client_error_to_napi)?;

        Ok(crate::js_statement::JsPreparedStatement::from_inner(stmt))
    }

    #[napi(js_name = "beginTransaction")]
    pub async fn begin_transaction(&self) -> Result<crate::js_transaction::JsTransaction> {
        let mut guard = self.inner.lock().await;
        let client = guard
            .as_mut()
            .ok_or_else(|| napi::Error::from_reason("Client is not connected"))?;

        let txn = client
            .begin_transaction()
            .await
            .map_err(client_error_to_napi)?;

        Ok(crate::js_transaction::JsTransaction::from_inner(txn))
    }

    #[napi]
    pub async fn close(&self) -> Result<()> {
        let mut guard = self.inner.lock().await;
        if let Some(client) = guard.take() {
            client.close().await.map_err(client_error_to_napi)?;
        }
        Ok(())
    }

    #[napi(js_name = "serverInfo")]
    pub async fn server_info(&self) -> Result<JsServerInfo> {
        let guard = self.inner.lock().await;
        let client = guard
            .as_ref()
            .ok_or_else(|| napi::Error::from_reason("Client is not connected"))?;
        match client.server_info().await {
            Some(info) => Ok(JsServerInfo {
                product_name: info.product_name,
                server_release: info.server_release,
            }),
            None => Ok(JsServerInfo {
                product_name: String::new(),
                server_release: String::new(),
            }),
        }
    }
}
