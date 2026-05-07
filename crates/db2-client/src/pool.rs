use std::collections::{HashMap, VecDeque};
use std::env;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinSet;
use tokio::time::timeout;
use tracing::{debug, trace, warn};

use crate::config::Config;
use crate::connection::{Client, PoolCheckoutEntry, PoolCheckoutMap};
use crate::error::Error;
use crate::transport::Transport;
use crate::types::{QueryResult, ToSql};

/// Configuration for the connection pool.
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Base connection configuration.
    pub connection: Config,
    /// Minimum number of connections to keep in the pool.
    pub min_connections: u32,
    /// Maximum number of connections the pool will create.
    pub max_connections: u32,
    /// How long an idle connection can sit in the pool before being closed.
    pub idle_timeout: Duration,
    /// Maximum lifetime of a connection before it is recycled.
    pub max_lifetime: Duration,
    /// How long an idle connection can be reused without a round-trip health check.
    pub health_check_interval: Duration,
}

impl PoolConfig {
    /// Create a PoolConfig with sensible defaults.
    pub fn new(connection: Config) -> Self {
        PoolConfig {
            connection,
            min_connections: 1,
            max_connections: 10,
            idle_timeout: Duration::from_secs(600),
            max_lifetime: Duration::from_secs(3600),
            health_check_interval: Duration::from_secs(30),
        }
    }

    /// Set the minimum number of connections.
    pub fn with_min_connections(mut self, min: u32) -> Self {
        self.min_connections = min;
        self
    }

    /// Set the maximum number of connections.
    pub fn with_max_connections(mut self, max: u32) -> Self {
        self.max_connections = max;
        self
    }

    /// Set the idle timeout.
    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = timeout;
        self
    }

    /// Set the max lifetime.
    pub fn with_max_lifetime(mut self, lifetime: Duration) -> Self {
        self.max_lifetime = lifetime;
        self
    }

    /// Set the minimum idle time before a pooled connection is health checked.
    ///
    /// A zero duration preserves the old behavior of checking every checkout.
    pub fn with_health_check_interval(mut self, interval: Duration) -> Self {
        self.health_check_interval = interval;
        self
    }
}

/// A pooled connection wrapping a Client with timing metadata.
struct PooledConnection {
    client: Client,
    created_at: Instant,
    last_used: Instant,
}

/// Outcome from returning a checked-out connection to the pool.
#[derive(Debug, Default, Clone)]
pub struct PoolReleaseOutcome {
    pub disconnected: bool,
    pub replacement_created: bool,
    pub replacement_error: Option<String>,
}

/// A connection pool that manages reusable DB2 connections.
///
/// The pool uses a semaphore to limit the maximum number of concurrent connections
/// and a queue of idle connections for reuse.
pub struct Pool {
    config: PoolConfig,
    connections: Arc<Mutex<VecDeque<PooledConnection>>>,
    checked_out: Arc<Mutex<PoolCheckoutMap>>,
    connection_create_lock: Arc<Mutex<()>>,
    creating_connections: Arc<AtomicUsize>,
    semaphore: Arc<Semaphore>,
}

impl Pool {
    /// Create a new connection pool synchronously without pre-creating connections.
    /// Connections are created lazily on first use.
    pub fn new_sync(config: PoolConfig) -> Self {
        Transport::warm_tls_config(&config.connection);
        Pool {
            semaphore: Arc::new(Semaphore::new(config.max_connections as usize)),
            config,
            connections: Arc::new(Mutex::new(VecDeque::new())),
            checked_out: Arc::new(Mutex::new(HashMap::new())),
            connection_create_lock: Arc::new(Mutex::new(())),
            creating_connections: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Create a new connection pool and establish the minimum number of connections.
    pub async fn new(config: PoolConfig) -> Result<Self, Error> {
        if config.max_connections == 0 {
            return Err(Error::Pool("max_connections must be > 0".into()));
        }
        if config.min_connections > config.max_connections {
            return Err(Error::Pool(
                "min_connections cannot exceed max_connections".into(),
            ));
        }
        Transport::warm_tls_config(&config.connection);

        let pool = Pool {
            config: config.clone(),
            connections: Arc::new(Mutex::new(VecDeque::new())),
            checked_out: Arc::new(Mutex::new(HashMap::new())),
            connection_create_lock: Arc::new(Mutex::new(())),
            creating_connections: Arc::new(AtomicUsize::new(0)),
            semaphore: Arc::new(Semaphore::new(config.max_connections as usize)),
        };

        // Pre-create minimum connections
        for _ in 0..config.min_connections {
            match pool.create_connection().await {
                Ok(client) => {
                    let conn = PooledConnection {
                        client,
                        created_at: Instant::now(),
                        last_used: Instant::now(),
                    };
                    pool.connections.lock().await.push_back(conn);
                }
                Err(e) => {
                    warn!("Failed to create initial pool connection: {}", e);
                    // Don't fail pool creation if initial connections fail
                }
            }
        }

        debug!(
            "Pool created with {}/{} connections",
            pool.connections.lock().await.len(),
            config.max_connections
        );

        Ok(pool)
    }

    /// Execute a query using a connection from the pool.
    pub async fn query(&self, sql: &str, params: &[&dyn ToSql]) -> Result<QueryResult, Error> {
        let client = self.acquire().await?;
        let result = client.query(sql, params).await;

        // Return connection to pool regardless of query result
        self.release(client).await;

        result
    }

    /// Execute a statement with no parameters using a connection from the pool.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult, Error> {
        self.query(sql, &[]).await
    }

    /// Acquire a connection from the pool.
    pub async fn acquire(&self) -> Result<Client, Error> {
        let conn = self.get_connection().await?;
        Ok(conn.client)
    }

    /// Release a connection back into the pool.
    pub async fn release(&self, client: Client) {
        let _ = self.release_with_outcome(client).await;
    }

    /// Release a connection and report whether pool maintenance was needed.
    pub async fn release_with_outcome(&self, client: Client) -> PoolReleaseOutcome {
        let conn = PooledConnection {
            client,
            created_at: Instant::now(), // approximate; ideally tracked from creation
            last_used: Instant::now(),
        };
        self.return_connection(conn).await
    }

    /// Open idle connections up to the configured minimum.
    ///
    /// For pools created with `new_sync`, this removes first-query connection
    /// cost without forcing constructors to block on network I/O.
    pub async fn warmup(&self) -> Result<usize, Error> {
        if self.config.max_connections == 0 {
            return Err(Error::Pool("max_connections must be > 0".into()));
        }

        let _create_guard = self.connection_create_lock.lock().await;
        let target = self.target_idle_count();
        let current = self.total_count().await;
        let to_create = target.saturating_sub(current);

        for _ in 0..to_create {
            let client = self.create_connection().await?;
            let conn = PooledConnection {
                client,
                created_at: Instant::now(),
                last_used: Instant::now(),
            };
            self.connections.lock().await.push_back(conn);
        }

        Ok(to_create)
    }

    /// Open idle connections up to the configured minimum with parallel handshakes.
    ///
    /// This is used for explicit pool warmup/connect calls where the caller is
    /// already waiting for the pool to become ready. Background replacement uses
    /// `warmup` so foreground checkouts can wait on a single in-flight socket.
    pub async fn warmup_parallel(&self) -> Result<usize, Error> {
        if self.config.max_connections == 0 {
            return Err(Error::Pool("max_connections must be > 0".into()));
        }

        let _create_guard = self.connection_create_lock.lock().await;
        let target = self.target_idle_count();
        let current = self.total_count().await + self.creating_count();
        let to_create = target.saturating_sub(current);
        if to_create == 0 {
            return Ok(0);
        }

        let mut tasks = JoinSet::new();
        for _ in 0..to_create {
            let config = self.config.connection.clone();
            let creating_connections = Arc::clone(&self.creating_connections);
            tasks.spawn(async move {
                let _creating = CreatingConnection::new(creating_connections);
                Client::connect_with(config).await
            });
        }

        let mut clients = Vec::with_capacity(to_create);
        let mut first_error = None;
        while let Some(result) = tasks.join_next().await {
            match result {
                Ok(Ok(client)) => clients.push(client),
                Ok(Err(err)) => {
                    first_error.get_or_insert(err);
                }
                Err(err) => {
                    first_error
                        .get_or_insert_with(|| Error::Pool(format!("warmup task failed: {err}")));
                }
            }
        }

        let created = clients.len();
        if created > 0 {
            let mut conns = self.connections.lock().await;
            for client in clients {
                conns.push_back(PooledConnection {
                    client,
                    created_at: Instant::now(),
                    last_used: Instant::now(),
                });
            }
        }

        if let Some(err) = first_error {
            return Err(err);
        }

        Ok(created)
    }

    /// Return whether a background warmup could add an idle connection.
    pub async fn warmup_needed(&self) -> bool {
        !self.semaphore.is_closed()
            && self.total_count().await + self.creating_count() < self.target_idle_count()
    }

    /// Close all connections in the pool.
    ///
    /// Waits up to `drain_timeout` for checked-out connections to be returned
    /// before closing. Idle connections are closed immediately.
    pub async fn close(&self) -> Result<(), Error> {
        self.close_with_timeout(Duration::from_secs(5)).await
    }

    /// Close the pool, waiting up to `drain_timeout` for in-flight connections.
    pub async fn close_with_timeout(&self, drain_timeout: Duration) -> Result<(), Error> {
        // Prevent new acquisitions
        self.semaphore.close();

        // Wait for checked-out connections to return
        let deadline = tokio::time::Instant::now() + drain_timeout;
        loop {
            let checked_out = self.checked_out.lock().await.len();
            if checked_out == 0 {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                warn!(
                    "Pool drain timeout: {} connection(s) still checked out; closing anyway",
                    checked_out
                );
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // Close idle connections
        let mut conns = self.connections.lock().await;
        debug!("Closing pool with {} idle connections", conns.len());
        while let Some(conn) = conns.pop_front() {
            if let Err(e) = conn.client.close().await {
                warn!("Error closing pooled connection: {}", e);
            }
        }

        Ok(())
    }

    /// Get the number of idle connections currently in the pool.
    pub async fn idle_count(&self) -> usize {
        self.connections.lock().await.len()
    }

    /// Get the number of connections currently checked out (in use).
    pub async fn active_count(&self) -> usize {
        self.checked_out.lock().await.len()
    }

    /// Get the total number of connections (idle + active).
    pub async fn total_count(&self) -> usize {
        let idle = self.connections.lock().await.len();
        let active = self.checked_out.lock().await.len();
        idle + active
    }

    /// Get the number of pool connections currently being opened.
    pub fn creating_count(&self) -> usize {
        self.creating_connections.load(Ordering::Relaxed)
    }

    /// Get the configured maximum number of connections.
    pub fn max_connections(&self) -> u32 {
        self.config.max_connections
    }

    /// Create a new connection using the pool's configuration.
    async fn create_connection(&self) -> Result<Client, Error> {
        let _creating = CreatingConnection::new(Arc::clone(&self.creating_connections));
        debug!("Creating new pool connection");
        let config = self.config.connection.clone();
        let client = Client::connect_with(config).await?;
        Ok(client)
    }

    /// Get a connection from the pool, creating one if necessary.
    async fn get_connection(&self) -> Result<PooledConnection, Error> {
        // Try to acquire a permit (limits max concurrent connections)
        let permit = self
            .semaphore
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| Error::Pool("Pool semaphore closed".into()))?;

        if let Some(conn) = self.take_reusable_idle_connection().await {
            trace!("Reusing pooled connection");
            conn.client.attach_pool_checkout(&self.checked_out);
            self.checked_out.lock().await.insert(
                conn.client.pool_key(),
                PoolCheckoutEntry {
                    created_at: conn.created_at,
                    _permit: permit,
                },
            );

            return Ok(PooledConnection {
                client: conn.client,
                created_at: conn.created_at,
                last_used: Instant::now(),
            });
        }

        // No idle connection was available. Serialize connection creation so a
        // foreground checkout can pick up a background warmup connection instead
        // of opening a duplicate socket.
        let _create_guard = self.connection_create_lock.lock().await;
        if let Some(conn) = self.take_reusable_idle_connection().await {
            trace!("Reusing pooled connection created by warmup");
            conn.client.attach_pool_checkout(&self.checked_out);
            self.checked_out.lock().await.insert(
                conn.client.pool_key(),
                PoolCheckoutEntry {
                    created_at: conn.created_at,
                    _permit: permit,
                },
            );

            return Ok(PooledConnection {
                client: conn.client,
                created_at: conn.created_at,
                last_used: Instant::now(),
            });
        }

        // No idle connection is available after waiting for in-flight creation.
        let client = self.create_connection().await?;
        client.attach_pool_checkout(&self.checked_out);
        self.checked_out.lock().await.insert(
            client.pool_key(),
            PoolCheckoutEntry {
                created_at: Instant::now(),
                _permit: permit,
            },
        );

        Ok(PooledConnection {
            client,
            created_at: Instant::now(),
            last_used: Instant::now(),
        })
    }

    async fn take_reusable_idle_connection(&self) -> Option<PooledConnection> {
        loop {
            let maybe_conn = { self.connections.lock().await.pop_back() };
            let conn = maybe_conn?;

            if conn.created_at.elapsed() > self.config.max_lifetime {
                trace!("Discarding expired connection");
                let _ = conn.client.close().await;
                continue;
            }

            if conn.last_used.elapsed() > self.config.idle_timeout {
                trace!("Discarding idle connection");
                let _ = conn.client.close().await;
                continue;
            }

            if !conn.client.is_connected().await {
                trace!("Discarding disconnected pooled connection");
                continue;
            }

            if self.should_health_check(&conn)
                && !Self::health_check(&conn.client, self.health_check_timeout()).await
            {
                trace!("Discarding unhealthy pooled connection");
                let _ = conn.client.close().await;
                continue;
            }

            return Some(conn);
        }
    }

    /// Return a connection to the pool for reuse.
    async fn return_connection(&self, conn: PooledConnection) -> PoolReleaseOutcome {
        let mut outcome = PoolReleaseOutcome::default();
        let checkout = conn.client.detach_pool_checkout().await;
        let created_at = checkout
            .as_ref()
            .map(|entry| entry.created_at)
            .unwrap_or(conn.created_at);

        if checkout.is_none() {
            warn!("Returning a client that is not tracked as checked out from this pool");
        }
        drop(checkout);

        // Check if the connection is still valid
        if !conn.client.is_connected().await {
            trace!("Not returning disconnected connection to pool");
            outcome.disconnected = true;
            match self.replace_disconnected_connection().await {
                Ok(replacement_created) => {
                    outcome.replacement_created = replacement_created;
                }
                Err(err) => {
                    warn!("Failed to replace disconnected pooled connection: {}", err);
                    outcome.replacement_error = Some(err.to_string());
                }
            }
            return outcome;
        }

        // Check lifetime
        if created_at.elapsed() > self.config.max_lifetime {
            trace!("Not returning expired connection to pool");
            let _ = conn.client.close().await;
            return outcome;
        }

        let mut conns = self.connections.lock().await;
        conns.push_back(PooledConnection {
            client: conn.client,
            created_at,
            last_used: Instant::now(),
        });
        outcome
    }

    async fn replace_disconnected_connection(&self) -> Result<bool, Error> {
        if !replace_disconnected_pool_connections() || self.semaphore.is_closed() {
            return Ok(false);
        }

        let _create_guard = self.connection_create_lock.lock().await;
        let target_idle = self.target_idle_count();
        if target_idle == 0 || self.idle_count().await >= target_idle {
            return Ok(false);
        }

        if self.total_count().await >= self.config.max_connections as usize {
            return Ok(false);
        }

        let client = self.create_connection().await?;
        let mut conns = self.connections.lock().await;
        if conns.len() >= target_idle {
            drop(conns);
            let _ = client.close().await;
            return Ok(false);
        }
        conns.push_back(PooledConnection {
            client,
            created_at: Instant::now(),
            last_used: Instant::now(),
        });
        Ok(true)
    }

    /// Perform a basic health check on a connection.
    pub async fn health_check(client: &Client, timeout_duration: Duration) -> bool {
        // Execute a simple query to verify the connection is alive
        matches!(
            timeout(timeout_duration, client.execute("VALUES 1")).await,
            Ok(Ok(_))
        )
    }

    fn health_check_timeout(&self) -> Duration {
        const DEFAULT_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
        let query_timeout = self.config.connection.query_timeout;
        if query_timeout.is_zero() {
            DEFAULT_HEALTH_CHECK_TIMEOUT
        } else {
            query_timeout.min(DEFAULT_HEALTH_CHECK_TIMEOUT)
        }
    }

    fn should_health_check(&self, conn: &PooledConnection) -> bool {
        self.config.health_check_interval.is_zero()
            || conn.last_used.elapsed() >= self.config.health_check_interval
    }

    fn target_idle_count(&self) -> usize {
        self.config
            .min_connections
            .max(1)
            .min(self.config.max_connections) as usize
    }
}

struct CreatingConnection {
    count: Arc<AtomicUsize>,
}

impl CreatingConnection {
    fn new(count: Arc<AtomicUsize>) -> Self {
        count.fetch_add(1, Ordering::Relaxed);
        CreatingConnection { count }
    }
}

impl Drop for CreatingConnection {
    fn drop(&mut self) {
        self.count.fetch_sub(1, Ordering::Relaxed);
    }
}

fn replace_disconnected_pool_connections() -> bool {
    env::var("DB2_POOL_REPLACE_DISCONNECTED")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}
