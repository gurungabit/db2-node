use std::env;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::debug;

use crate::column::ColumnInfo;
use crate::connection::{build_sqldta, ClientInner};
use crate::error::Error;
use crate::types::{QueryResult, ToSql};
use db2_proto::dss::DssWriter;

/// A prepared SQL statement that can be executed multiple times with different parameters.
pub struct PreparedStatement {
    inner: Arc<Mutex<ClientInner>>,
    sql: String,
    package_id: &'static str,
    section_number: u16,
    session_generation: u64,
    column_metadata: Vec<ColumnInfo>,
    result_descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
    param_descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
    prepared: bool,
}

impl PreparedStatement {
    /// Create a new PreparedStatement. Called internally by Client::prepare().
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        inner: Arc<Mutex<ClientInner>>,
        sql: String,
        package_id: &'static str,
        section_number: u16,
        session_generation: u64,
        column_metadata: Vec<ColumnInfo>,
        result_descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
        param_descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
    ) -> Self {
        PreparedStatement {
            inner,
            sql,
            package_id,
            section_number,
            session_generation,
            column_metadata,
            result_descriptors,
            param_descriptors,
            prepared: true,
        }
    }

    /// Execute the prepared statement with the given parameters.
    pub async fn execute(&self, params: &[&dyn ToSql]) -> Result<QueryResult, Error> {
        if !self.prepared {
            return Err(Error::Other("Statement has been closed".into()));
        }

        let mut guard = self.inner.lock().await;
        if guard.session_generation != self.session_generation {
            return Err(Error::Connection(
                "Prepared statement is no longer valid because the connection session changed"
                    .into(),
            ));
        }
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }

        debug!(
            "Executing prepared statement (package={}, section={}): {}",
            self.package_id, self.section_number, self.sql
        );

        let is_query = sql_is_query(&self.sql);
        let pkgnamcsn = guard.build_pkgnamcsn_for(self.package_id, self.section_number);
        guard.activate_section(self.package_id, self.section_number);
        let query_timeout = guard.config.query_timeout;

        if is_query {
            let execute_future = async {
                let corr_id = guard.next_correlation_id();
                let opnqry_data = {
                    let mut ddm = db2_proto::ddm::DdmBuilder::new(db2_proto::codepoints::OPNQRY);
                    ddm.add_code_point(db2_proto::codepoints::PKGNAMCSN, &pkgnamcsn);
                    ddm.add_u32(db2_proto::codepoints::QRYBLKSZ, 0x0000_FFFF);
                    ddm.add_code_point(0x215D, &[0x01]); // QRYCLSIMP = 1
                    ddm.build()
                };
                let sqldta_data = build_sqldta(params, &self.param_descriptors)?;

                let mut writer = DssWriter::new(corr_id);
                writer.write_request_next_same_corr(&opnqry_data, true);
                writer.write_object(&sqldta_data, false);
                let send_buf = writer.finish();
                if env::var_os("DB2_WIRE_DEBUG_HEX").is_some() {
                    eprintln!(
                        "[db2-wire] prepared query send bytes={}",
                        format_hex_preview(&send_buf, 160)
                    );
                }
                guard.send_bytes(&send_buf).await?;

                let frames = guard.read_reply_frames().await?;
                guard
                    .process_query_reply_public(
                        &frames,
                        &self.column_metadata,
                        Some(&self.result_descriptors),
                    )
                    .await
            };

            if query_timeout.is_zero() {
                match execute_future.await {
                    Ok(result) => Ok(result),
                    Err(err) => Err(guard
                        .finalize_operation_error("prepared statement execute", err)
                        .await),
                }
            } else {
                match timeout(query_timeout, execute_future).await {
                    Ok(result) => match result {
                        Ok(result) => Ok(result),
                        Err(err) => Err(guard
                            .finalize_operation_error("prepared statement execute", err)
                            .await),
                    },
                    Err(_) => Err(guard
                        .disconnect_after_timeout("prepared statement execute", query_timeout)
                        .await),
                }
            }
        } else {
            let execute_future = async {
                let corr_id = guard.next_correlation_id();
                let excsqlstt_data = if guard.auto_commit {
                    db2_proto::commands::excsqlstt::build_excsqlstt_autocommit(&pkgnamcsn)
                } else {
                    db2_proto::commands::excsqlstt::build_excsqlstt_default(&pkgnamcsn)
                };
                let sqldta_data = build_sqldta(params, &self.param_descriptors)?;
                let rdbcmm_data = db2_proto::commands::rdbcmm::build_rdbcmm();

                let mut writer = DssWriter::new(corr_id);
                writer.write_request_next_same_corr(&excsqlstt_data, true);
                writer.write_object(&sqldta_data, guard.auto_commit);
                if guard.auto_commit {
                    writer.write_request(&rdbcmm_data, false);
                }
                let send_buf = writer.finish();
                guard.send_bytes(&send_buf).await?;

                let frames = guard.read_execute_reply_frames_public().await?;
                guard.process_execute_reply_public(&frames).await
            };

            if query_timeout.is_zero() {
                match execute_future.await {
                    Ok(result) => Ok(result),
                    Err(err) => Err(guard
                        .finalize_operation_error("prepared statement execute", err)
                        .await),
                }
            } else {
                match timeout(query_timeout, execute_future).await {
                    Ok(result) => match result {
                        Ok(result) => Ok(result),
                        Err(err) => Err(guard
                            .finalize_operation_error("prepared statement execute", err)
                            .await),
                    },
                    Err(_) => Err(guard
                        .disconnect_after_timeout("prepared statement execute", query_timeout)
                        .await),
                }
            }
        }
    }

    /// Execute the prepared statement as a batch with multiple rows of parameters.
    /// This uses DRDA block insert (EXCSQLSTT with NBRROW) for high-throughput inserts.
    pub async fn execute_batch(
        &self,
        param_rows: &[Vec<&dyn ToSql>],
    ) -> Result<QueryResult, Error> {
        if !self.prepared {
            return Err(Error::Other("Statement has been closed".into()));
        }

        let mut guard = self.inner.lock().await;
        if guard.session_generation != self.session_generation {
            return Err(Error::Connection(
                "Prepared statement is no longer valid because the connection session changed"
                    .into(),
            ));
        }
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }

        debug!(
            "Executing batch insert (package={}, section={}, rows={}): {}",
            self.package_id,
            self.section_number,
            param_rows.len(),
            self.sql
        );

        let pkgnamcsn = guard.build_pkgnamcsn_for(self.package_id, self.section_number);
        guard.activate_section(self.package_id, self.section_number);

        let query_timeout = guard.config.query_timeout;
        if query_timeout.is_zero() {
            match guard
                .execute_batch_with_params(&pkgnamcsn, param_rows, &self.param_descriptors)
                .await
            {
                Ok(result) => Ok(result),
                Err(err) => Err(guard
                    .finalize_operation_error("prepared statement batch execute", err)
                    .await),
            }
        } else {
            match timeout(
                query_timeout,
                guard.execute_batch_with_params(&pkgnamcsn, param_rows, &self.param_descriptors),
            )
            .await
            {
                Ok(result) => match result {
                    Ok(result) => Ok(result),
                    Err(err) => Err(guard
                        .finalize_operation_error("prepared statement batch execute", err)
                        .await),
                },
                Err(_) => Err(guard
                    .disconnect_after_timeout("prepared statement batch execute", query_timeout)
                    .await),
            }
        }
    }

    /// Close the prepared statement, releasing server-side resources.
    pub async fn close(mut self) -> Result<(), Error> {
        self.prepared = false;
        let mut guard = self.inner.lock().await;
        if guard.session_generation == self.session_generation {
            guard.release_prepared_section(self.section_number);
        }
        debug!(
            "Closed prepared statement locally (package={}, section={}); the section can now be reused for another prepare on this connection",
            self.package_id,
            self.section_number,
        );
        Ok(())
    }

    /// Get the SQL text of this prepared statement.
    pub fn sql(&self) -> &str {
        &self.sql
    }

    /// Get the column metadata from the prepare response.
    pub fn columns(&self) -> &[ColumnInfo] {
        &self.column_metadata
    }

    /// Get the section number assigned to this statement.
    pub fn section_number(&self) -> u16 {
        self.section_number
    }
}

impl Drop for PreparedStatement {
    fn drop(&mut self) {
        if !self.prepared {
            return;
        }

        let inner = self.inner.clone();
        let session_generation = self.session_generation;
        let section_number = self.section_number;

        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut guard = inner.lock().await;
                if guard.session_generation == session_generation {
                    guard.release_prepared_section(section_number);
                }
            });
        }
    }
}

/// Simple heuristic to determine if a SQL string is a query (SELECT).
fn sql_is_query(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("SELECT")
        || trimmed.starts_with("WITH")
        || trimmed.starts_with("VALUES")
        || trimmed.starts_with("CALL")
}

fn format_hex_preview(data: &[u8], max_bytes: usize) -> String {
    let take = data.len().min(max_bytes);
    let mut out = String::new();
    for (index, byte) in data[..take].iter().enumerate() {
        if index > 0 {
            if index % 16 == 0 {
                out.push_str(" | ");
            } else {
                out.push(' ');
            }
        }
        out.push_str(&format!("{:02X}", byte));
    }
    if data.len() > max_bytes {
        out.push_str(" ...");
    }
    out
}
