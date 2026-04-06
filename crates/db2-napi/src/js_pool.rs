use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::sync::Arc;

use crate::js_connection::{JsClient, JsQueryResult};
use crate::js_types::{client_error_to_napi, config_from_js, js_params_to_db2, query_result_to_js};

#[napi(object)]
pub struct JsPoolConfig {
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
    pub min_connections: Option<u32>,
    pub max_connections: Option<u32>,
    pub idle_timeout: Option<u32>,
    pub max_lifetime: Option<u32>,
}

#[napi]
pub struct JsPool {
    inner: Arc<db2_client::Pool>,
    config: db2_client::Config,
}

#[napi]
impl JsPool {
    #[napi(constructor)]
    pub fn new(config: JsPoolConfig) -> Result<Self> {
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
            config.current_schema.clone(),
            config.fetch_size,
        );

        let pool_config = db2_client::PoolConfig {
            connection: client_config.clone(),
            min_connections: config.min_connections.unwrap_or(0),
            max_connections: config.max_connections.unwrap_or(10),
            idle_timeout: std::time::Duration::from_secs(config.idle_timeout.unwrap_or(600) as u64),
            max_lifetime: std::time::Duration::from_secs(config.max_lifetime.unwrap_or(3600) as u64),
        };

        // Pool::new is async in the client crate, but napi constructors are sync.
        // We create the pool struct synchronously and defer actual connection creation.
        let pool = db2_client::Pool::new_sync(pool_config);

        Ok(JsPool {
            inner: Arc::new(pool),
            config: client_config,
        })
    }

    #[napi]
    pub async fn query(
        &self,
        sql: String,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<JsQueryResult> {
        let db2_params = match &params {
            Some(p) => js_params_to_db2(p),
            None => Vec::new(),
        };

        let param_refs: Vec<&dyn db2_client::ToSql> = db2_params
            .iter()
            .map(|p| p as &dyn db2_client::ToSql)
            .collect();

        let result = self
            .inner
            .query(&sql, &param_refs)
            .await
            .map_err(client_error_to_napi)?;

        Ok(query_result_to_js(result))
    }

    #[napi]
    pub async fn acquire(&self) -> Result<JsClient> {
        let client = self.inner.acquire().await.map_err(client_error_to_napi)?;
        Ok(JsClient::from_inner(client, self.config.clone()))
    }

    #[napi]
    pub async fn release(&self, client: &JsClient) -> Result<()> {
        let mut guard = client.inner.lock().await;
        if let Some(client) = guard.take() {
            self.inner.release(client).await;
        }
        Ok(())
    }

    #[napi]
    pub async fn close(&self) -> Result<()> {
        self.inner.close().await.map_err(client_error_to_napi)?;
        Ok(())
    }

    #[napi(js_name = "idleCount")]
    pub async fn idle_count(&self) -> Result<u32> {
        Ok(self.inner.idle_count().await as u32)
    }

    #[napi(js_name = "activeCount")]
    pub async fn active_count(&self) -> Result<u32> {
        Ok(self.inner.active_count().await as u32)
    }

    #[napi(js_name = "totalCount")]
    pub async fn total_count(&self) -> Result<u32> {
        Ok(self.inner.total_count().await as u32)
    }

    #[napi(js_name = "maxConnections")]
    pub fn max_connections(&self) -> u32 {
        self.inner.max_connections()
    }
}
