use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, Semaphore};
use tokio::time::timeout;
use tracing::{debug, trace, warn};

use crate::config::Config;
use crate::connection::{Client, PoolCheckoutEntry, PoolCheckoutMap};
use crate::error::Error;
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
}

/// A pooled connection wrapping a Client with timing metadata.
struct PooledConnection {
    client: Client,
    created_at: Instant,
    last_used: Instant,
}

/// A connection pool that manages reusable DB2 connections.
///
/// The pool uses a semaphore to limit the maximum number of concurrent connections
/// and a queue of idle connections for reuse.
pub struct Pool {
    config: PoolConfig,
    connections: Arc<Mutex<VecDeque<PooledConnection>>>,
    checked_out: Arc<Mutex<PoolCheckoutMap>>,
    semaphore: Arc<Semaphore>,
}

impl Pool {
    /// Create a new connection pool synchronously without pre-creating connections.
    /// Connections are created lazily on first use.
    pub fn new_sync(config: PoolConfig) -> Self {
        Pool {
            semaphore: Arc::new(Semaphore::new(config.max_connections as usize)),
            config,
            connections: Arc::new(Mutex::new(VecDeque::new())),
            checked_out: Arc::new(Mutex::new(HashMap::new())),
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

        let pool = Pool {
            config: config.clone(),
            connections: Arc::new(Mutex::new(VecDeque::new())),
            checked_out: Arc::new(Mutex::new(HashMap::new())),
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
        let conn = PooledConnection {
            client,
            created_at: Instant::now(), // approximate; ideally tracked from creation
            last_used: Instant::now(),
        };
        self.return_connection(conn).await;
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

    /// Get the configured maximum number of connections.
    pub fn max_connections(&self) -> u32 {
        self.config.max_connections
    }

    /// Create a new connection using the pool's configuration.
    async fn create_connection(&self) -> Result<Client, Error> {
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

        // Try to reuse an idle connection.
        loop {
            let maybe_conn = { self.connections.lock().await.pop_front() };
            let Some(conn) = maybe_conn else {
                break;
            };

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

            if !Self::health_check(&conn.client, self.health_check_timeout()).await {
                trace!("Discarding unhealthy pooled connection");
                let _ = conn.client.close().await;
                continue;
            }

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

        // No idle connections available - create a new one
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

    /// Return a connection to the pool for reuse.
    async fn return_connection(&self, conn: PooledConnection) {
        let checkout = conn.client.detach_pool_checkout().await;
        let created_at = checkout
            .as_ref()
            .map(|entry| entry.created_at)
            .unwrap_or(conn.created_at);

        if checkout.is_none() {
            warn!("Returning a client that is not tracked as checked out from this pool");
        }

        // Check if the connection is still valid
        if !conn.client.is_connected().await {
            trace!("Not returning disconnected connection to pool");
            return;
        }

        // Check lifetime
        if created_at.elapsed() > self.config.max_lifetime {
            trace!("Not returning expired connection to pool");
            let _ = conn.client.close().await;
            return;
        }

        let mut conns = self.connections.lock().await;
        conns.push_back(PooledConnection {
            client: conn.client,
            created_at,
            last_used: Instant::now(),
        });
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
}
