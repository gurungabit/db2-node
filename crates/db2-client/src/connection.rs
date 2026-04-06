use bytes::BytesMut;
use std::collections::HashMap;
use std::env;
use std::sync::{Arc, Mutex as StdMutex, Weak};
use std::time::Duration;
use tokio::sync::{Mutex, OwnedSemaphorePermit};
use tokio::time::timeout;
use tracing::{debug, trace};

use crate::auth::{self, ServerInfo};
use crate::column::ColumnInfo;
use crate::config::Config;
use crate::cursor::Cursor;
use crate::error::Error;
use crate::row::Row;
use crate::transport::Transport;
use crate::types::{QueryResult, ToSql};
use db2_proto::codepoints;
use db2_proto::ddm::DdmObject;
use db2_proto::dss::{DssFrame, DssReader, DssWriter};

pub(crate) const DIRECT_QUERY_PKGID: &str = db2_proto::commands::DEFAULT_PKGID;
pub(crate) const DIRECT_QUERY_SECTION: u16 = 65;
// DB2 CLI binds large placeholder packages as SYSLHxyy. Using the first one gives
// long-lived prepared statements their own section space instead of colliding with
// the one-shot section we keep for direct query()/execute() calls.
pub(crate) const PREPARED_STATEMENT_PKGID: &str = "SYSLH200";
pub(crate) const PREPARED_STATEMENT_MAX_SECTION: u16 = 385;

pub(crate) struct PoolCheckoutEntry {
    pub(crate) created_at: std::time::Instant,
    pub(crate) _permit: OwnedSemaphorePermit,
}

pub(crate) type PoolCheckoutMap = HashMap<usize, PoolCheckoutEntry>;

struct PoolCheckoutHandle {
    key: usize,
    checked_out: Weak<Mutex<PoolCheckoutMap>>,
}

/// Internal shared state for a DB2 connection.
pub(crate) struct ClientInner {
    pub transport: Option<Transport>,
    pub config: Config,
    pub server_info: Option<ServerInfo>,
    pub correlation_id: u16,
    pub section_number: u16,
    pub package_id: &'static str,
    pub auto_commit: bool,
    pub connected: bool,
    pub connected_once: bool,
    pub closed_explicitly: bool,
    pub session_generation: u64,
    pub recv_buf: BytesMut,
    pub next_prepared_section: u16,
    pub free_prepared_sections: Vec<u16>,
}

impl ClientInner {
    /// Get the next correlation ID.
    /// DB2 LUW treats correlation IDs as signed 16-bit values, so we
    /// wrap at 0x7FFF (32767) back to 1 to avoid negative values.
    pub fn next_correlation_id(&mut self) -> u16 {
        let id = self.correlation_id;
        self.correlation_id = self.correlation_id.wrapping_add(1);
        if self.correlation_id == 0 || self.correlation_id > 0x7FFF {
            self.correlation_id = 1;
        }
        id
    }

    pub fn activate_section(&mut self, package_id: &'static str, section_number: u16) {
        self.package_id = package_id;
        self.section_number = section_number;
    }

    pub fn direct_query_pkgnamcsn(&mut self) -> Vec<u8> {
        self.activate_section(DIRECT_QUERY_PKGID, DIRECT_QUERY_SECTION);
        self.build_pkgnamcsn_for(DIRECT_QUERY_PKGID, DIRECT_QUERY_SECTION)
    }

    pub fn build_pkgnamcsn_for(&self, package_id: &str, section_number: u16) -> Vec<u8> {
        db2_proto::commands::build_pkgnamcsn(
            &self.config.database,
            db2_proto::commands::DEFAULT_RDBCOLID,
            package_id,
            &db2_proto::commands::DEFAULT_PKGCNSTKN,
            section_number,
        )
    }

    pub fn allocate_prepared_section(&mut self) -> Result<u16, Error> {
        if let Some(section_number) = self.free_prepared_sections.pop() {
            return Ok(section_number);
        }

        if self.next_prepared_section > PREPARED_STATEMENT_MAX_SECTION {
            return Err(Error::Other(format!(
                "Too many prepared statements are open on this connection; package '{}' supports {} concurrent sections",
                PREPARED_STATEMENT_PKGID,
                PREPARED_STATEMENT_MAX_SECTION
            )));
        }

        let section_number = self.next_prepared_section;
        self.next_prepared_section += 1;
        Ok(section_number)
    }

    pub fn release_prepared_section(&mut self, section_number: u16) {
        if section_number == 0 || section_number > PREPARED_STATEMENT_MAX_SECTION {
            return;
        }
        if !self.free_prepared_sections.contains(&section_number) {
            self.free_prepared_sections.push(section_number);
        }
    }

    /// Send raw bytes over the transport.
    pub async fn send_bytes(&mut self, data: &[u8]) -> Result<(), Error> {
        let transport = self
            .transport
            .as_mut()
            .ok_or_else(|| Error::Connection("Transport not initialized".into()))?;
        transport.write_bytes(data).await
    }

    /// Read DSS frames from the transport, waiting for at least `min_frames` frames.
    pub async fn read_frames(&mut self, min_frames: usize) -> Result<Vec<DssFrame>, Error> {
        let transport = self
            .transport
            .as_mut()
            .ok_or_else(|| Error::Connection("Transport not initialized".into()))?;

        // Ensure we have enough data
        if self.recv_buf.len() < 6 {
            transport.read_at_least(&mut self.recv_buf, 6).await?;
        }

        loop {
            let mut reader = DssReader::new(self.recv_buf.to_vec());
            let frames = match reader.read_all_frames() {
                Ok(frames) => frames,
                Err(e) => {
                    if debug_hex_enabled() {
                        eprintln!(
                            "[db2-wire] DSS parse error with {} buffered bytes: {}",
                            self.recv_buf.len(),
                            e
                        );
                        let _ = std::fs::write("/tmp/db2-wire-recv.bin", &self.recv_buf);
                        eprintln!(
                            "[db2-wire] recv_buf preview: {}",
                            format_hex_preview(&self.recv_buf, 256)
                        );
                    }
                    return Err(Error::Protocol(e.to_string()));
                }
            };
            if frames.len() >= min_frames {
                let remaining = reader.into_remaining();
                self.recv_buf = BytesMut::from(remaining.as_slice());
                return Ok(frames);
            }
            transport.read_bytes(&mut self.recv_buf).await?;
        }
    }

    /// Read all available DSS frames (at least 1).
    pub async fn read_reply_frames(&mut self) -> Result<Vec<DssFrame>, Error> {
        self.read_frames(1).await
    }

    fn frame_drain_timeout(&self) -> Duration {
        self.config.frame_drain_timeout
    }

    fn should_auto_reconnect(&self) -> bool {
        !self.connected && self.connected_once && !self.closed_explicitly
    }

    async fn reset_session_state(&mut self, explicit_close: bool) {
        self.connected = false;
        self.auto_commit = true;
        self.closed_explicitly = explicit_close;
        self.server_info = None;
        self.section_number = DIRECT_QUERY_SECTION;
        self.package_id = DIRECT_QUERY_PKGID;
        self.recv_buf.clear();
        self.next_prepared_section = 1;
        self.free_prepared_sections.clear();

        if let Some(mut transport) = self.transport.take() {
            let _ = transport.close().await;
        }
    }

    async fn reconnect_if_needed(&mut self, operation: &str) -> Result<(), Error> {
        if !self.should_auto_reconnect() {
            return Ok(());
        }

        debug!("Reconnecting before {}", operation);
        self.establish_session().await
    }

    pub async fn disconnect_after_timeout(
        &mut self,
        operation: &str,
        timeout_duration: Duration,
    ) -> Error {
        self.reset_session_state(false).await;

        Error::Timeout(format!(
            "{} timed out after {:?}; connection was closed to avoid protocol desynchronization",
            operation, timeout_duration
        ))
    }

    pub async fn finalize_operation_error(&mut self, operation: &str, err: Error) -> Error {
        if matches!(
            err,
            Error::Connection(_) | Error::Io(_) | Error::Protocol(_) | Error::Tls(_)
        ) {
            debug!(
                "{} failed with a fatal connection/session error; resetting connection state",
                operation
            );
            self.reset_session_state(false).await;
        }

        err
    }

    /// Read an execute reply and drain any chained commit frames that arrive immediately after.
    async fn read_execute_reply_frames(&mut self) -> Result<Vec<DssFrame>, Error> {
        let mut frames = self.read_reply_frames().await?;
        let frame_drain_timeout = self.frame_drain_timeout();

        loop {
            let more_frames = match timeout(frame_drain_timeout, self.read_reply_frames()).await {
                Ok(Ok(frames)) => frames,
                Ok(Err(err)) => return Err(err),
                Err(_) => break,
            };

            if more_frames.is_empty() {
                break;
            }

            if debug_hex_enabled() {
                eprintln!(
                    "[db2-wire] drained {} additional execute frame(s)",
                    more_frames.len()
                );
            }

            frames.extend(more_frames);
        }

        Ok(frames)
    }

    /// Parse a DDM object from a DSS frame payload.
    pub fn parse_ddm(payload: &[u8]) -> Result<DdmObject, Error> {
        let (obj, _) = DdmObject::parse(payload).map_err(|e| Error::Protocol(e.to_string()))?;
        Ok(obj)
    }

    /// Parse all DDM objects from a DSS frame payload.
    pub fn parse_ddm_objects(payload: &[u8]) -> Result<Vec<DdmObject>, Error> {
        db2_proto::ddm::parse_ddm_objects(payload).map_err(|e| Error::Protocol(e.to_string()))
    }

    /// Execute an SQL statement immediately (no parameters).
    pub async fn execute_immediate(&mut self, sql: &str) -> Result<QueryResult, Error> {
        ensure_sqlstt_sql_len(sql)?;
        debug!("Execute immediate: {}", sql);

        let corr_id = self.next_correlation_id();
        let pkgnamcsn = self.direct_query_pkgnamcsn();
        // Use EXCSQLIMM (0x200A) for non-query SQL execution
        let exec_data = if self.auto_commit {
            db2_proto::commands::excsqlimm::build_excsqlimm_autocommit(&pkgnamcsn)
        } else {
            db2_proto::commands::excsqlimm::build_excsqlimm_default(&pkgnamcsn)
        };
        let sqlstt_data = db2_proto::commands::sqlstt::build_sqlstt(sql);
        let rdbcmm_data = db2_proto::commands::rdbcmm::build_rdbcmm();

        // EXCSQLIMM + SQLSTT
        let mut writer = DssWriter::new(corr_id);
        writer.write_request_next_same_corr(&exec_data, true);
        writer.write_object(&sqlstt_data, self.auto_commit);
        if self.auto_commit {
            writer.write_request(&rdbcmm_data, false);
        }

        let send_buf = writer.finish();
        if debug_hex_enabled() {
            eprintln!(
                "[db2-wire] execute_immediate send bytes={}",
                format_hex_preview(&send_buf, 192)
            );
        }
        self.send_bytes(&send_buf).await?;

        // Read reply frames
        let frames = self.read_execute_reply_frames().await?;

        // Check if this is a query that returns rows
        let has_query_data = frames.iter().any(|f| {
            if let Ok(objects) = Self::parse_ddm_objects(&f.payload) {
                objects.iter().any(|obj| {
                    matches!(
                        obj.code_point,
                        codepoints::OPNQRYRM | codepoints::QRYDSC | codepoints::QRYDTA
                    )
                })
            } else {
                false
            }
        });

        if has_query_data {
            // Parse as query result with rows
            // Extract column info from SQLDARD if present, otherwise use empty
            let column_info = self.parse_prepare_reply(&frames).unwrap_or_default();
            self.process_query_reply(&frames, &column_info, None).await
        } else {
            self.process_execute_reply(&frames).await
        }
    }

    /// Post-auth initialization: second EXCSAT + SET CLIENT + COMMIT
    /// This matches pydrda's connection flow and initializes the package context.
    /// Post-auth initialization matching pydrda's flow.
    /// Sends: EXCSAT(XAMGR, chained) + EXCSQLSET(chained) + SQLSTT(SET CLIENT, chained) + SQLSTT(SET LOCALE) + RDBCMM
    async fn post_auth_init(&mut self) -> Result<(), Error> {
        // Second EXCSAT with XAMGR=1208
        let mut excsat2 = db2_proto::ddm::DdmBuilder::new(codepoints::EXCSAT);
        let mut mgr = Vec::new();
        mgr.extend_from_slice(&codepoints::XAMGR.to_be_bytes());
        mgr.extend_from_slice(&1208u16.to_be_bytes());
        excsat2.add_code_point(codepoints::MGRLVLLS, &mgr);
        let excsat2_bytes = excsat2.build();

        // EXCSQLSET with 0x01*8 token and section 1 (NO RDBCMTOK!)
        let pkgnamcsn = db2_proto::commands::build_pkgnamcsn(
            &self.config.database,
            db2_proto::commands::DEFAULT_RDBCOLID,
            db2_proto::commands::DEFAULT_PKGID,
            &db2_proto::commands::PKGCNSTKN_EXCSQLSET,
            1,
        );
        let mut excsqlset = db2_proto::ddm::DdmBuilder::new(codepoints::EXCSQLSET);
        excsqlset.add_code_point(codepoints::PKGNAMCSN, &pkgnamcsn);
        let excsqlset_bytes = excsqlset.build();

        let sqlstt1 = db2_proto::commands::sqlstt::build_sqlstt("SET CLIENT WRKSTNNAME 'db2wire'");
        let sqlstt2 =
            db2_proto::commands::sqlstt::build_sqlstt("SET CURRENT LOCALE LC_CTYPE='en_US'");
        let rdbcmm = db2_proto::commands::rdbcmm::build_rdbcmm();

        let corr1 = self.next_correlation_id();
        let corr2 = self.next_correlation_id();
        let corr3 = self.next_correlation_id();

        let mut writer = DssWriter::new(corr1);
        writer.write_request(&excsat2_bytes, true); // EXCSAT chained
        writer.set_correlation_id(corr2);
        writer.write_request_next_same_corr(&excsqlset_bytes, true); // EXCSQLSET chained+samecorr
        writer.write_object_same_corr(&sqlstt1, true); // SQLSTT chained+samecorr
        writer.write_object(&sqlstt2, true); // SQLSTT chained — RDBCMM follows
        writer.set_correlation_id(corr3);
        writer.write_request(&rdbcmm, false); // RDBCMM

        let send_buf = writer.finish();
        self.send_bytes(&send_buf).await?;

        // Read at least 1 frame
        let _frames = self.read_frames(1).await?;
        debug!("Post-auth init complete");
        Ok(())
    }

    async fn establish_session(&mut self) -> Result<(), Error> {
        let mut transport = Transport::connect(&self.config).await?;

        let (server_info, next_corr_id) =
            match auth::authenticate(&mut transport, &self.config).await {
                Ok(result) => result,
                Err(Error::Connection(msg)) if msg.to_lowercase().contains("closed by server") => {
                    return Err(Error::Connection(
                        "RDB not accessed or database not found".into(),
                    ));
                }
                Err(err) => return Err(err),
            };

        self.transport = Some(transport);
        self.server_info = Some(server_info);
        self.correlation_id = next_corr_id;
        self.section_number = DIRECT_QUERY_SECTION;
        self.package_id = DIRECT_QUERY_PKGID;
        self.auto_commit = true;
        self.connected = true;
        self.connected_once = true;
        self.closed_explicitly = false;
        self.recv_buf.clear();
        self.next_prepared_section = 1;
        self.free_prepared_sections.clear();
        self.session_generation = self.session_generation.wrapping_add(1);
        if self.session_generation == 0 {
            self.session_generation = 1;
        }

        if let Err(err) = self.post_auth_init().await {
            self.reset_session_state(false).await;
            return Err(err);
        }

        debug!("Client connected to DB2 server");
        Ok(())
    }

    /// Execute a SET command via EXCSQLSET (code point 0x2014).
    #[allow(dead_code)]
    async fn execute_set(&mut self, sql: &str) -> Result<(), Error> {
        let corr_id = self.next_correlation_id();
        // EXCSQLSET uses 0x01*8 token and section 1 (matching pydrda)
        let pkgnamcsn = db2_proto::commands::build_pkgnamcsn(
            &self.config.database,
            db2_proto::commands::DEFAULT_RDBCOLID,
            db2_proto::commands::DEFAULT_PKGID,
            &db2_proto::commands::PKGCNSTKN_EXCSQLSET,
            1, // section 1 for EXCSQLSET
        );
        let exec_data = {
            let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::EXCSQLSET);
            ddm.add_code_point(codepoints::PKGNAMCSN, &pkgnamcsn);
            // Note: EXCSQLSET does NOT take RDBCMTOK
            ddm.build()
        };
        let sqlstt_data = db2_proto::commands::sqlstt::build_sqlstt(sql);

        let mut writer = DssWriter::new(corr_id);
        writer.write_request_next_same_corr(&exec_data, true);
        writer.write_object_same_corr(&sqlstt_data, false);

        let send_buf = writer.finish();
        self.send_bytes(&send_buf).await?;

        let frames = self.read_reply_frames().await?;
        // Just check for errors
        for frame in &frames {
            let obj = Self::parse_ddm(&frame.payload)?;
            if obj.code_point == codepoints::SQLCARD {
                let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if card.is_error() {
                    return Err(Error::Sql {
                        sqlstate: card.sqlstate,
                        sqlcode: card.sqlcode,
                        message: card.sqlerrmc,
                    });
                }
            }
        }
        Ok(())
    }

    /// Execute a query with parameters.
    pub async fn execute_query(
        &mut self,
        sql: &str,
        params: &[&dyn ToSql],
    ) -> Result<QueryResult, Error> {
        ensure_sqlstt_sql_len(sql)?;
        if params.is_empty() && !sql_is_query(sql) {
            return self.execute_immediate(sql).await;
        }

        debug!("Execute query with {} params: {}", params.len(), sql);

        let is_query = sql_is_query(sql);
        let pkgnamcsn = self.direct_query_pkgnamcsn();
        let mut input_descriptors = Vec::new();

        if is_query {
            // Prepare first, then open the query explicitly. This is a little
            // more chatty than chaining OPNQRY onto the prepare request, but it
            // is much more reliable for large multi-segment SQLSTT payloads.
            let corr_id = self.next_correlation_id();
            let prpsqlstt_data =
                db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn);
            let sqlstt_data = db2_proto::commands::sqlstt::build_sqlstt(sql);
            let qryblksz: u32 = 0x0000FFFF;
            let mut writer = DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&prpsqlstt_data, true);
            writer.write_object(&sqlstt_data, false);

            let send_buf = writer.finish();
            self.send_bytes(&send_buf).await?;

            let frames = self.read_reply_frames().await?;
            let column_info = self.parse_prepare_reply(&frames)?;
            let result_descriptors = self.parse_prepare_result_descriptors(&frames);

            if params.is_empty() {
                let corr_id = self.next_correlation_id();
                let opnqry_data = {
                    let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::OPNQRY);
                    ddm.add_code_point(codepoints::PKGNAMCSN, &pkgnamcsn);
                    ddm.add_u32(codepoints::QRYBLKSZ, qryblksz);
                    ddm.add_u16(codepoints::MAXBLKEXT, qryblksz as u16);
                    ddm.add_code_point(0x215D, &[0x01]); // QRYCLSIMP = 1 (close on endqry)
                    ddm.build()
                };

                let mut writer = DssWriter::new(corr_id);
                writer.write_request(&opnqry_data, false);
                let send_buf = writer.finish();
                self.send_bytes(&send_buf).await?;

                let frames = self.read_reply_frames().await?;
                return self
                    .process_query_reply(&frames, &column_info, Some(&result_descriptors))
                    .await;
            }

            input_descriptors = self.describe_input(&pkgnamcsn).await?;
            let sqldta_data = build_sqldta(params, &input_descriptors)?;
            let corr_id = self.next_correlation_id();
            let opnqry_data = {
                let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::OPNQRY);
                ddm.add_code_point(codepoints::PKGNAMCSN, &pkgnamcsn);
                ddm.add_u32(codepoints::QRYBLKSZ, qryblksz);
                ddm.add_code_point(0x215D, &[0x01]); // QRYCLSIMP = 1 (close on endqry)
                ddm.build()
            };

            let mut writer = DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&opnqry_data, true);
            writer.write_object(&sqldta_data, false);
            let send_buf = writer.finish();
            self.send_bytes(&send_buf).await?;

            let frames = self.read_frames(2).await?;
            self.process_query_reply(&frames, &column_info, Some(&result_descriptors))
                .await
        } else {
            // For DML: PRPSQLSTT + SQLSTT first, then EXCSQLSTT + SQLDTA
            let corr_id = self.next_correlation_id();
            let prpsqlstt_data =
                db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn);
            let sqlstt_data = db2_proto::commands::sqlstt::build_sqlstt(sql);

            let mut writer = DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&prpsqlstt_data, true);
            writer.write_object(&sqlstt_data, false);

            let send_buf = writer.finish();
            self.send_bytes(&send_buf).await?;

            let frames = self.read_reply_frames().await?;
            let _column_info = self.parse_prepare_reply(&frames)?;
            if !params.is_empty() {
                input_descriptors = self.describe_input(&pkgnamcsn).await?;
            }

            self.execute_with_params(&pkgnamcsn, params, &input_descriptors)
                .await
        }
    }

    /// Execute a DML statement with parameters.
    async fn execute_with_params(
        &mut self,
        pkgnamcsn: &[u8],
        params: &[&dyn ToSql],
        descriptors: &[db2_proto::fdoca::ColumnDescriptor],
    ) -> Result<QueryResult, Error> {
        let corr_id = self.next_correlation_id();
        let excsqlstt_data = if self.auto_commit {
            db2_proto::commands::excsqlstt::build_excsqlstt_autocommit(pkgnamcsn)
        } else {
            db2_proto::commands::excsqlstt::build_excsqlstt_default(pkgnamcsn)
        };
        let sqldta_data = build_sqldta(params, descriptors)?;
        let rdbcmm_data = db2_proto::commands::rdbcmm::build_rdbcmm();

        let mut writer = DssWriter::new(corr_id);
        writer.write_request_next_same_corr(&excsqlstt_data, true);
        writer.write_object(&sqldta_data, self.auto_commit);
        if self.auto_commit {
            writer.write_request(&rdbcmm_data, false);
        }

        let send_buf = writer.finish();
        if debug_hex_enabled() {
            eprintln!(
                "[db2-wire] execute_with_params send bytes={}",
                format_hex_preview(&send_buf, 192)
            );
        }
        self.send_bytes(&send_buf).await?;

        let frames = self.read_execute_reply_frames().await?;
        self.process_execute_reply(&frames).await
    }

    /// Execute a batch of rows using pipelined EXCSQLSTT+SQLDTA commands.
    /// Commands are sent in micro-batches (pipeline chunks) and replies
    /// are read back. This eliminates both SQL text overhead and
    /// per-row network round-trip latency.
    pub async fn execute_batch_with_params(
        &mut self,
        pkgnamcsn: &[u8],
        param_rows: &[Vec<&dyn ToSql>],
        descriptors: &[db2_proto::fdoca::ColumnDescriptor],
    ) -> Result<QueryResult, Error> {
        if param_rows.is_empty() {
            return Ok(QueryResult {
                rows: Vec::new(),
                columns: Vec::new(),
                row_count: 0,
            });
        }

        // Pipeline chunk size — how many commands per TCP write/read cycle.
        const PIPELINE_CHUNK: usize = 500;

        let mut total_row_count: i64 = 0;

        for chunk in param_rows.chunks(PIPELINE_CHUNK) {
            let chunk_len = chunk.len();
            let mut send_buf = Vec::with_capacity(chunk_len * 100);

            for (i, row) in chunk.iter().enumerate() {
                let is_last = i == chunk_len - 1;
                let corr_id = self.next_correlation_id();

                let excsqlstt_data =
                    db2_proto::commands::excsqlstt::build_excsqlstt_default(pkgnamcsn);
                let sqldta_data = build_sqldta(row, descriptors)?;

                let mut writer = DssWriter::new(corr_id);
                writer.write_request_next_same_corr(&excsqlstt_data, true);
                writer.write_object(&sqldta_data, !is_last);
                send_buf.extend_from_slice(&writer.finish());
            }

            self.send_bytes(&send_buf).await?;

            // Read reply frames until we've seen SQLCARD for each row in the chunk.
            let mut sqlcards_seen = 0;
            while sqlcards_seen < chunk_len {
                let frames = self.read_reply_frames().await?;
                for frame in &frames {
                    for obj in Self::parse_ddm_objects(&frame.payload)? {
                        if let Some(err) = protocol_reply_error(&obj, "batch execute") {
                            return Err(err);
                        }
                        match obj.code_point {
                            codepoints::SQLCARD => {
                                let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                                    .map_err(|e| Error::Protocol(e.to_string()))?;
                                if card.is_error() {
                                    return Err(Error::Sql {
                                        sqlstate: card.sqlstate,
                                        sqlcode: card.sqlcode,
                                        message: if card.sqlerrmc.is_empty() {
                                            format!(
                                                "SQL error in batch row {}: SQLCODE={}",
                                                sqlcards_seen, card.sqlcode
                                            )
                                        } else {
                                            card.sqlerrmc
                                        },
                                    });
                                }
                                total_row_count += card.row_count() as i64;
                                sqlcards_seen += 1;
                            }
                            codepoints::RDBUPDRM | codepoints::ENDQRYRM => {}
                            _ => {
                                trace!(
                                    "batch reply: unexpected code point 0x{:04X}",
                                    obj.code_point
                                );
                            }
                        }
                    }
                }
            }
        }

        Ok(QueryResult {
            rows: Vec::new(),
            columns: Vec::new(),
            row_count: total_row_count,
        })
    }

    pub async fn describe_input(
        &mut self,
        pkgnamcsn: &[u8],
    ) -> Result<Vec<db2_proto::fdoca::ColumnDescriptor>, Error> {
        let corr_id = self.next_correlation_id();
        let dscsqlstt_data = db2_proto::commands::dscsqlstt::build_dscsqlstt_input(pkgnamcsn);

        let mut writer = DssWriter::new(corr_id);
        writer.write_request(&dscsqlstt_data, false);
        let send_buf = writer.finish();
        self.send_bytes(&send_buf).await?;

        let mut frames = self.read_reply_frames().await?;
        let frame_drain_timeout = self.frame_drain_timeout();
        loop {
            let more_frames = match timeout(frame_drain_timeout, self.read_reply_frames()).await {
                Ok(Ok(frames)) => frames,
                Ok(Err(err)) => return Err(err),
                Err(_) => break,
            };
            if more_frames.is_empty() {
                break;
            }
            frames.extend(more_frames);
        }
        if debug_hex_enabled() {
            for (frame_index, frame) in frames.iter().enumerate() {
                let cps: Vec<String> = Self::parse_ddm_objects(&frame.payload)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|obj| format!("0x{:04X}", obj.code_point))
                    .collect();
                eprintln!(
                    "[db2-wire] describe input frame#{} cps={:?}",
                    frame_index + 1,
                    cps
                );
            }
        }
        let descriptors = self.parse_input_descriptors(&frames)?;
        if debug_hex_enabled() {
            eprintln!(
                "[db2-wire] describe input returned {} descriptor(s): {:?}",
                descriptors.len(),
                descriptors
            );
        }
        Ok(descriptors)
    }

    /// Public wrapper for process_query_reply (used by PreparedStatement).
    pub async fn process_query_reply_public(
        &mut self,
        frames: &[DssFrame],
        column_info: &[ColumnInfo],
        initial_descriptors: Option<&[db2_proto::fdoca::ColumnDescriptor]>,
    ) -> Result<QueryResult, Error> {
        self.process_query_reply(frames, column_info, initial_descriptors)
            .await
    }

    /// Public wrapper for process_execute_reply (used by PreparedStatement).
    pub async fn process_execute_reply_public(
        &mut self,
        frames: &[DssFrame],
    ) -> Result<QueryResult, Error> {
        self.process_execute_reply(frames).await
    }

    pub async fn read_execute_reply_frames_public(&mut self) -> Result<Vec<DssFrame>, Error> {
        self.read_execute_reply_frames().await
    }

    /// Process reply frames from a query that returns rows.
    async fn process_query_reply(
        &mut self,
        frames: &[DssFrame],
        column_info: &[ColumnInfo],
        initial_descriptors: Option<&[db2_proto::fdoca::ColumnDescriptor]>,
    ) -> Result<QueryResult, Error> {
        let mut rows = Vec::new();
        let mut sqldard_descriptors = initial_descriptors
            .filter(|descriptors| !descriptors.is_empty())
            .map(|descriptors| descriptors.to_vec());
        let mut qrydsc_descriptors: Option<Vec<db2_proto::fdoca::ColumnDescriptor>> = None;
        let mut query_instance_id: Option<Vec<u8>> = None;
        let mut pending_row_bytes = Vec::new();
        let mut end_of_query = false;

        process_query_frames(
            frames,
            column_info,
            &mut rows,
            &mut sqldard_descriptors,
            &mut qrydsc_descriptors,
            &mut query_instance_id,
            &mut pending_row_bytes,
            &mut end_of_query,
        )?;

        let frame_drain_timeout = self.frame_drain_timeout();

        // DB2 LUW can stream additional QRYDTA blocks immediately after OPNQRY.
        // Drain those frames before sending CNTQRY, otherwise the server may reject
        // the fetch request as out-of-sequence while the original reply is still active.
        while !end_of_query {
            let more_frames = match timeout(frame_drain_timeout, self.read_reply_frames()).await {
                Ok(Ok(frames)) => frames,
                Ok(Err(err)) => return Err(err),
                Err(_) => {
                    if debug_hex_enabled() {
                        eprintln!("[db2-wire] query drain timed out; switching to CNTQRY");
                    }
                    break;
                }
            };

            if more_frames.is_empty() {
                break;
            }

            if debug_hex_enabled() {
                eprintln!(
                    "[db2-wire] drained {} additional query frame(s) before CNTQRY",
                    more_frames.len()
                );
            }

            process_query_frames(
                &more_frames,
                column_info,
                &mut rows,
                &mut sqldard_descriptors,
                &mut qrydsc_descriptors,
                &mut query_instance_id,
                &mut pending_row_bytes,
                &mut end_of_query,
            )?;
        }

        // If not end of query, continue fetching explicitly.
        if !end_of_query {
            let cursor_descriptors = sqldard_descriptors
                .clone()
                .or(qrydsc_descriptors.clone())
                .filter(|descriptors| !descriptors.is_empty());
            if let Some(descriptors) = cursor_descriptors {
                if debug_hex_enabled() {
                    eprintln!(
                        "[db2-wire] opening cursor fallback with {} decoded row(s), pending_tail={}",
                        rows.len(),
                        pending_row_bytes.len()
                    );
                }
                let mut cursor = Cursor::new(
                    column_info.to_vec(),
                    descriptors,
                    query_instance_id,
                    self.config.fetch_size,
                );
                cursor.pending_row_bytes = pending_row_bytes;

                loop {
                    let (more_rows, done) = cursor.fetch_next_from(self).await?;
                    rows.extend(more_rows);
                    if done {
                        break;
                    }
                }
            }
        }

        let columns = if !column_info.is_empty() {
            column_info.to_vec()
        } else if let Some(descriptors) =
            sqldard_descriptors.as_ref().or(qrydsc_descriptors.as_ref())
        {
            column_info_from_descriptors(descriptors)
        } else {
            Vec::new()
        };

        if debug_hex_enabled() {
            eprintln!(
                "[db2-wire] process_query_reply final columns={} rows={} initial_columns={} qrydsc_desc={} sqldard_desc={}",
                columns.len(),
                rows.len(),
                column_info.len(),
                qrydsc_descriptors.as_ref().map(|v| v.len()).unwrap_or(0),
                sqldard_descriptors.as_ref().map(|v| v.len()).unwrap_or(0)
            );
        }

        Ok(QueryResult::with_rows(rows, columns))
    }

    /// Process reply frames from an execute (non-query) statement.
    async fn process_execute_reply(&mut self, frames: &[DssFrame]) -> Result<QueryResult, Error> {
        let mut row_count: i64 = 0;
        let mut columns = Vec::new();

        for frame in frames.iter() {
            for obj in Self::parse_ddm_objects(&frame.payload)? {
                if let Some(err) = protocol_reply_error(&obj, "execute") {
                    return Err(err);
                }
                match obj.code_point {
                    codepoints::SQLCARD => {
                        trace!(
                            "Received SQLCARD, data[0..min(20,len)]={:02X?}",
                            &obj.data[..std::cmp::min(20, obj.data.len())]
                        );
                        let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                            .map_err(|e| Error::Protocol(e.to_string()))?;

                        if card.is_error() {
                            return Err(Error::Sql {
                                sqlstate: card.sqlstate,
                                sqlcode: card.sqlcode,
                                message: if card.sqlerrmc.is_empty() {
                                    format!("SQL error: SQLCODE={}", card.sqlcode)
                                } else {
                                    card.sqlerrmc
                                },
                            });
                        }
                        let card_row_count = card.row_count() as i64;
                        if card_row_count != 0 || row_count == 0 {
                            row_count = card_row_count;
                        }
                    }
                    codepoints::RDBUPDRM => {
                        trace!("Received RDBUPDRM");
                    }
                    codepoints::SQLERRRM => {
                        trace!("Received SQLERRRM");
                        return Err(Error::Sql {
                            sqlstate: "HY000".into(),
                            sqlcode: -1,
                            message: "SQL error reply received".into(),
                        });
                    }
                    codepoints::SQLDARD => {
                        trace!("Received SQLDARD");
                        columns = parse_sqldard_columns(&obj);
                    }
                    codepoints::ENDQRYRM => {
                        trace!("Received ENDQRYRM");
                    }
                    _ => {
                        trace!("Ignoring reply codepoint 0x{:04X}", obj.code_point);
                    }
                }
            }
        }

        Ok(QueryResult {
            rows: Vec::new(),
            row_count,
            columns,
        })
    }

    /// Parse the reply to a PRPSQLSTT (prepare) command.
    pub fn parse_prepare_reply(&self, frames: &[DssFrame]) -> Result<Vec<ColumnInfo>, Error> {
        let mut columns = Vec::new();

        for frame in frames {
            for obj in Self::parse_ddm_objects(&frame.payload)? {
                if let Some(err) = protocol_reply_error(&obj, "prepare") {
                    return Err(err);
                }
                match obj.code_point {
                    codepoints::SQLDARD => {
                        trace!("Received SQLDARD from prepare");
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] prepare SQLDARD preview={}",
                                format_hex_preview(&obj.data, 192)
                            );
                        }
                        columns = parse_sqldard_columns(&obj);
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] prepare SQLDARD produced {} column metadata entrie(s)",
                                columns.len()
                            );
                        }
                    }
                    codepoints::SQLCARD => {
                        let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        if card.is_error() {
                            return Err(Error::Sql {
                                sqlstate: card.sqlstate,
                                sqlcode: card.sqlcode,
                                message: format!("Prepare failed: {}", card.sqlerrmc),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(columns)
    }

    pub fn parse_prepare_result_descriptors(
        &self,
        frames: &[DssFrame],
    ) -> Vec<db2_proto::fdoca::ColumnDescriptor> {
        for frame in frames {
            let Ok(objects) = Self::parse_ddm_objects(&frame.payload) else {
                continue;
            };
            for obj in objects {
                if obj.code_point == codepoints::SQLDARD {
                    let descriptors = parse_sqldard_descriptors(&obj);
                    if !descriptors.is_empty() {
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] prepare SQLDARD produced {} result descriptor(s)",
                                descriptors.len()
                            );
                        }
                        return descriptors;
                    }
                    if debug_hex_enabled() {
                        eprintln!("[db2-wire] prepare SQLDARD descriptor parse returned 0 entries");
                    }
                }
            }
        }

        Vec::new()
    }

    fn parse_input_descriptors(
        &self,
        frames: &[DssFrame],
    ) -> Result<Vec<db2_proto::fdoca::ColumnDescriptor>, Error> {
        let mut descriptors = Vec::new();

        for frame in frames {
            for obj in Self::parse_ddm_objects(&frame.payload)? {
                if let Some(err) = protocol_reply_error(&obj, "describe input") {
                    return Err(err);
                }
                match obj.code_point {
                    codepoints::SQLDARD => {
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] describe input SQLDARD len={} preview={}",
                                obj.data.len(),
                                format_hex_preview(&obj.data, 160)
                            );
                        }
                        descriptors = parse_input_sqldard_descriptors(&obj);
                    }
                    codepoints::SQLCARD => {
                        let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        if card.is_error() {
                            return Err(Error::Sql {
                                sqlstate: card.sqlstate,
                                sqlcode: card.sqlcode,
                                message: format!("Describe input failed: {}", card.sqlerrmc),
                            });
                        }
                    }
                    _ => {}
                }
            }
        }

        Ok(descriptors)
    }

    /// Send RDBCMM (commit) command.
    pub async fn commit(&mut self) -> Result<(), Error> {
        debug!("Sending RDBCMM (commit)");
        let corr_id = self.next_correlation_id();
        let rdbcmm_data = db2_proto::commands::rdbcmm::build_rdbcmm();

        let mut writer = DssWriter::new(corr_id);
        writer.write_request(&rdbcmm_data, false);
        let send_buf = writer.finish();
        self.send_bytes(&send_buf).await?;

        let frames = self.read_reply_frames().await?;
        for frame in &frames {
            let obj = Self::parse_ddm(&frame.payload)?;
            if obj.code_point == codepoints::SQLCARD {
                let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if card.is_error() {
                    return Err(Error::Sql {
                        sqlstate: card.sqlstate,
                        sqlcode: card.sqlcode,
                        message: format!("Commit failed: {}", card.sqlerrmc),
                    });
                }
            }
        }

        debug!("Commit successful");
        self.auto_commit = true;
        Ok(())
    }

    /// Send RDBRLLBCK (rollback) command.
    pub async fn rollback(&mut self) -> Result<(), Error> {
        debug!("Sending RDBRLLBCK (rollback)");
        let corr_id = self.next_correlation_id();
        let rdbrllbck_data = db2_proto::commands::rdbrllbck::build_rdbrllbck();

        let mut writer = DssWriter::new(corr_id);
        writer.write_request(&rdbrllbck_data, false);
        let send_buf = writer.finish();
        self.send_bytes(&send_buf).await?;

        let frames = self.read_reply_frames().await?;
        for frame in &frames {
            let obj = Self::parse_ddm(&frame.payload)?;
            if obj.code_point == codepoints::SQLCARD {
                let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                    .map_err(|e| Error::Protocol(e.to_string()))?;
                if card.is_error() {
                    return Err(Error::Sql {
                        sqlstate: card.sqlstate,
                        sqlcode: card.sqlcode,
                        message: format!("Rollback failed: {}", card.sqlerrmc),
                    });
                }
            }
        }

        debug!("Rollback successful");
        self.auto_commit = true;
        Ok(())
    }
}

/// The main DB2 client. Wraps shared internal state in an Arc<Mutex<>>.
pub struct Client {
    pub(crate) inner: Arc<Mutex<ClientInner>>,
    pool_checkout: StdMutex<Option<PoolCheckoutHandle>>,
}

impl Client {
    /// Create a new Client with the given configuration. Does not connect immediately.
    pub fn new(config: Config) -> Self {
        Client {
            inner: Arc::new(Mutex::new(ClientInner {
                transport: None,
                config,
                server_info: None,
                correlation_id: 1,
                section_number: DIRECT_QUERY_SECTION,
                package_id: DIRECT_QUERY_PKGID,
                auto_commit: true,
                connected: false,
                connected_once: false,
                closed_explicitly: false,
                session_generation: 0,
                recv_buf: BytesMut::with_capacity(8192),
                next_prepared_section: 1,
                free_prepared_sections: Vec::new(),
            })),
            pool_checkout: StdMutex::new(None),
        }
    }

    pub(crate) fn pool_key(&self) -> usize {
        Arc::as_ptr(&self.inner) as usize
    }

    pub(crate) fn attach_pool_checkout(&self, checked_out: &Arc<Mutex<PoolCheckoutMap>>) {
        if let Ok(mut guard) = self.pool_checkout.lock() {
            *guard = Some(PoolCheckoutHandle {
                key: self.pool_key(),
                checked_out: Arc::downgrade(checked_out),
            });
        }
    }

    pub(crate) async fn detach_pool_checkout(&self) -> Option<PoolCheckoutEntry> {
        let handle = self.pool_checkout.lock().ok()?.take()?;
        let checked_out = handle.checked_out.upgrade()?;
        let entry = checked_out.lock().await.remove(&handle.key);
        entry
    }

    /// Connect to the DB2 server, performing TLS upgrade and DRDA authentication.
    pub async fn connect(&mut self) -> Result<(), Error> {
        let mut guard = self.inner.lock().await;
        if guard.connected {
            return Ok(());
        }
        guard.closed_explicitly = false;
        guard.establish_session().await
    }

    /// Create a new client and immediately connect.
    pub async fn connect_with(config: Config) -> Result<Self, Error> {
        let mut client = Client::new(config);
        client.connect().await?;
        Ok(client)
    }

    /// Execute a SQL query or statement with optional parameters.
    pub async fn query(&self, sql: &str, params: &[&dyn ToSql]) -> Result<QueryResult, Error> {
        let mut guard = self.inner.lock().await;
        guard.reconnect_if_needed("query").await?;
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }
        let query_timeout = guard.config.query_timeout;
        if query_timeout.is_zero() {
            match guard.execute_query(sql, params).await {
                Ok(result) => Ok(result),
                Err(err) => Err(guard.finalize_operation_error("query", err).await),
            }
        } else {
            match timeout(query_timeout, guard.execute_query(sql, params)).await {
                Ok(result) => match result {
                    Ok(result) => Ok(result),
                    Err(err) => Err(guard.finalize_operation_error("query", err).await),
                },
                Err(_) => Err(guard.disconnect_after_timeout("query", query_timeout).await),
            }
        }
    }

    /// Execute a SQL statement with no parameters.
    pub async fn execute(&self, sql: &str) -> Result<QueryResult, Error> {
        self.query(sql, &[]).await
    }

    /// Prepare a SQL statement for later execution with parameters.
    pub async fn prepare(&self, sql: &str) -> Result<crate::statement::PreparedStatement, Error> {
        ensure_sqlstt_sql_len(sql)?;
        let mut guard = self.inner.lock().await;
        guard.reconnect_if_needed("prepare").await?;
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }

        let query_timeout = guard.config.query_timeout;
        let prepare_future = async {
            let section_number = guard.allocate_prepared_section()?;
            let corr_id = guard.next_correlation_id();
            let pkgnamcsn = guard.build_pkgnamcsn_for(PREPARED_STATEMENT_PKGID, section_number);

            let prpsqlstt_data =
                db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn);
            let sqlstt_data = db2_proto::commands::sqlstt::build_sqlstt(sql);

            let mut writer = DssWriter::new(corr_id);
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
                guard.session_generation,
                column_metadata,
                result_descriptors,
                input_descriptors,
            ))
        };

        if query_timeout.is_zero() {
            match prepare_future.await {
                Ok(statement) => Ok(statement),
                Err(err) => Err(guard.finalize_operation_error("prepare", err).await),
            }
        } else {
            match timeout(query_timeout, prepare_future).await {
                Ok(result) => match result {
                    Ok(statement) => Ok(statement),
                    Err(err) => Err(guard.finalize_operation_error("prepare", err).await),
                },
                Err(_) => Err(guard
                    .disconnect_after_timeout("prepare", query_timeout)
                    .await),
            }
        }
    }

    /// Begin a new transaction (turns off auto-commit behavior).
    pub async fn begin_transaction(&self) -> Result<crate::transaction::Transaction, Error> {
        let mut guard = self.inner.lock().await;
        guard.reconnect_if_needed("begin transaction").await?;
        if !guard.connected {
            return Err(Error::Connection("Not connected".into()));
        }
        guard.auto_commit = false;
        let session_generation = guard.session_generation;
        drop(guard);

        Ok(crate::transaction::Transaction::new(
            self.inner.clone(),
            session_generation,
        ))
    }

    /// Close the connection.
    pub async fn close(&self) -> Result<(), Error> {
        let mut guard = self.inner.lock().await;
        if guard.connected {
            if let Some(transport) = guard.transport.as_mut() {
                transport.close().await?;
            }
            debug!("Connection closed");
        }
        guard.transport = None;
        guard.connected = false;
        guard.auto_commit = true;
        guard.closed_explicitly = true;
        guard.server_info = None;
        guard.section_number = DIRECT_QUERY_SECTION;
        guard.package_id = DIRECT_QUERY_PKGID;
        guard.recv_buf.clear();
        guard.next_prepared_section = 1;
        guard.free_prepared_sections.clear();
        drop(guard);
        let _ = self.detach_pool_checkout().await;
        Ok(())
    }

    /// Get the server info (populated after connect).
    pub async fn server_info(&self) -> Option<ServerInfo> {
        let guard = self.inner.lock().await;
        guard.server_info.clone()
    }

    /// Check if the client is connected.
    pub async fn is_connected(&self) -> bool {
        let guard = self.inner.lock().await;
        guard.connected
    }
}

// ============================================================
// Helper functions
// ============================================================

const SQLSTT_SQL_TEXT_LEN_LIMIT: usize = u16::MAX as usize;

pub(crate) fn ensure_sqlstt_sql_len(sql: &str) -> Result<(), Error> {
    let sql_len = sql.len();
    if sql_len > SQLSTT_SQL_TEXT_LEN_LIMIT {
        return Err(Error::Other(format!(
            "SQL text is {} bytes, exceeding the current SQLSTT limit of {} bytes",
            sql_len, SQLSTT_SQL_TEXT_LEN_LIMIT
        )));
    }
    Ok(())
}

pub(crate) fn build_sqldta(
    params: &[&dyn ToSql],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Result<Vec<u8>, Error> {
    let descriptors = if descriptors.is_empty() {
        infer_parameter_descriptors(params)?
    } else {
        descriptors.to_vec()
    };

    if descriptors.len() != params.len() {
        return Err(Error::Protocol(format!(
            "parameter descriptor count {} does not match parameter count {}",
            descriptors.len(),
            params.len()
        )));
    }

    let mut builder = db2_proto::ddm::DdmBuilder::new(codepoints::SQLDTA);
    builder.add_raw(&build_sqldta_fdoca_prefix(&descriptors)?);
    let data = build_sqldta_row_data(params, &descriptors)?;
    let mut inner = db2_proto::ddm::DdmBuilder::new(codepoints::FDODTA);
    inner.add_raw(&data);
    builder.add_raw(&inner.build());
    let sqldta = builder.build();
    if debug_hex_enabled() {
        eprintln!(
            "[db2-wire] built SQLDTA with {} descriptor(s), total={} bytes, preview={}",
            descriptors.len(),
            sqldta.len(),
            format_hex_preview(&sqldta, 128)
        );
    }
    Ok(sqldta)
}

fn build_sqldta_fdoca_prefix(
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Result<Vec<u8>, Error> {
    const FDODTA_HEADER_ID: u16 = 0x0010;
    const TRIPLET_TYPE_GDA: u8 = 0x76;
    const TRIPLET_TYPE_RLO: u8 = 0x71;
    const GDA_PREFIX: u8 = 0xD0;
    const RLO_BYTES: [u8; 4] = [0xE4, 0xD0, 0x00, 0x01];

    let gda_len = 3 + descriptors.len() * 3;
    if gda_len > u8::MAX as usize {
        return Err(Error::Other(format!(
            "too many parameters for SQLDTA descriptor header: {}",
            descriptors.len()
        )));
    }

    let mut gda = Vec::with_capacity(gda_len);
    gda.push(gda_len as u8);
    gda.push(TRIPLET_TYPE_GDA);
    gda.push(GDA_PREFIX);
    for descriptor in descriptors {
        gda.push(descriptor.drda_type);
        gda.extend_from_slice(&descriptor.length.to_be_bytes());
    }

    let rlo = [
        0x06,
        TRIPLET_TYPE_RLO,
        RLO_BYTES[0],
        RLO_BYTES[1],
        RLO_BYTES[2],
        RLO_BYTES[3],
    ];
    let prefix_len = 4 + gda.len() + rlo.len();

    let mut prefix = Vec::with_capacity(prefix_len);
    prefix.extend_from_slice(&(prefix_len as u16).to_be_bytes());
    prefix.extend_from_slice(&FDODTA_HEADER_ID.to_be_bytes());
    prefix.extend_from_slice(&gda);
    prefix.extend_from_slice(&rlo);
    Ok(prefix)
}

/// Parse column info from an SQLDARD DDM object.
fn parse_sqldard_columns(obj: &DdmObject) -> Vec<ColumnInfo> {
    let dard = match db2_proto::replies::sqldard::parse_sqldard(obj) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    dard.columns
        .into_iter()
        .map(|col| ColumnInfo {
            name: col.name.clone(),
            type_name: format!("{:?}", col.db2_type),
            nullable: col.nullable,
            precision: match col.db2_type {
                db2_proto::types::Db2Type::DecFloat(digits) => Some(digits as u16),
                _ if col.precision > 0 => Some(col.precision as u16),
                _ => None,
            },
            scale: if col.scale > 0 {
                Some(col.scale as u16)
            } else {
                None
            },
        })
        .collect()
}

fn parse_sqldard_descriptors(obj: &DdmObject) -> Vec<db2_proto::fdoca::ColumnDescriptor> {
    let dard = match db2_proto::replies::sqldard::parse_sqldard(obj) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    dard.columns
        .into_iter()
        .map(|col| db2_proto::fdoca::ColumnDescriptor {
            column_index: col.index,
            drda_type: col.drda_type,
            length: col.length,
            precision: col.precision,
            scale: col.scale,
            nullable: col.nullable,
            ccsid: col.ccsid,
            db2_type: col.db2_type,
        })
        .collect()
}

fn parse_input_sqldard_descriptors(obj: &DdmObject) -> Vec<db2_proto::fdoca::ColumnDescriptor> {
    let dard = db2_proto::replies::sqldard::parse_sqldard(obj).ok();
    if let Some(dard) = dard {
        if !dard.columns.is_empty() {
            return dard
                .columns
                .into_iter()
                .map(|col| db2_proto::fdoca::ColumnDescriptor {
                    column_index: col.index,
                    drda_type: input_drda_type_for(&col.db2_type, col.ccsid, true),
                    length: input_length_for(&col.db2_type, col.length, col.precision, col.scale),
                    precision: col.precision,
                    scale: col.scale,
                    nullable: true,
                    ccsid: col.ccsid,
                    db2_type: col.db2_type,
                })
                .collect();
        }
    }

    parse_input_sqldard_compact(&obj.data)
}

const NULL_LID: u8 = 0x00;
const NULL_DATA: u8 = 0xFF;
const INDICATOR_NOT_NULL: u8 = 0x00;
const INPUT_SQLDARD_PREFIX_LEN: usize = 28;

fn parse_input_sqldard_compact(data: &[u8]) -> Vec<db2_proto::fdoca::ColumnDescriptor> {
    if data.len() < INPUT_SQLDARD_PREFIX_LEN {
        return Vec::new();
    }

    let count = u16::from_le_bytes([
        data[INPUT_SQLDARD_PREFIX_LEN - 2],
        data[INPUT_SQLDARD_PREFIX_LEN - 1],
    ]) as usize;
    if count == 0 {
        return Vec::new();
    }

    let mut descriptors = Vec::with_capacity(count);
    let mut offset = INPUT_SQLDARD_PREFIX_LEN;

    for index in 0..count {
        let end =
            next_input_descriptor_offset(data, offset, index + 1 < count).unwrap_or(data.len());
        if offset + 16 > end || end > data.len() {
            break;
        }

        let descriptor = &data[offset..end];
        let precision = u16::from_le_bytes([descriptor[0], descriptor[1]]) as u8;
        let scale = u16::from_le_bytes([descriptor[2], descriptor[3]]) as u8;
        let raw_length = u64::from_le_bytes([
            descriptor[4],
            descriptor[5],
            descriptor[6],
            descriptor[7],
            descriptor[8],
            descriptor[9],
            descriptor[10],
            descriptor[11],
        ]);
        let sql_type = u16::from_le_bytes([descriptor[12], descriptor[13]]);
        let nullable = (sql_type & 0x0001) != 0;
        let db2_type = compact_sqlda_db2_type(sql_type, raw_length, precision, scale);
        let length = input_length_for(
            &db2_type,
            raw_length.min(u16::MAX as u64) as u16,
            precision,
            scale,
        );

        descriptors.push(db2_proto::fdoca::ColumnDescriptor {
            column_index: index,
            drda_type: input_drda_type_for(&db2_type, 1208, nullable),
            length,
            precision,
            scale,
            nullable,
            ccsid: 1208,
            db2_type,
        });

        offset = end;
    }

    descriptors
}

fn next_input_descriptor_offset(data: &[u8], start: usize, expect_more: bool) -> Option<usize> {
    if !expect_more {
        return Some(data.len());
    }

    for pos in (start + 16)..data.len() {
        if data[pos] != 0xFF {
            continue;
        }

        let next = pos + 1;
        if next + 16 > data.len() {
            continue;
        }

        if looks_like_input_descriptor_start(&data[next..]) {
            return Some(next);
        }
    }

    None
}

fn looks_like_input_descriptor_start(data: &[u8]) -> bool {
    if data.len() < 16 {
        return false;
    }

    let precision = u16::from_le_bytes([data[0], data[1]]) as u8;
    let scale = u16::from_le_bytes([data[2], data[3]]) as u8;
    let raw_length = u64::from_le_bytes([
        data[4], data[5], data[6], data[7], data[8], data[9], data[10], data[11],
    ]);
    let sql_type = u16::from_le_bytes([data[12], data[13]]);
    let base_sql_type = sql_type & !1;

    if !matches!(
        base_sql_type,
        384 | 388
            | 392
            | 404
            | 408
            | 412
            | 448
            | 452
            | 456
            | 464
            | 468
            | 472
            | 480
            | 484
            | 488
            | 492
            | 496
            | 500
            | 996
            | 908
            | 912
            | 988
    ) {
        return false;
    }

    if precision > 63 || scale > 63 {
        return false;
    }

    raw_length <= 0x0001_0000
}

fn compact_sqlda_db2_type(
    sql_type: u16,
    raw_length: u64,
    precision: u8,
    scale: u8,
) -> db2_proto::types::Db2Type {
    use db2_proto::types::Db2Type;

    match sql_type & !1 {
        384 => Db2Type::Date,
        388 => Db2Type::Time,
        392 => Db2Type::Timestamp,
        404 => Db2Type::Blob,
        408 => Db2Type::Clob,
        412 => Db2Type::DbClob,
        448 | 456 => Db2Type::VarChar(raw_length.min(u16::MAX as u64) as u16),
        452 => Db2Type::Char(raw_length.min(u16::MAX as u64) as u16),
        464 | 472 => Db2Type::VarGraphic(raw_length.min(u16::MAX as u64) as u16),
        468 => Db2Type::Graphic(raw_length.min(u16::MAX as u64) as u16),
        480 => {
            if raw_length >= 8 {
                Db2Type::Double
            } else {
                Db2Type::Real
            }
        }
        484 | 488 => Db2Type::Decimal { precision, scale },
        996 => Db2Type::DecFloat(if raw_length > 8 { 34 } else { 16 }),
        492 => Db2Type::BigInt,
        496 => Db2Type::Integer,
        500 => Db2Type::SmallInt,
        908 => Db2Type::VarBinary(raw_length.min(u16::MAX as u64) as u16),
        912 => Db2Type::Binary(raw_length.min(u16::MAX as u64) as u16),
        988 => Db2Type::Xml,
        _ => Db2Type::VarChar(raw_length.min(u16::MAX as u64) as u16),
    }
}

fn infer_parameter_descriptors(
    params: &[&dyn ToSql],
) -> Result<Vec<db2_proto::fdoca::ColumnDescriptor>, Error> {
    params
        .iter()
        .enumerate()
        .map(|(index, param)| {
            let db2_type = param.db2_type();
            if matches!(db2_type, db2_proto::types::Db2Type::Null) {
                return Err(Error::Other(format!(
                    "cannot infer protocol type for NULL parameter {} without input metadata",
                    index + 1
                )));
            }

            let length = input_length_for(&db2_type, 0, 0, 0);
            Ok(db2_proto::fdoca::ColumnDescriptor {
                column_index: index,
                drda_type: input_drda_type_for(&db2_type, 1208, true),
                length,
                precision: 0,
                scale: 0,
                nullable: true,
                ccsid: 1208,
                db2_type,
            })
        })
        .collect()
}

fn build_sqldta_row_data(
    params: &[&dyn ToSql],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Result<Vec<u8>, Error> {
    let mut data = Vec::with_capacity(1 + params.len() * 8);
    data.push(NULL_LID);

    for (index, (param, descriptor)) in params.iter().zip(descriptors.iter()).enumerate() {
        let value = param.to_db2_value();
        if descriptor.nullable {
            if value.is_null() {
                data.push(NULL_DATA);
                continue;
            }
            data.push(INDICATOR_NOT_NULL);
        } else if value.is_null() {
            return Err(Error::Other(format!(
                "parameter {} is NULL but the server reported a non-nullable input type",
                index + 1
            )));
        }

        data.extend_from_slice(&encode_parameter_value(&value, descriptor)?);
    }

    Ok(data)
}

fn encode_parameter_value(
    value: &db2_proto::types::Db2Value,
    descriptor: &db2_proto::fdoca::ColumnDescriptor,
) -> Result<Vec<u8>, Error> {
    use db2_proto::types::{Db2Type, Db2Value};

    let encoded = match &descriptor.db2_type {
        Db2Type::SmallInt => {
            let v = i16::try_from(expect_i64(value)?)
                .map_err(|_| Error::Other("SMALLINT parameter out of range".into()))?;
            v.to_le_bytes().to_vec()
        }
        Db2Type::Integer => {
            let v = i32::try_from(expect_i64(value)?)
                .map_err(|_| Error::Other("INTEGER parameter out of range".into()))?;
            v.to_le_bytes().to_vec()
        }
        Db2Type::BigInt => expect_i64(value)?.to_le_bytes().to_vec(),
        Db2Type::Real => (expect_f64(value)? as f32).to_le_bytes().to_vec(),
        Db2Type::Double => expect_f64(value)?.to_le_bytes().to_vec(),
        Db2Type::Decimal { precision, scale } => {
            let decimal = match value {
                Db2Value::Decimal(v)
                | Db2Value::Char(v)
                | Db2Value::VarChar(v)
                | Db2Value::Clob(v) => v.clone(),
                _ => value
                    .as_i64()
                    .map(|v| v.to_string())
                    .or_else(|| value.as_f64().map(|v| v.to_string()))
                    .ok_or_else(|| {
                        Error::Other("DECIMAL parameters must be numeric or string values".into())
                    })?,
            };
            db2_proto::types::encode_packed_decimal(&decimal, *precision, *scale)
                .map_err(Error::from)?
        }
        Db2Type::DecFloat(digits) => {
            let decimal = match value {
                Db2Value::Decimal(v)
                | Db2Value::Char(v)
                | Db2Value::VarChar(v)
                | Db2Value::Clob(v) => v.clone(),
                _ => value
                    .as_i64()
                    .map(|v| v.to_string())
                    .or_else(|| value.as_f64().map(|v| v.to_string()))
                    .ok_or_else(|| {
                        Error::Other("DECFLOAT parameters must be numeric or string values".into())
                    })?,
            };
            db2_proto::types::encode_decfloat(&decimal, *digits).map_err(Error::from)?
        }
        Db2Type::Char(len) => encode_fixed_string(value, *len as usize, descriptor.ccsid)?,
        Db2Type::VarChar(_) | Db2Type::LongVarChar | Db2Type::Clob | Db2Type::Xml => {
            encode_ld_string(value, descriptor.ccsid)?
        }
        Db2Type::Binary(len) => encode_fixed_binary(value, *len as usize)?,
        Db2Type::VarBinary(_) | Db2Type::Blob => encode_ld_binary(value)?,
        Db2Type::Date => encode_exact_string(value, 10, descriptor.ccsid)?,
        Db2Type::Time => encode_exact_string(value, 8, descriptor.ccsid)?,
        Db2Type::Timestamp => encode_timestamp(value, descriptor.ccsid)?,
        Db2Type::Boolean => vec![if expect_bool(value)? { 1 } else { 0 }],
        Db2Type::Graphic(_) | Db2Type::VarGraphic(_) | Db2Type::DbClob | Db2Type::Null => {
            return Err(Error::Other(format!(
                "unsupported parameter type for SQLDTA encoding: {:?}",
                descriptor.db2_type
            )));
        }
    };

    Ok(encoded)
}

fn expect_i64(value: &db2_proto::types::Db2Value) -> Result<i64, Error> {
    value.as_i64().ok_or_else(|| {
        Error::Other(format!(
            "expected integer-compatible parameter, got {:?}",
            value
        ))
    })
}

fn expect_f64(value: &db2_proto::types::Db2Value) -> Result<f64, Error> {
    value
        .as_f64()
        .ok_or_else(|| Error::Other(format!("expected numeric parameter, got {:?}", value)))
}

fn expect_bool(value: &db2_proto::types::Db2Value) -> Result<bool, Error> {
    match value {
        db2_proto::types::Db2Value::Boolean(v) => Ok(*v),
        db2_proto::types::Db2Value::SmallInt(v) => Ok(*v != 0),
        db2_proto::types::Db2Value::Integer(v) => Ok(*v != 0),
        db2_proto::types::Db2Value::BigInt(v) => Ok(*v != 0),
        _ => Err(Error::Other(format!(
            "expected boolean-compatible parameter, got {:?}",
            value
        ))),
    }
}

fn encode_exact_string(
    value: &db2_proto::types::Db2Value,
    len: usize,
    ccsid: u16,
) -> Result<Vec<u8>, Error> {
    let bytes = encode_text_bytes(value, ccsid)?;
    if bytes.len() != len {
        return Err(Error::Other(format!(
            "expected string length {} bytes, got {}",
            len,
            bytes.len()
        )));
    }
    Ok(bytes)
}

fn encode_timestamp(value: &db2_proto::types::Db2Value, ccsid: u16) -> Result<Vec<u8>, Error> {
    let bytes = encode_text_bytes(value, ccsid)?;
    if matches!(bytes.len(), 26 | 29) {
        Ok(bytes)
    } else {
        Err(Error::Other(format!(
            "expected timestamp length 26 or 29 bytes, got {}",
            bytes.len()
        )))
    }
}

fn encode_fixed_string(
    value: &db2_proto::types::Db2Value,
    len: usize,
    ccsid: u16,
) -> Result<Vec<u8>, Error> {
    let mut bytes = encode_text_bytes(value, ccsid)?;
    if bytes.len() > len {
        bytes.truncate(len);
    } else if bytes.len() < len {
        bytes.resize(len, b' ');
    }
    Ok(bytes)
}

fn encode_ld_string(value: &db2_proto::types::Db2Value, ccsid: u16) -> Result<Vec<u8>, Error> {
    let bytes = encode_text_bytes(value, ccsid)?;
    if bytes.len() > u16::MAX as usize {
        return Err(Error::Other("string parameter too large for SQLDTA".into()));
    }
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(out)
}

fn encode_text_bytes(value: &db2_proto::types::Db2Value, ccsid: u16) -> Result<Vec<u8>, Error> {
    let text = match value {
        db2_proto::types::Db2Value::Char(v)
        | db2_proto::types::Db2Value::VarChar(v)
        | db2_proto::types::Db2Value::Clob(v)
        | db2_proto::types::Db2Value::Date(v)
        | db2_proto::types::Db2Value::Time(v)
        | db2_proto::types::Db2Value::Timestamp(v)
        | db2_proto::types::Db2Value::Decimal(v)
        | db2_proto::types::Db2Value::Xml(v) => v.as_str(),
        _ => {
            return Err(Error::Other(format!(
                "expected string-compatible parameter, got {:?}",
                value
            )))
        }
    };

    if matches!(ccsid, 37 | 500) {
        Ok(db2_proto::codepage::utf8_to_ebcdic037(text))
    } else {
        Ok(text.as_bytes().to_vec())
    }
}

fn encode_fixed_binary(value: &db2_proto::types::Db2Value, len: usize) -> Result<Vec<u8>, Error> {
    let mut bytes = extract_binary(value)?;
    if bytes.len() > len {
        bytes.truncate(len);
    } else if bytes.len() < len {
        bytes.resize(len, 0);
    }
    Ok(bytes)
}

fn encode_ld_binary(value: &db2_proto::types::Db2Value) -> Result<Vec<u8>, Error> {
    let bytes = extract_binary(value)?;
    if bytes.len() > u16::MAX as usize {
        return Err(Error::Other("binary parameter too large for SQLDTA".into()));
    }
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.extend_from_slice(&(bytes.len() as u16).to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(out)
}

fn extract_binary(value: &db2_proto::types::Db2Value) -> Result<Vec<u8>, Error> {
    match value {
        db2_proto::types::Db2Value::Binary(bytes) | db2_proto::types::Db2Value::Blob(bytes) => {
            Ok(bytes.clone())
        }
        _ => Err(Error::Other(format!(
            "expected binary-compatible parameter, got {:?}",
            value
        ))),
    }
}

fn input_length_for(
    db2_type: &db2_proto::types::Db2Type,
    length: u16,
    precision: u8,
    scale: u8,
) -> u16 {
    use db2_proto::types::Db2Type;

    match db2_type {
        Db2Type::Decimal { precision, scale } => ((*precision as u16) << 8) | (*scale as u16),
        Db2Type::DecFloat(34) => 16,
        Db2Type::DecFloat(_) => 8,
        Db2Type::Char(len)
        | Db2Type::VarChar(len)
        | Db2Type::Binary(len)
        | Db2Type::VarBinary(len)
        | Db2Type::Graphic(len)
        | Db2Type::VarGraphic(len) => *len,
        Db2Type::LongVarChar | Db2Type::Clob | Db2Type::Xml => {
            if length == 0 {
                32767
            } else {
                length
            }
        }
        Db2Type::SmallInt => 2,
        Db2Type::Integer => 4,
        Db2Type::BigInt => 8,
        Db2Type::Real => 4,
        Db2Type::Double => 8,
        Db2Type::Date => 10,
        Db2Type::Time => 8,
        Db2Type::Timestamp => 26,
        Db2Type::Boolean => 1,
        Db2Type::Blob => {
            if length == 0 {
                32767
            } else {
                length
            }
        }
        Db2Type::DbClob => {
            if length == 0 {
                32767
            } else {
                length
            }
        }
        Db2Type::Null => {
            if precision > 0 || scale > 0 {
                ((precision as u16) << 8) | (scale as u16)
            } else if length == 0 {
                32767
            } else {
                length
            }
        }
    }
}

fn input_drda_type_for(db2_type: &db2_proto::types::Db2Type, ccsid: u16, nullable: bool) -> u8 {
    use db2_proto::types::Db2Type;

    let base = match db2_type {
        Db2Type::SmallInt => 0x04,
        Db2Type::Integer => 0x02,
        Db2Type::BigInt => 0x16,
        Db2Type::Real => 0x0C,
        Db2Type::Double => 0x0A,
        Db2Type::Decimal { .. } => 0x0E,
        Db2Type::DecFloat(_) => db2_proto::types::DRDA_TYPE_DECFLOAT,
        Db2Type::Char(_) => {
            if matches!(ccsid, 37 | 500) {
                0x30
            } else {
                0x3C
            }
        }
        Db2Type::VarChar(_) | Db2Type::LongVarChar | Db2Type::Clob | Db2Type::Xml => {
            if matches!(ccsid, 37 | 500) {
                0x32
            } else {
                0x3E
            }
        }
        Db2Type::Binary(_) => 0x26,
        Db2Type::VarBinary(_) | Db2Type::Blob => 0x28,
        Db2Type::Date => 0x20,
        Db2Type::Time => 0x22,
        Db2Type::Timestamp => 0x24,
        Db2Type::Graphic(_) => 0x36,
        Db2Type::VarGraphic(_) | Db2Type::DbClob => 0x38,
        Db2Type::Boolean => 0xBE,
        Db2Type::Null => {
            if matches!(ccsid, 37 | 500) {
                0x32
            } else {
                0x3E
            }
        }
    };

    if nullable {
        base | 0x01
    } else {
        base
    }
}

#[allow(clippy::too_many_arguments)]
fn process_query_frames(
    frames: &[DssFrame],
    column_info: &[ColumnInfo],
    rows: &mut Vec<Row>,
    sqldard_descriptors: &mut Option<Vec<db2_proto::fdoca::ColumnDescriptor>>,
    qrydsc_descriptors: &mut Option<Vec<db2_proto::fdoca::ColumnDescriptor>>,
    query_instance_id: &mut Option<Vec<u8>>,
    pending_row_bytes: &mut Vec<u8>,
    end_of_query: &mut bool,
) -> Result<(), Error> {
    for frame in frames {
        for obj in ClientInner::parse_ddm_objects(&frame.payload)? {
            if debug_hex_enabled() {
                eprintln!(
                    "[db2-wire] query reply object cp=0x{:04X} len={}",
                    obj.code_point,
                    obj.data.len()
                );
                if matches!(obj.code_point, codepoints::SYNTAXRM | codepoints::PRCCNVRM) {
                    let params: Vec<String> = obj
                        .parameters()
                        .into_iter()
                        .map(|param| format!("0x{:04X}", param.code_point))
                        .collect();
                    eprintln!(
                        "[db2-wire] query reply error preview={} params={:?}",
                        format_hex_preview(&obj.data, 160),
                        params
                    );
                }
            }
            if let Some(err) = protocol_reply_error(&obj, "query") {
                return Err(err);
            }
            match obj.code_point {
                codepoints::OPNQRYRM => {
                    trace!("Received OPNQRYRM");
                    let reply = db2_proto::replies::opnqryrm::parse_opnqryrm(&obj)
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                    if !reply.is_success() {
                        return Err(Error::Sql {
                            sqlstate: "HY000".into(),
                            sqlcode: -(reply.severity_code as i32),
                            message: "Open query failed".into(),
                        });
                    }
                    if reply.query_instance_id.is_some() {
                        *query_instance_id = reply.query_instance_id;
                    }
                }
                codepoints::QRYDSC => {
                    trace!("Received QRYDSC");
                    if let Ok(descriptors) = db2_proto::fdoca::parse_qrydsc(&obj.data) {
                        if !descriptors.is_empty() {
                            if debug_hex_enabled() {
                                eprintln!(
                                    "[db2-wire] parsed {} QRYDSC descriptor(s)",
                                    descriptors.len()
                                );
                            }
                            *qrydsc_descriptors = Some(descriptors);
                        } else if debug_hex_enabled() {
                            eprintln!("[db2-wire] parsed 0 QRYDSC descriptor(s)");
                        }
                    } else if debug_hex_enabled() {
                        eprintln!("[db2-wire] QRYDSC parse failed");
                    }
                }
                codepoints::QRYDTA => {
                    trace!("Received QRYDTA");
                    let active_descriptors =
                        sqldard_descriptors.as_ref().or(qrydsc_descriptors.as_ref());
                    if let Some(descs) = active_descriptors {
                        if descs.is_empty() {
                            continue;
                        }
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] query QRYDTA preview={} descriptors={:?}",
                                format_hex_preview(&obj.data, 128),
                                descs
                            );
                            if obj.data.len() > 33_000 {
                                let mid = 32_740usize.min(obj.data.len());
                                let end = (mid + 128).min(obj.data.len());
                                eprintln!(
                                    "[db2-wire] query QRYDTA mid[{}..{}]={}",
                                    mid,
                                    end,
                                    format_hex_preview(&obj.data[mid..end], 128)
                                );
                            }
                        }
                        let decoded_rows = db2_proto::fdoca::decode_rows_with_tail(
                            &obj.data,
                            descs,
                            pending_row_bytes,
                        )
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                        for values in decoded_rows {
                            let col_names = row_column_names(column_info, values.len());
                            rows.push(Row::new(col_names, values));
                        }
                    }
                }
                codepoints::ENDQRYRM => {
                    trace!("Received ENDQRYRM");
                    *end_of_query = true;
                }
                codepoints::SQLDARD => {
                    trace!("Received SQLDARD in query reply");
                    if debug_hex_enabled() {
                        eprintln!(
                            "[db2-wire] query SQLDARD preview={}",
                            format_hex_preview(&obj.data, 192)
                        );
                    }
                    let descriptors = parse_sqldard_descriptors(&obj);
                    if !descriptors.is_empty() {
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] parsed {} SQLDARD descriptor(s) in query reply",
                                descriptors.len()
                            );
                        }
                        *sqldard_descriptors = Some(descriptors);
                    } else if debug_hex_enabled() {
                        eprintln!("[db2-wire] parsed 0 SQLDARD descriptor(s) in query reply");
                    }
                }
                codepoints::SQLCARD => {
                    trace!("Received SQLCARD in query reply");
                    let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                    if card.is_error() {
                        return Err(Error::Sql {
                            sqlstate: card.sqlstate,
                            sqlcode: card.sqlcode,
                            message: if card.sqlerrmc.is_empty() {
                                format!("Query failed: SQLCODE={}", card.sqlcode)
                            } else {
                                card.sqlerrmc
                            },
                        });
                    }
                }
                _ => {
                    trace!("Ignoring reply codepoint 0x{:04X}", obj.code_point);
                }
            }
        }
    }

    Ok(())
}

pub(crate) fn protocol_reply_error(obj: &DdmObject, context: &str) -> Option<Error> {
    let name = reply_codepoint_name(obj.code_point)?;
    let detail = reply_detail(obj);

    let message = if detail.is_empty() {
        format!("{context} failed with {name}")
    } else {
        format!("{context} failed with {name}: {detail}")
    };

    match obj.code_point {
        codepoints::SQLERRRM => Some(Error::Sql {
            sqlstate: "HY000".into(),
            sqlcode: -1,
            message,
        }),
        _ => Some(Error::Protocol(message)),
    }
}

fn reply_codepoint_name(code_point: u16) -> Option<&'static str> {
    match code_point {
        codepoints::SYNTAXRM => Some("SYNTAXRM"),
        codepoints::PRCCNVRM => Some("PRCCNVRM"),
        codepoints::SQLERRRM => Some("SQLERRRM"),
        codepoints::CMDCHKRM => Some("CMDCHKRM"),
        codepoints::DTAMCHRM => Some("DTAMCHRM"),
        codepoints::OBJNSPRM => Some("OBJNSPRM"),
        codepoints::RDBNACRM => Some("RDBNACRM"),
        _ => None,
    }
}

fn reply_detail(obj: &DdmObject) -> String {
    let params = obj.parameters();
    if params.is_empty() {
        return format!(
            "codepoint=0x{:04X} data={}",
            obj.code_point,
            format_hex_preview(&obj.data, 96)
        );
    }

    params
        .into_iter()
        .map(|param| {
            format!(
                "0x{:04X}={}",
                param.code_point,
                format_hex_preview(&param.data, 32)
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn row_column_names(column_info: &[ColumnInfo], value_count: usize) -> Vec<String> {
    if column_info.len() == value_count && !column_info.is_empty() {
        return column_info.iter().map(|c| c.name.clone()).collect();
    }

    (0..value_count)
        .map(|i| {
            column_info
                .get(i)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| format!("COL{}", i + 1))
        })
        .collect()
}

fn column_info_from_descriptors(
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Vec<ColumnInfo> {
    descriptors
        .iter()
        .enumerate()
        .map(|(index, descriptor)| ColumnInfo {
            name: format!("COL{}", index + 1),
            type_name: format!("{:?}", descriptor.db2_type),
            nullable: descriptor.nullable,
            precision: if descriptor.precision > 0 {
                Some(descriptor.precision as u16)
            } else {
                None
            },
            scale: if descriptor.scale > 0 {
                Some(descriptor.scale as u16)
            } else {
                None
            },
        })
        .collect()
}

fn debug_hex_enabled() -> bool {
    env::var_os("DB2_WIRE_DEBUG_HEX").is_some()
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

/// Simple heuristic to determine if a SQL string is a query (SELECT).
fn sql_is_query(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("SELECT")
        || trimmed.starts_with("WITH")
        || trimmed.starts_with("VALUES")
        || trimmed.starts_with("CALL")
}
