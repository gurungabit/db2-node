use napi::bindgen_prelude::*;
use napi_derive::napi;
use std::env;
use std::sync::Arc;
use std::time::Instant;

use crate::js_connection::{JsClient, JsQueryResult};
use crate::js_types::{
    client_error_to_napi, config_from_js, emit_napi_diagnostics, js_params_to_db2,
    push_elapsed_diagnostic, query_diagnostics_enabled, query_result_to_js,
};

#[napi(object)]
pub struct JsPoolConfig {
    pub host: String,
    pub port: Option<u32>,
    pub database: String,
    pub user: String,
    pub password: String,
    pub ssl: Option<bool>,
    pub reject_unauthorized: Option<bool>,
    pub ssl_client_hostname_validation: Option<String>,
    pub ca_cert: Option<String>,
    pub security_mechanism: Option<String>,
    pub encryption_algorithm: Option<String>,
    pub credential_encoding: Option<String>,
    pub encrypted_password_encoding: Option<String>,
    pub encrypted_password_token_encoding: Option<String>,
    pub connect_timeout: Option<u32>,
    pub query_timeout: Option<u32>,
    pub frame_drain_timeout: Option<u32>,
    pub current_schema: Option<String>,
    pub type_definition_name: Option<String>,
    pub fetch_size: Option<u32>,
    pub min_connections: Option<u32>,
    pub max_connections: Option<u32>,
    pub idle_timeout: Option<u32>,
    pub max_lifetime: Option<u32>,
    pub health_check_interval: Option<u32>,
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
            config.ssl_client_hostname_validation,
            config.ca_cert,
            config.security_mechanism,
            config.encryption_algorithm,
            config.credential_encoding,
            config.encrypted_password_encoding,
            config.encrypted_password_token_encoding,
            config.connect_timeout,
            config.query_timeout,
            config.frame_drain_timeout,
            config.current_schema.clone(),
            config.type_definition_name.clone(),
            config.fetch_size,
        )?;

        let max_connections = config.max_connections.unwrap_or(10);
        if max_connections == 0 {
            return Err(Error::from_reason("maxConnections must be > 0"));
        }
        let min_connections = config
            .min_connections
            .unwrap_or_else(|| max_connections.min(2));
        if min_connections > max_connections {
            return Err(Error::from_reason(
                "minConnections cannot exceed maxConnections",
            ));
        }

        let pool_config = db2_client::PoolConfig {
            connection: client_config.clone(),
            min_connections,
            max_connections,
            idle_timeout: std::time::Duration::from_secs(config.idle_timeout.unwrap_or(600) as u64),
            max_lifetime: std::time::Duration::from_secs(config.max_lifetime.unwrap_or(3600) as u64),
            health_check_interval: std::time::Duration::from_secs(
                config.health_check_interval.unwrap_or(30) as u64,
            ),
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
    pub async fn connect(&self) -> Result<()> {
        self.inner
            .warmup_parallel()
            .await
            .map_err(client_error_to_napi)?;
        Ok(())
    }

    #[napi]
    pub async fn warmup(&self) -> Result<u32> {
        let created = self
            .inner
            .warmup_parallel()
            .await
            .map_err(client_error_to_napi)?;
        Ok(created as u32)
    }

    #[napi]
    pub async fn query(
        &self,
        sql: String,
        params: Option<Vec<serde_json::Value>>,
    ) -> Result<JsQueryResult> {
        let collect_diagnostics = query_diagnostics_enabled();
        let total_started = collect_diagnostics.then(Instant::now);
        let mut napi_diagnostics = Vec::new();

        let params_started = collect_diagnostics.then(Instant::now);
        let db2_params = match &params {
            Some(p) => js_params_to_db2(p),
            None => Vec::new(),
        };
        push_elapsed_diagnostic(&mut napi_diagnostics, "napi_pool_params_ms", params_started);

        let refs_started = collect_diagnostics.then(Instant::now);
        let param_refs: Vec<&dyn db2_client::ToSql> = db2_params
            .iter()
            .map(|p| p as &dyn db2_client::ToSql)
            .collect();
        push_elapsed_diagnostic(
            &mut napi_diagnostics,
            "napi_pool_param_refs_ms",
            refs_started,
        );

        let acquire_state_before = if collect_diagnostics {
            Some((
                self.inner.idle_count().await,
                self.inner.active_count().await,
                self.inner.creating_count(),
                self.inner.max_connections() as usize,
            ))
        } else {
            None
        };
        let acquire_path = acquire_state_before
            .as_ref()
            .map(|(idle, active, creating, max)| {
                pool_acquire_path(*idle, *active, *creating, *max)
            });
        let acquire_started = collect_diagnostics.then(Instant::now);
        let client = match self.inner.acquire().await {
            Ok(client) => client,
            Err(err) => {
                push_elapsed_diagnostic(
                    &mut napi_diagnostics,
                    "napi_pool_acquire_ms",
                    acquire_started,
                );
                emit_napi_diagnostics(&napi_diagnostics);
                return Err(client_error_to_napi(err));
            }
        };
        push_elapsed_diagnostic(
            &mut napi_diagnostics,
            "napi_pool_acquire_ms",
            acquire_started,
        );
        if collect_diagnostics {
            napi_diagnostics.extend(client.take_connection_diagnostics().await);
        }
        if let Some((idle_before, active_before, creating_before, max_connections)) =
            acquire_state_before
        {
            let idle_after = self.inner.idle_count().await;
            let active_after = self.inner.active_count().await;
            let creating_after = self.inner.creating_count();
            napi_diagnostics.push(format!(
                "napi_pool_acquire_state path={} idle_before={} active_before={} creating_before={} total_before={} idle_after={} active_after={} creating_after={} total_after={} max={}",
                acquire_path.unwrap_or("unknown"),
                idle_before,
                active_before,
                creating_before,
                idle_before + active_before,
                idle_after,
                active_after,
                creating_after,
                idle_after + active_after,
                max_connections
            ));
        }

        let query_started = collect_diagnostics.then(Instant::now);
        let result = client.query(&sql, &param_refs).await;
        push_elapsed_diagnostic(
            &mut napi_diagnostics,
            "napi_pool_client_query_ms",
            query_started,
        );

        let release_started = collect_diagnostics.then(Instant::now);
        let release_outcome = self.inner.release_with_outcome(client).await;
        push_elapsed_diagnostic(
            &mut napi_diagnostics,
            "napi_pool_release_ms",
            release_started,
        );
        let warmup_deferred = defer_background_warmup(&self.inner).await;
        if collect_diagnostics && release_outcome.disconnected {
            let idle_after = self.inner.idle_count().await;
            let active_after = self.inner.active_count().await;
            let replacement_error = release_outcome
                .replacement_error
                .as_deref()
                .unwrap_or("none")
                .replace(' ', "_");
            napi_diagnostics.push(format!(
                "napi_pool_release_state disconnected=true replacement_created={} replacement_deferred={} replacement_error={} idle_after={} active_after={} total_after={}",
                release_outcome.replacement_created,
                warmup_deferred,
                replacement_error,
                idle_after,
                active_after,
                idle_after + active_after
            ));
        } else if collect_diagnostics && warmup_deferred {
            napi_diagnostics.push("napi_pool_background_warmup_deferred=true".to_string());
        }

        let result = match result {
            Ok(result) => result,
            Err(err) => {
                emit_napi_diagnostics(&napi_diagnostics);
                return Err(client_error_to_napi(err));
            }
        };

        let result_prepare_started = collect_diagnostics.then(Instant::now);
        let mut js_result = query_result_to_js(result);
        push_elapsed_diagnostic(
            &mut napi_diagnostics,
            "napi_result_prepare_ms",
            result_prepare_started,
        );
        push_elapsed_diagnostic(
            &mut napi_diagnostics,
            "napi_pool_total_before_return_ms",
            total_started,
        );
        emit_napi_diagnostics(&napi_diagnostics);
        js_result.diagnostics.extend(napi_diagnostics);

        Ok(js_result)
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
            let _ = self.inner.release_with_outcome(client).await;
            let _ = defer_background_warmup(&self.inner).await;
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

fn pool_acquire_path(
    idle: usize,
    active: usize,
    creating: usize,
    max_connections: usize,
) -> &'static str {
    if idle > 0 {
        "idle"
    } else if creating > 0 {
        "warmup"
    } else if active < max_connections {
        "new"
    } else {
        "wait"
    }
}

async fn defer_background_warmup(pool: &Arc<db2_client::Pool>) -> bool {
    if !use_background_pool_warmup() {
        return false;
    }
    if !pool.warmup_needed().await {
        return false;
    }

    let pool = Arc::clone(pool);
    tokio::spawn(async move {
        let _ = pool.warmup().await;
    });
    true
}

fn use_background_pool_warmup() -> bool {
    env::var("DB2_POOL_BACKGROUND_WARMUP")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}
