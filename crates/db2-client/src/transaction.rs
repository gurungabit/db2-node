use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::connection::{ClientInner, PREPARED_STATEMENT_PKGID};
use crate::error::Error;
use crate::types::{QueryResult, ToSql};

/// A database transaction that groups multiple operations into an atomic unit.
///
/// If the Transaction is dropped without calling `commit()` or `rollback()`,
/// a warning is logged. In practice, the server will typically rollback
/// uncommitted work when the next statement executes or the connection closes.
pub struct Transaction {
    inner: Arc<Mutex<ClientInner>>,
    session_generation: u64,
    committed: bool,
    rolled_back: bool,
}

impl Transaction {
    /// Create a new Transaction wrapping the given connection state.
    pub(crate) fn new(inner: Arc<Mutex<ClientInner>>, session_generation: u64) -> Self {
        debug!("Transaction started");
        Transaction {
            inner,
            session_generation,
            committed: false,
            rolled_back: false,
        }
    }

    /// Execute a SQL query or statement within this transaction.
    pub async fn query(&self, sql: &str, params: &[&dyn ToSql]) -> Result<QueryResult, Error> {
        if self.committed {
            return Err(Error::Other("Transaction already committed".into()));
        }
        if self.rolled_back {
            return Err(Error::Other("Transaction already rolled back".into()));
        }

        let mut guard = self.inner.lock().await;
        if guard.session_generation != self.session_generation {
            return Err(Error::Connection(
                "Transaction is no longer valid because the connection session changed".into(),
            ));
        }
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }
        let query_timeout = guard.config.query_timeout;
        if query_timeout.is_zero() {
            match guard.execute_query(sql, params).await {
                Ok(result) => Ok(result),
                Err(err) => Err(guard
                    .finalize_operation_error("transaction query", err)
                    .await),
            }
        } else {
            match timeout(query_timeout, guard.execute_query(sql, params)).await {
                Ok(result) => match result {
                    Ok(result) => Ok(result),
                    Err(err) => Err(guard
                        .finalize_operation_error("transaction query", err)
                        .await),
                },
                Err(_) => Err(guard
                    .disconnect_after_timeout("transaction query", query_timeout)
                    .await),
            }
        }
    }

    /// Execute a SQL statement with no parameters within this transaction.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult, Error> {
        self.query(sql, &[]).await
    }

    /// Prepare a SQL statement within this transaction.
    /// The prepared statement will execute in the transaction's context (auto_commit=false).
    pub async fn prepare(&self, sql: &str) -> Result<crate::statement::PreparedStatement, Error> {
        if self.committed {
            return Err(Error::Other("Transaction already committed".into()));
        }
        if self.rolled_back {
            return Err(Error::Other("Transaction already rolled back".into()));
        }

        let mut guard = self.inner.lock().await;
        if guard.session_generation != self.session_generation {
            return Err(Error::Connection(
                "Transaction is no longer valid because the connection session changed".into(),
            ));
        }
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }

        crate::connection::ensure_sqlstt_sql_len(sql)?;

        let query_timeout = guard.config.query_timeout;
        let prepare_future = async {
            let section_number = guard.allocate_prepared_section()?;
            let corr_id = guard.next_correlation_id();
            let pkgnamcsn = guard.build_pkgnamcsn_for(PREPARED_STATEMENT_PKGID, section_number);

            let prpsqlstt_data =
                db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn);
            let sqlstt_data = db2_proto::commands::sqlstt::build_sqlstt(sql);

            let mut writer = db2_proto::dss::DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&prpsqlstt_data, true);
            writer.write_object(&sqlstt_data, false);

            let send_buf = writer.finish();
            if let Err(err) = guard.send_bytes(&send_buf).await {
                guard.release_prepared_section(section_number);
                return Err(err);
            }

            let frames = match guard.read_reply_frames().await {
                Ok(frames) => frames,
                Err(err) => {
                    guard.release_prepared_section(section_number);
                    return Err(err);
                }
            };
            let column_metadata = match guard.parse_prepare_reply(&frames) {
                Ok(column_metadata) => column_metadata,
                Err(err) => {
                    guard.release_prepared_section(section_number);
                    return Err(err);
                }
            };
            let result_descriptors = guard.parse_prepare_result_descriptors(&frames);
            let input_descriptors = match guard.describe_input(&pkgnamcsn).await {
                Ok(input_descriptors) => input_descriptors,
                Err(err) => {
                    guard.release_prepared_section(section_number);
                    return Err(err);
                }
            };

            Ok::<_, Error>(crate::statement::PreparedStatement::new(
                self.inner.clone(),
                sql.to_string(),
                PREPARED_STATEMENT_PKGID,
                section_number,
                self.session_generation,
                column_metadata,
                result_descriptors,
                input_descriptors,
            ))
        };

        if query_timeout.is_zero() {
            match prepare_future.await {
                Ok(statement) => Ok(statement),
                Err(err) => Err(guard
                    .finalize_operation_error("transaction prepare", err)
                    .await),
            }
        } else {
            match timeout(query_timeout, prepare_future).await {
                Ok(result) => match result {
                    Ok(statement) => Ok(statement),
                    Err(err) => Err(guard
                        .finalize_operation_error("transaction prepare", err)
                        .await),
                },
                Err(_) => Err(guard
                    .disconnect_after_timeout("transaction prepare", query_timeout)
                    .await),
            }
        }
    }

    /// Commit the transaction, making all changes permanent.
    /// Consumes the Transaction.
    pub async fn commit(mut self) -> Result<(), Error> {
        if self.committed {
            return Err(Error::Other("Transaction already committed".into()));
        }
        if self.rolled_back {
            return Err(Error::Other("Transaction already rolled back".into()));
        }

        let mut guard = self.inner.lock().await;
        if guard.session_generation != self.session_generation {
            return Err(Error::Connection(
                "Transaction is no longer valid because the connection session changed".into(),
            ));
        }
        if let Err(err) = guard.commit().await {
            return Err(guard
                .finalize_operation_error("transaction commit", err)
                .await);
        }
        guard.auto_commit = true;
        self.committed = true;
        debug!("Transaction committed");
        Ok(())
    }

    /// Rollback the transaction, undoing all changes.
    /// Consumes the Transaction.
    pub async fn rollback(mut self) -> Result<(), Error> {
        if self.committed {
            return Err(Error::Other("Transaction already committed".into()));
        }
        if self.rolled_back {
            return Err(Error::Other("Transaction already rolled back".into()));
        }

        let mut guard = self.inner.lock().await;
        if guard.session_generation != self.session_generation {
            return Err(Error::Connection(
                "Transaction is no longer valid because the connection session changed".into(),
            ));
        }
        if let Err(err) = guard.rollback().await {
            return Err(guard
                .finalize_operation_error("transaction rollback", err)
                .await);
        }
        guard.auto_commit = true;
        self.rolled_back = true;
        debug!("Transaction rolled back");
        Ok(())
    }

    /// Check if this transaction has been committed.
    pub fn is_committed(&self) -> bool {
        self.committed
    }

    /// Check if this transaction has been rolled back.
    pub fn is_rolled_back(&self) -> bool {
        self.rolled_back
    }

    /// Check if this transaction is still active (not committed or rolled back).
    pub fn is_active(&self) -> bool {
        !self.committed && !self.rolled_back
    }
}

impl Drop for Transaction {
    fn drop(&mut self) {
        if !self.committed && !self.rolled_back {
            let inner = self.inner.clone();
            let session_generation = self.session_generation;
            if let Ok(handle) = tokio::runtime::Handle::try_current() {
                handle.spawn(async move {
                    let mut guard = inner.lock().await;
                    if guard.session_generation != session_generation {
                        return;
                    }

                    if !guard.connected {
                        guard.auto_commit = true;
                        return;
                    }

                    if guard.auto_commit {
                        return;
                    }

                    if let Err(err) = guard.rollback().await {
                        warn!("Transaction dropped without commit or rollback, and rollback failed: {}", err);
                    } else {
                        debug!("Rolled back dropped transaction");
                    }
                });
            } else {
                warn!(
                    "Transaction dropped without commit or rollback and no Tokio runtime was available for cleanup"
                );
            }
        }
    }
}
