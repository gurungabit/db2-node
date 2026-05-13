use bytes::BytesMut;
use std::collections::{HashMap, HashSet};
use std::env;
use std::sync::{Arc, Mutex as StdMutex, OnceLock, Weak};
use std::time::{Duration, Instant};
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
pub(crate) const ZOS_DIRECT_QUERY_SECTION: u16 = 1;
// DB2 CLI binds large placeholder packages as SYSLHxyy. Using the first one gives
// long-lived prepared statements their own section space instead of colliding with
// the one-shot section we keep for direct query()/execute() calls.
pub(crate) const PREPARED_STATEMENT_PKGID: &str = "SYSLH200";
pub(crate) const PREPARED_STATEMENT_MAX_SECTION: u16 = 385;
const ZOS_SELECT_CACHE_MAX_ENTRIES: usize = 64;
const ZOS_SELECT_METADATA_CACHE_MAX_ENTRIES: usize = 256;
const ZOS_NON_LOB_QRYBLKSZ_MIN: usize = 32_767;
const ZOS_NON_LOB_QRYBLKSZ_STEP: usize = 32_768;
const ZOS_NON_LOB_QRYBLKSZ_DEFAULT: usize = 262_143;
const ZOS_NON_LOB_QRYBLKSZ_MAX: usize = 1_048_575;

pub(crate) struct PoolCheckoutEntry {
    pub(crate) created_at: std::time::Instant,
    pub(crate) _permit: OwnedSemaphorePermit,
}

pub(crate) type PoolCheckoutMap = HashMap<usize, PoolCheckoutEntry>;

struct PoolCheckoutHandle {
    key: usize,
    checked_out: Weak<Mutex<PoolCheckoutMap>>,
}

#[derive(Clone)]
struct CachedZosSelect {
    package_id: &'static str,
    section_number: u16,
    pkgnamcsn: Arc<[u8]>,
    column_info: Vec<ColumnInfo>,
    result_descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
    query_instance_id: Option<Arc<[u8]>>,
    pipeline_fetch_after_open: bool,
}

#[derive(Clone)]
struct CachedZosSelectMetadata {
    column_info: Vec<ColumnInfo>,
    result_descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
}

struct ZosSelectOpenResult {
    result: QueryResult,
    query_instance_id: Option<Vec<u8>>,
    pipeline_fetch_after_open: bool,
}

type LobInitialGridRows = (
    Vec<Vec<db2_proto::types::Db2Value>>,
    Vec<Vec<Option<usize>>>,
);

static ZOS_SELECT_METADATA_CACHE: OnceLock<StdMutex<HashMap<String, CachedZosSelectMetadata>>> =
    OnceLock::new();
static ZOS_SELECT_LOB_CACHE_DENYLIST: OnceLock<StdMutex<HashSet<String>>> = OnceLock::new();

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
    pub zos_lob_internal_depth: usize,
    connection_diagnostics: Vec<String>,
    zos_select_cache: HashMap<String, CachedZosSelect>,
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
        let section_number = if self.server_info.as_ref().is_some_and(is_db2_zos_server) {
            ZOS_DIRECT_QUERY_SECTION
        } else {
            DIRECT_QUERY_SECTION
        };
        self.activate_section(DIRECT_QUERY_PKGID, section_number);
        self.build_pkgnamcsn_for(DIRECT_QUERY_PKGID, section_number)
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

    fn zos_select_metadata_cache_key(&self, sql: &str) -> String {
        let server = self
            .server_info
            .as_ref()
            .map(|info| {
                format!(
                    "{}:{}",
                    info.server_class.trim(),
                    info.server_release.trim()
                )
            })
            .unwrap_or_default();
        format!(
            "{}\n{}\n{}\n{}",
            server,
            self.config.database.trim(),
            self.config.current_schema.as_deref().unwrap_or("").trim(),
            sql.trim()
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

    fn allocate_zos_cached_select_section(&mut self) -> Option<u16> {
        loop {
            let section_number = self.allocate_prepared_section().ok()?;
            if section_number != ZOS_DIRECT_QUERY_SECTION {
                return Some(section_number);
            }
        }
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
        while DssReader::first_complete_frame_len(&self.recv_buf).is_none() {
            transport.read_bytes(&mut self.recv_buf).await?;
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

    pub(crate) async fn read_prepare_reply_frames(&mut self) -> Result<Vec<DssFrame>, Error> {
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

        Ok(frames)
    }

    async fn read_zos_select_prepare_reply_frames(&mut self) -> Result<Vec<DssFrame>, Error> {
        let mut frames = self.read_reply_frames().await?;
        if prepare_frames_have_result_metadata(&frames) {
            return Ok(frames);
        }

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
            if prepare_frames_have_result_metadata(&frames) {
                break;
            }
        }

        Ok(frames)
    }

    async fn drain_zos_open_reply_frames(
        &mut self,
        frames: &mut Vec<DssFrame>,
        wait_for_data_or_terminal_reply: bool,
    ) -> Result<(), Error> {
        let drain_timeout = if wait_for_data_or_terminal_reply {
            zos_non_lob_open_data_drain_timeout()
        } else {
            zos_non_lob_open_drain_timeout()
        };
        if drain_timeout.is_zero() {
            return Ok(());
        }
        if wait_for_data_or_terminal_reply && frames_have_data_or_terminal_reply(frames) {
            return Ok(());
        }

        loop {
            let more_frames = match timeout(drain_timeout, self.read_reply_frames()).await {
                Ok(Ok(frames)) => frames,
                Ok(Err(err)) => return Err(err),
                Err(_) => break,
            };

            if more_frames.is_empty() {
                break;
            }
            frames.extend(more_frames);
            if wait_for_data_or_terminal_reply && frames_have_data_or_terminal_reply(frames) {
                break;
            }
        }

        Ok(())
    }

    async fn drain_zos_cached_fetch_reply_frames(
        &mut self,
        frames: &mut Vec<DssFrame>,
    ) -> Result<(), Error> {
        let drain_timeout = zos_non_lob_cached_fetch_drain_timeout();
        if drain_timeout.is_zero() || frames_have_query_data_or_query_end_reply(frames) {
            return Ok(());
        }

        loop {
            let more_frames = match timeout(drain_timeout, self.read_reply_frames()).await {
                Ok(Ok(frames)) => frames,
                Ok(Err(err)) => return Err(err),
                Err(_) => break,
            };

            if more_frames.is_empty() {
                break;
            }
            frames.extend(more_frames);
            if frames_have_query_data_or_query_end_reply(frames) {
                break;
            }
        }

        Ok(())
    }

    fn frame_drain_timeout(&self) -> Duration {
        let timeout = self.config.frame_drain_timeout;
        if self.zos_lob_internal_depth > 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
        {
            return timeout.max(zos_lob_frame_drain_timeout());
        }
        timeout
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
        self.zos_lob_internal_depth = 0;
        self.zos_select_cache.clear();

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
        ) || error_indicates_stale_session_state(&err)
        {
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
        let use_zos_sqlstt = self.server_info.as_ref().is_some_and(is_db2_zos_server);
        let pkgnamcsn = self.direct_query_pkgnamcsn();
        // Use EXCSQLIMM (0x200A) for non-query SQL execution
        let exec_data = if self.auto_commit {
            db2_proto::commands::excsqlimm::build_excsqlimm_autocommit(&pkgnamcsn)
        } else {
            db2_proto::commands::excsqlimm::build_excsqlimm_default(&pkgnamcsn)
        };
        let sqlstt_data = build_sqlstt_for_server(sql, use_zos_sqlstt);
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
            let column_info = column_info_with_select_aliases(
                sql,
                self.parse_prepare_reply(&frames).unwrap_or_default(),
            );
            self.process_query_reply(&frames, sql, &column_info, None)
                .await
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
        let collect_diagnostics = query_diagnostics_enabled();
        self.connection_diagnostics.clear();
        let connect_total_started = collect_diagnostics.then(Instant::now);

        let transport_started = collect_diagnostics.then(Instant::now);
        let mut transport = Transport::connect_with_diagnostics(
            &self.config,
            collect_diagnostics.then_some(&mut self.connection_diagnostics),
        )
        .await?;
        if let Some(started) = transport_started {
            self.connection_diagnostics.push(format!(
                "db2_connect_transport_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            ));
        }

        let auth_started = collect_diagnostics.then(Instant::now);
        let (server_info, next_corr_id) = match auth::authenticate(
            &mut transport,
            &self.config,
            auth::AccsecRdbnamMode::Trimmed,
            collect_diagnostics.then_some(&mut self.connection_diagnostics),
        )
        .await
        {
            Ok(result) => {
                if let Some(started) = auth_started {
                    self.connection_diagnostics.push(format!(
                        "db2_connect_auth_ms={:.3}",
                        started.elapsed().as_secs_f64() * 1000.0
                    ));
                }
                result
            }
            Err(err) if Self::should_retry_accsec_with_luw_legacy_handshake(&err) => {
                trace!(
                    "Retrying authentication with LUW legacy handshake after trimmed RDBNAM was rejected: {}",
                    err
                );
                if collect_diagnostics {
                    self.connection_diagnostics
                        .push("db2_connect_auth_retry=luw_legacy".to_string());
                }
                let retry_transport_started = collect_diagnostics.then(Instant::now);
                transport = Transport::connect_with_diagnostics(
                    &self.config,
                    collect_diagnostics.then_some(&mut self.connection_diagnostics),
                )
                .await?;
                if let Some(started) = retry_transport_started {
                    self.connection_diagnostics.push(format!(
                        "db2_connect_retry_transport_ms={:.3}",
                        started.elapsed().as_secs_f64() * 1000.0
                    ));
                }
                let retry_auth_started = collect_diagnostics.then(Instant::now);
                let result = auth::authenticate(
                    &mut transport,
                    &self.config,
                    auth::AccsecRdbnamMode::LuwLegacy,
                    collect_diagnostics.then_some(&mut self.connection_diagnostics),
                )
                .await?;
                if let Some(started) = retry_auth_started {
                    self.connection_diagnostics.push(format!(
                        "db2_connect_retry_auth_ms={:.3}",
                        started.elapsed().as_secs_f64() * 1000.0
                    ));
                }
                result
            }
            Err(Error::Connection(msg)) if msg.to_lowercase().contains("closed by server") => {
                return Err(Error::Connection(
                    "RDB not accessed or database not found".into(),
                ));
            }
            Err(err) => return Err(err),
        };

        self.transport = Some(transport);
        let skip_post_auth_init = is_db2_zos_server(&server_info);
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
        self.zos_select_cache.clear();
        self.session_generation = self.session_generation.wrapping_add(1);
        if self.session_generation == 0 {
            self.session_generation = 1;
        }

        if skip_post_auth_init {
            debug!("Skipping LUW-style post-auth init for Db2 for z/OS server");
        } else {
            if let Err(err) = self.post_auth_init().await {
                self.reset_session_state(false).await;
                return Err(err);
            }
        }

        if let Some(started) = connect_total_started {
            self.connection_diagnostics.push(format!(
                "db2_connect_total_ms={:.3} server_class={} server_release={}",
                started.elapsed().as_secs_f64() * 1000.0,
                self.server_info
                    .as_ref()
                    .map(|info| info.server_class.trim())
                    .unwrap_or(""),
                self.server_info
                    .as_ref()
                    .map(|info| info.server_release.trim())
                    .unwrap_or("")
            ));
        }

        debug!("Client connected to DB2 server");
        Ok(())
    }

    fn should_retry_accsec_with_luw_legacy_handshake(err: &Error) -> bool {
        match err {
            Error::Connection(msg) => {
                let msg = msg.to_lowercase();
                msg.contains("rdb not accessed") || msg.contains("database not found")
            }
            Error::Protocol(msg) => msg.contains("Expected ACCSECRD, got 0x2211"),
            _ => false,
        }
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
        let use_zos_sqlstt = self.server_info.as_ref().is_some_and(is_db2_zos_server);
        let prefer_zos_excsqlstt_output = params.is_empty()
            && is_query
            && use_zos_sqlstt
            && self.zos_lob_internal_depth == 0
            && sql_prefers_zos_non_lob_excsqlstt_output(sql);
        let optimized_zos_sql = if params.is_empty()
            && is_query
            && use_zos_sqlstt
            && self.zos_lob_internal_depth == 0
            && !prefer_zos_excsqlstt_output
        {
            optimize_zos_select_sql(sql)
        } else {
            None
        };
        let sql = optimized_zos_sql.as_deref().unwrap_or(sql);
        ensure_sqlstt_sql_len(sql)?;
        let use_zos_cursor_attributes = is_query
            && use_zos_sqlstt
            && use_zos_read_only_cursor_attributes()
            && !prefer_zos_excsqlstt_output;
        let pkgnamcsn = self.direct_query_pkgnamcsn();
        let mut input_descriptors = Vec::new();

        if is_query {
            // Prepare first, then open the query explicitly. This is a little
            // more chatty than chaining OPNQRY onto the prepare request, but it
            // is much more reliable for large multi-segment SQLSTT payloads.
            let corr_id = self.next_correlation_id();
            let prpsqlstt_data =
                db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn);
            let sqlstt_data = build_sqlstt_for_server(sql, use_zos_sqlstt);
            let qryblksz: u32 = 0x0000FFFF;

            if params.is_empty() && use_zos_sqlstt {
                let collect_diagnostics = query_diagnostics_enabled();
                let mut deferred_zos_prepare_diagnostics = Vec::new();
                let global_metadata_cache_key = (self.zos_lob_internal_depth == 0
                    && use_zos_select_metadata_cache())
                .then(|| self.zos_select_metadata_cache_key(sql));
                let zos_select_lob_cache_denied = global_metadata_cache_key
                    .as_deref()
                    .is_some_and(zos_select_lob_cache_denied);
                if zos_select_lob_cache_denied {
                    if let Some(cached) = self.zos_select_cache.remove(sql) {
                        self.release_prepared_section(cached.section_number);
                    }
                }
                if self.zos_lob_internal_depth == 0
                    && use_zos_select_cache()
                    && !zos_select_lob_cache_denied
                {
                    if let Some(cached) = self.zos_select_cache.get(sql).cloned() {
                        if !zos_select_section_cacheable(
                            &cached.column_info,
                            &cached.result_descriptors,
                        ) {
                            self.zos_select_cache.remove(sql);
                            self.release_prepared_section(cached.section_number);
                        } else {
                            self.activate_section(cached.package_id, cached.section_number);
                            let cached_section_number = cached.section_number;
                            let cached_query_instance_id = cached.query_instance_id.clone();
                            let result = self
                                .open_zos_select(
                                    sql,
                                    &cached.pkgnamcsn,
                                    &cached.column_info,
                                    &cached.result_descriptors,
                                    cached_query_instance_id.as_deref(),
                                    cached.pipeline_fetch_after_open,
                                )
                                .await;
                            match result {
                                Ok(opened) => {
                                    let cached_result_has_lobs =
                                        result_has_zos_lob_materialization(&opened.result);
                                    if opened.result.rows.is_empty()
                                        && use_zos_select_cached_empty_retry()
                                    {
                                        self.zos_select_cache.remove(sql);
                                        self.release_prepared_section(cached.section_number);
                                        let metadata_cache_removed = global_metadata_cache_key
                                            .as_deref()
                                            .is_some_and(remove_zos_select_metadata);
                                        if collect_diagnostics {
                                            deferred_zos_prepare_diagnostics.push(format!(
                                                "zos_prepare_cache_hit_empty_retry=true section={} metadata_cache_removed={}",
                                                cached_section_number, metadata_cache_removed
                                            ));
                                        }
                                    } else if cached_result_has_lobs {
                                        self.zos_select_cache.remove(sql);
                                        self.release_prepared_section(cached.section_number);
                                        if let Some(metadata_key) =
                                            global_metadata_cache_key.as_deref()
                                        {
                                            mark_zos_select_lob_cache_denied(metadata_key);
                                            remove_zos_select_metadata(metadata_key);
                                        }
                                        if opened.result.rows.is_empty() {
                                            if collect_diagnostics {
                                                deferred_zos_prepare_diagnostics.push(format!(
                                                    "zos_prepare_cache_hit_lob_retry=true section={}",
                                                    cached_section_number
                                                ));
                                            }
                                        } else {
                                            let mut result = opened.result;
                                            if collect_diagnostics {
                                                result.diagnostics.push(format!(
                                                    "zos_prepare_cache_hit=true cache=connection section={} dropped_lob_cache=true",
                                                    cached_section_number
                                                ));
                                            }
                                            return Ok(result);
                                        }
                                    } else {
                                        if let Some(cached) = self.zos_select_cache.get_mut(sql) {
                                            if let Some(query_instance_id) =
                                                opened.query_instance_id
                                            {
                                                cached.query_instance_id =
                                                    Some(query_instance_id.into());
                                            }
                                            cached.pipeline_fetch_after_open =
                                                opened.pipeline_fetch_after_open;
                                        }
                                        let mut result = opened.result;
                                        if collect_diagnostics {
                                            result.diagnostics.push(format!(
                                            "zos_prepare_cache_hit=true cache=connection section={}",
                                            cached_section_number
                                        ));
                                        }
                                        return Ok(result);
                                    }
                                }
                                Err(err) if should_reprepare_cached_zos_select(&err) => {
                                    self.zos_select_cache.remove(sql);
                                    self.release_prepared_section(cached.section_number);
                                }
                                Err(err) => return Err(err),
                            }
                        }
                    }
                }

                let cache_section = if self.zos_lob_internal_depth == 0
                    && use_zos_select_cache()
                    && !zos_select_lob_cache_denied
                    && self.zos_select_cache.len() < ZOS_SELECT_CACHE_MAX_ENTRIES
                {
                    self.allocate_zos_cached_select_section()
                } else {
                    None
                };
                let use_dedicated_zos_select_section =
                    sql_uses_zos_like_predicate_large_package(sql);
                let one_shot_section =
                    if cache_section.is_none() && use_dedicated_zos_select_section {
                        self.allocate_zos_cached_select_section()
                    } else {
                        None
                    };
                let allocated_section = cache_section.or(one_shot_section);
                let (package_id, section_number) =
                    if use_dedicated_zos_select_section && allocated_section.is_some() {
                        (
                            PREPARED_STATEMENT_PKGID,
                            allocated_section.unwrap_or(ZOS_DIRECT_QUERY_SECTION),
                        )
                    } else {
                        (
                            DIRECT_QUERY_PKGID,
                            allocated_section.unwrap_or(ZOS_DIRECT_QUERY_SECTION),
                        )
                    };
                self.activate_section(package_id, section_number);
                let pkgnamcsn = self.build_pkgnamcsn_for(package_id, section_number);
                let global_cached_metadata = (!zos_select_lob_cache_denied)
                    .then(|| {
                        global_metadata_cache_key
                            .as_deref()
                            .and_then(lookup_zos_select_metadata)
                    })
                    .flatten();
                let global_metadata_cache_hit = global_cached_metadata.is_some();
                let prepare_total_started = collect_diagnostics.then(Instant::now);
                let mut zos_prepare_diagnostics = Vec::new();
                if collect_diagnostics {
                    zos_prepare_diagnostics.push(format!(
                        "zos_prepare_plan package={} section={} cache_section={} metadata_cache_hit={} request_sqlda={} lob_cache_denied={}",
                        package_id,
                        section_number,
                        cache_section.is_some(),
                        global_metadata_cache_hit,
                        !global_metadata_cache_hit,
                        zos_select_lob_cache_denied
                    ));
                    zos_prepare_diagnostics.extend(deferred_zos_prepare_diagnostics);
                }
                let prpsqlstt_data = if global_cached_metadata.is_some() {
                    db2_proto::commands::prpsqlstt::build_prpsqlstt_without_sqlda(&pkgnamcsn)
                } else {
                    db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn)
                };

                let mut writer = DssWriter::new(corr_id);
                writer.write_request_next_same_corr(&prpsqlstt_data, true);
                if use_zos_cursor_attributes {
                    let sqlattr_data =
                        db2_proto::commands::sqlattr::build_sqlattr_for_read_only_cursor();
                    writer.write_object_same_corr(&sqlattr_data, true);
                }
                writer.write_object(&sqlstt_data, false);

                let send_buf = writer.finish();
                let prepare_send_started = collect_diagnostics.then(Instant::now);
                if let Err(err) = self.send_bytes(&send_buf).await {
                    if let Some(section_number) = allocated_section {
                        self.release_prepared_section(section_number);
                    }
                    return Err(err);
                }
                if let Some(started) = prepare_send_started {
                    zos_prepare_diagnostics.push(format!(
                        "zos_prepare_send_ms={:.3} bytes={}",
                        started.elapsed().as_secs_f64() * 1000.0,
                        send_buf.len()
                    ));
                }

                let prepare_read_started = collect_diagnostics.then(Instant::now);
                let frames = match if global_cached_metadata.is_some() {
                    self.read_reply_frames().await
                } else {
                    self.read_zos_select_prepare_reply_frames().await
                } {
                    Ok(frames) => frames,
                    Err(err) => {
                        if let Some(section_number) = allocated_section {
                            self.release_prepared_section(section_number);
                        }
                        return Err(err);
                    }
                };
                if let Some(started) = prepare_read_started {
                    zos_prepare_diagnostics.push(format!(
                        "zos_prepare_read_ms={:.3} frames={} metadata_cache_hit={}",
                        started.elapsed().as_secs_f64() * 1000.0,
                        frames.len(),
                        global_metadata_cache_hit
                    ));
                }
                let prepare_parse_started = collect_diagnostics.then(Instant::now);
                let (column_info, result_descriptors) = match global_cached_metadata {
                    Some(metadata) => {
                        if let Err(err) = self.parse_prepare_reply(&frames) {
                            if let Some(section_number) = allocated_section {
                                self.release_prepared_section(section_number);
                            }
                            return Err(err);
                        }
                        (
                            column_info_with_select_aliases(sql, metadata.column_info),
                            metadata.result_descriptors,
                        )
                    }
                    None => match self.parse_prepare_reply(&frames) {
                        Ok(column_info) => {
                            let column_info = column_info_with_select_aliases(sql, column_info);
                            let result_descriptors = self.parse_prepare_result_descriptors(&frames);
                            if let Some(cache_key) = global_metadata_cache_key.as_deref() {
                                let metadata_cache_stored = store_zos_select_metadata(
                                    cache_key,
                                    &column_info,
                                    &result_descriptors,
                                );
                                if collect_diagnostics {
                                    zos_prepare_diagnostics.push(format!(
                                        "zos_prepare_metadata_cache_store={} columns={} descriptors={}",
                                        metadata_cache_stored,
                                        column_info.len(),
                                        result_descriptors.len()
                                    ));
                                }
                            }
                            (column_info, result_descriptors)
                        }
                        Err(err) => {
                            if let Some(section_number) = allocated_section {
                                self.release_prepared_section(section_number);
                            }
                            return Err(err);
                        }
                    },
                };
                if let Some(started) = prepare_parse_started {
                    zos_prepare_diagnostics.push(format!(
                        "zos_prepare_parse_ms={:.3} columns={} descriptors={} metadata_cache_hit={}",
                        started.elapsed().as_secs_f64() * 1000.0,
                        column_info.len(),
                        result_descriptors.len(),
                        global_metadata_cache_hit
                    ));
                }
                if let Some(started) = prepare_total_started {
                    zos_prepare_diagnostics.push(format!(
                        "zos_prepare_total_ms={:.3}",
                        started.elapsed().as_secs_f64() * 1000.0
                    ));
                }
                let result = self
                    .open_zos_select(
                        sql,
                        &pkgnamcsn,
                        &column_info,
                        &result_descriptors,
                        None,
                        false,
                    )
                    .await;
                match result {
                    Ok(opened) => {
                        let mut result = opened.result;
                        let opened_result_has_lobs = result_has_zos_lob_materialization(&result);
                        if opened_result_has_lobs {
                            let metadata_cache_evicted =
                                global_metadata_cache_key.as_deref().is_some_and(|key| {
                                    mark_zos_select_lob_cache_denied(key);
                                    remove_zos_select_metadata(key)
                                });
                            if collect_diagnostics {
                                result.diagnostics.push(format!(
                                    "zos_prepare_metadata_cache_evict_lob={}",
                                    metadata_cache_evicted
                                ));
                            }
                        }
                        result.diagnostics.extend(zos_prepare_diagnostics);
                        if let Some(section_number) = allocated_section {
                            let prepare_section_cacheable =
                                zos_select_section_cacheable(&column_info, &result_descriptors);
                            let section_cacheable = cache_section.is_some()
                                && prepare_section_cacheable
                                && !opened_result_has_lobs;
                            if collect_diagnostics {
                                result.diagnostics.push(format!(
                                    "zos_prepare_section_cache_store={} has_lobs={} prepare_has_lobs={} opened_has_lobs={}",
                                    section_cacheable,
                                    !section_cacheable,
                                    !prepare_section_cacheable,
                                    opened_result_has_lobs
                                ));
                            }
                            if section_cacheable {
                                let query_instance_id = opened
                                    .query_instance_id
                                    .as_ref()
                                    .map(|value| Arc::<[u8]>::from(value.as_slice()));
                                self.zos_select_cache.insert(
                                    sql.to_string(),
                                    CachedZosSelect {
                                        package_id,
                                        section_number,
                                        pkgnamcsn: pkgnamcsn.clone().into(),
                                        column_info,
                                        result_descriptors,
                                        query_instance_id,
                                        pipeline_fetch_after_open: opened.pipeline_fetch_after_open,
                                    },
                                );
                            } else {
                                self.release_prepared_section(section_number);
                            }
                        }
                        return Ok(result);
                    }
                    Err(err) => {
                        if let Some(section_number) = allocated_section {
                            self.release_prepared_section(section_number);
                        }
                        return Err(err);
                    }
                }
            }

            let mut writer = DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&prpsqlstt_data, true);
            if use_zos_cursor_attributes {
                let sqlattr_data =
                    db2_proto::commands::sqlattr::build_sqlattr_for_read_only_cursor();
                writer.write_object_same_corr(&sqlattr_data, true);
            }
            writer.write_object(&sqlstt_data, false);

            let send_buf = writer.finish();
            self.send_bytes(&send_buf).await?;

            let frames = self.read_prepare_reply_frames().await?;
            let column_info =
                column_info_with_select_aliases(sql, self.parse_prepare_reply(&frames)?);
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
                    .process_query_reply(&frames, sql, &column_info, Some(&result_descriptors))
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
            self.process_query_reply(&frames, sql, &column_info, Some(&result_descriptors))
                .await
        } else {
            // For DML: PRPSQLSTT + SQLSTT first, then EXCSQLSTT + SQLDTA
            let corr_id = self.next_correlation_id();
            let prpsqlstt_data =
                db2_proto::commands::prpsqlstt::build_prpsqlstt_with_sqlda(&pkgnamcsn);
            let sqlstt_data = build_sqlstt_for_server(sql, use_zos_sqlstt);

            let mut writer = DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&prpsqlstt_data, true);
            if use_zos_cursor_attributes {
                let sqlattr_data =
                    db2_proto::commands::sqlattr::build_sqlattr_for_read_only_cursor();
                writer.write_object_same_corr(&sqlattr_data, true);
            }
            writer.write_object(&sqlstt_data, false);

            let send_buf = writer.finish();
            self.send_bytes(&send_buf).await?;

            let frames = self.read_prepare_reply_frames().await?;
            let _column_info = self.parse_prepare_reply(&frames)?;
            if !params.is_empty() {
                input_descriptors = self.describe_input(&pkgnamcsn).await?;
            }

            self.execute_with_params(&pkgnamcsn, params, &input_descriptors)
                .await
        }
    }

    async fn open_zos_select(
        &mut self,
        sql: &str,
        pkgnamcsn: &[u8],
        column_info: &[ColumnInfo],
        result_descriptors: &[db2_proto::fdoca::ColumnDescriptor],
        cached_query_instance_id: Option<&[u8]>,
        cached_pipeline_fetch_after_open: bool,
    ) -> Result<ZosSelectOpenResult, Error> {
        if self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !use_zos_native_lob_only()
        {
            let current_schema = self.config.current_schema.clone();
            let prepare_columns =
                catalog_columns_from_prepare_metadata(column_info, result_descriptors);
            if let Some(result) = self
                .execute_zos_select_lobs_chunked_with_catalog(
                    sql,
                    current_schema.as_deref(),
                    &prepare_columns,
                    "prepare",
                )
                .await?
            {
                return Ok(ZosSelectOpenResult {
                    result,
                    query_instance_id: None,
                    pipeline_fetch_after_open: false,
                });
            }

            if result_metadata_needs_zos_lob_route(column_info, result_descriptors) {
                if let Some(metadata_sql) =
                    build_zos_select_star_metadata_query(sql, current_schema.as_deref())
                {
                    let metadata = self
                        .execute_zos_lob_internal_query("metadata-after-prepare", &metadata_sql)
                        .await?;
                    if let Some(result) = self
                        .execute_zos_select_star_lobs_chunked(
                            sql,
                            current_schema.as_deref(),
                            &metadata,
                        )
                        .await?
                    {
                        return Ok(ZosSelectOpenResult {
                            result,
                            query_instance_id: None,
                            pipeline_fetch_after_open: false,
                        });
                    }
                }
            }
        }

        let has_zos_lobs = result_metadata_needs_zos_lob_route(column_info, result_descriptors);
        if self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && sql_prefers_zos_non_lob_excsqlstt_output(sql)
        {
            return self
                .execute_zos_select_with_excsqlstt_output(
                    sql,
                    pkgnamcsn,
                    column_info,
                    result_descriptors,
                )
                .await;
        }

        let use_extended_materialized_blocks = self.zos_lob_internal_depth > 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server);
        let use_zos_non_lob_extra_blocks = self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && use_zos_non_lob_extra_blocks();
        let fetch_size_override = if self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && use_zos_non_lob_sql_rowset_cap()
        {
            parse_fetch_first_row_limit(sql)
                .and_then(|limit| u32::try_from(limit).ok())
                .map(|limit| limit.min(self.config.fetch_size.max(1)))
        } else {
            None
        };
        let zos_non_lob_open_rowset = if self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && use_zos_non_lob_open_rowset()
        {
            fetch_size_override
        } else {
            None
        };
        let zos_non_lob_limited_block_open = self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && fetch_size_override.is_some()
            && zos_non_lob_open_rowset.is_none();
        let pipeline_cached_fetch = self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && (cached_query_instance_id.is_some() || cached_pipeline_fetch_after_open)
            && (zos_non_lob_open_rowset.is_some() || cached_pipeline_fetch_after_open)
            && use_zos_non_lob_cached_open_fetch_pipeline();
        // If a cached statement already proved that OPNQRY does not carry the
        // first row block, move directly to CNTQRY on later opens.
        let learned_cntqry_after_open = cached_pipeline_fetch_after_open
            && !pipeline_cached_fetch
            && zos_non_lob_open_rowset.is_none();
        let qryblksz = if self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
        {
            zos_non_lob_qryblksz()
        } else {
            db2_proto::commands::opnqry::DEFAULT_QRYBLKSZ
        };
        let wait_for_open_data = self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && !has_zos_lobs
            && fetch_size_override.is_some()
            && (zos_non_lob_limited_block_open || use_zos_non_lob_open_data_drain());
        let wait_for_open_data = wait_for_open_data && !learned_cntqry_after_open;
        let collect_diagnostics = query_diagnostics_enabled();
        let mut open_diagnostics = Vec::new();
        if collect_diagnostics {
            open_diagnostics.push(format!(
                "zos_open_plan has_lobs={} cached_qryinsid={} cached_pipeline_after_open={} learned_cntqry_after_open={} pipeline={} fetch_size_override={} open_rowset={} limited_block_open={} wait_open_data={} non_lob_extra_blocks={} qryblksz={}",
                has_zos_lobs,
                cached_query_instance_id.is_some(),
                cached_pipeline_fetch_after_open,
                learned_cntqry_after_open,
                pipeline_cached_fetch,
                fetch_size_override
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                zos_non_lob_open_rowset
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "none".to_string()),
                zos_non_lob_limited_block_open,
                wait_for_open_data,
                use_zos_non_lob_extra_blocks,
                qryblksz
            ));
        }
        let opnqry_data = {
            let mut ddm = db2_proto::ddm::DdmBuilder::new(codepoints::OPNQRY);
            ddm.add_code_point(codepoints::PKGNAMCSN, pkgnamcsn);
            ddm.add_u32(codepoints::QRYBLKSZ, qryblksz);
            if let Some(rows) = zos_non_lob_open_rowset {
                ddm.add_u32(codepoints::QRYROWSET, rows);
            }
            if has_zos_lobs && use_native_zos_lob_strategy() {
                ddm.add_u16(codepoints::MAXBLKEXT, (-1i16) as u16);
                ddm.add_u16(codepoints::QRYPRCTYP, codepoints::QRYPRCTYP_LMTBLKPRC);
            } else if use_extended_materialized_blocks || use_zos_non_lob_extra_blocks {
                ddm.add_u16(codepoints::MAXBLKEXT, (-1i16) as u16);
            }
            ddm.add_code_point(0x215D, &[0x01]); // QRYCLSIMP = 1 (close on endqry)
            ddm.build()
        };

        let corr_id = self.next_correlation_id();
        let mut writer = DssWriter::new(corr_id);
        if pipeline_cached_fetch {
            let cntqry_data = db2_proto::commands::cntqry::build_cntqry(
                pkgnamcsn,
                cached_query_instance_id,
                qryblksz,
                use_zos_non_lob_extra_blocks.then_some(-1),
                fetch_size_override,
            );
            writer.write_request(&opnqry_data, true);
            writer.set_correlation_id(self.next_correlation_id());
            writer.write_request(&cntqry_data, false);
        } else {
            writer.write_request(&opnqry_data, false);
        }
        let send_buf = writer.finish();
        let send_started = collect_diagnostics.then(Instant::now);
        self.send_bytes(&send_buf).await?;
        if let Some(started) = send_started {
            open_diagnostics.push(format!(
                "zos_open_send_ms={:.3} bytes={}",
                started.elapsed().as_secs_f64() * 1000.0,
                send_buf.len()
            ));
        }

        let read_started = collect_diagnostics.then(Instant::now);
        let mut frames = self.read_reply_frames().await?;
        if let Some(started) = read_started {
            open_diagnostics.push(format!(
                "zos_open_first_read_ms={:.3} frames={}",
                started.elapsed().as_secs_f64() * 1000.0,
                frames.len()
            ));
        }
        let mut cached_fetch_pipeline_observed = false;
        if pipeline_cached_fetch {
            let frames_before_drain = frames.len();
            let drain_started = collect_diagnostics.then(Instant::now);
            self.drain_zos_cached_fetch_reply_frames(&mut frames)
                .await?;
            cached_fetch_pipeline_observed = frames_have_query_data_or_query_end_reply(&frames);
            if let Some(started) = drain_started {
                open_diagnostics.push(format!(
                    "zos_open_cached_fetch_drain_ms={:.3} frames_before={} frames_after={} has_query_reply={}",
                    started.elapsed().as_secs_f64() * 1000.0,
                    frames_before_drain,
                    frames.len(),
                    cached_fetch_pipeline_observed
                ));
            }
        } else if (use_extended_materialized_blocks || use_zos_non_lob_extra_blocks)
            && !has_zos_lobs
        {
            let frames_before_drain = frames.len();
            let drain_started = collect_diagnostics.then(Instant::now);
            self.drain_zos_open_reply_frames(&mut frames, wait_for_open_data)
                .await?;
            if let Some(started) = drain_started {
                open_diagnostics.push(format!(
                    "zos_open_drain_ms={:.3} frames_before={} frames_after={} has_data={} has_terminal={}",
                    started.elapsed().as_secs_f64() * 1000.0,
                    frames_before_drain,
                    frames.len(),
                    frames_have_query_data(&frames),
                    frames_have_data_or_terminal_reply(&frames)
                ));
            }
        }
        let pipeline_fetch_after_open = if pipeline_cached_fetch {
            cached_pipeline_fetch_after_open && cached_fetch_pipeline_observed
        } else {
            zos_non_lob_limited_block_open
                && !frames_have_query_data(&frames)
                && !frames_have_data_or_terminal_reply(&frames)
        };
        let query_instance_id = query_instance_id_from_frames(&frames)?;
        if collect_diagnostics {
            open_diagnostics.push(format!(
                "zos_open_observed qryinsid={} pipeline_after_open={} frames={} has_data={} has_terminal={}",
                query_instance_id.is_some(),
                pipeline_fetch_after_open,
                frames.len(),
                frames_have_query_data(&frames),
                frames_have_data_or_terminal_reply(&frames)
            ));
        }
        let process_started = collect_diagnostics.then(Instant::now);
        let result = self
            .process_query_reply_with_fetch_size(
                &frames,
                sql,
                column_info,
                Some(result_descriptors),
                fetch_size_override,
            )
            .await;
        if let Some(started) = process_started {
            open_diagnostics.push(format!(
                "zos_open_process_ms={:.3}",
                started.elapsed().as_secs_f64() * 1000.0
            ));
        }
        let retry_source = if use_native_zos_lob_strategy() {
            "native-error"
        } else {
            "direct-decode-error"
        };
        self.retry_zos_lob_chunking_after_decode_error(
            sql,
            result,
            column_info,
            result_descriptors,
            retry_source,
        )
        .await
        .map(|mut result| {
            result.diagnostics.extend(open_diagnostics);
            ZosSelectOpenResult {
                result,
                query_instance_id,
                pipeline_fetch_after_open,
            }
        })
    }

    async fn execute_zos_select_with_excsqlstt_output(
        &mut self,
        sql: &str,
        pkgnamcsn: &[u8],
        column_info: &[ColumnInfo],
        result_descriptors: &[db2_proto::fdoca::ColumnDescriptor],
    ) -> Result<ZosSelectOpenResult, Error> {
        let corr_id = self.next_correlation_id();
        let excsqlstt_data = db2_proto::commands::excsqlstt::build_excsqlstt_output(pkgnamcsn);
        let mut writer = DssWriter::new(corr_id);
        writer.write_request(&excsqlstt_data, false);
        let send_buf = writer.finish();
        self.send_bytes(&send_buf).await?;

        let frames = self.read_reply_frames().await?;
        let query_instance_id = query_instance_id_from_frames(&frames)?;
        let fetch_size_override = if use_zos_non_lob_sql_rowset_cap() {
            parse_fetch_first_row_limit(sql)
                .and_then(|limit| u32::try_from(limit).ok())
                .map(|limit| limit.min(self.config.fetch_size.max(1)))
        } else {
            None
        };
        let result = self
            .process_query_reply_with_fetch_size(
                &frames,
                sql,
                column_info,
                Some(result_descriptors),
                fetch_size_override,
            )
            .await?;

        Ok(ZosSelectOpenResult {
            result,
            query_instance_id,
            pipeline_fetch_after_open: false,
        })
    }

    async fn execute_query_with_reconnect_retry(
        &mut self,
        sql: &str,
        params: &[&dyn ToSql],
    ) -> Result<QueryResult, Error> {
        let mut attempts = 0usize;
        loop {
            match self.execute_query(sql, params).await {
                Ok(result) => return Ok(result),
                Err(err)
                    if attempts < 4
                        && should_retry_query_after_session_error(sql, params, &err) =>
                {
                    attempts += 1;
                    let original_error = err.to_string();
                    self.reset_session_state(false).await;
                    self.establish_session().await?;
                    if can_retry_zos_lob_query_from_catalog(
                        sql,
                        params,
                        self.zos_lob_internal_depth,
                        self.config.current_schema.as_deref(),
                        self.server_info.as_ref(),
                    ) {
                        loop {
                            match self
                                .execute_zos_select_lobs_chunked_from_catalog(
                                    sql,
                                    "session-error-retry",
                                )
                                .await
                            {
                                Ok(Some(mut result)) => {
                                    if query_diagnostics_enabled() {
                                        result.diagnostics.push(format!(
                                            "zos_lob_retry source=session-error reason={}",
                                            sanitize_diagnostic_value(&original_error)
                                        ));
                                    }
                                    return Ok(result);
                                }
                                Ok(None) => break,
                                Err(retry_err)
                                    if attempts < 4
                                        && should_retry_query_after_session_error(
                                            sql, params, &retry_err,
                                        ) =>
                                {
                                    attempts += 1;
                                    self.reset_session_state(false).await;
                                    self.establish_session().await?;
                                    continue;
                                }
                                Err(retry_err) => return Err(retry_err),
                            }
                        }
                    }
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn execute_zos_select_lobs_chunked_from_catalog(
        &mut self,
        sql: &str,
        source: &str,
    ) -> Result<Option<QueryResult>, Error> {
        if self.zos_lob_internal_depth > 0
            || !self.server_info.as_ref().is_some_and(is_db2_zos_server)
            || use_zos_native_lob_only()
        {
            return Ok(None);
        }

        let current_schema = self.config.current_schema.clone();
        let Some(metadata_sql) =
            build_zos_select_star_metadata_query(sql, current_schema.as_deref())
        else {
            return Ok(None);
        };

        let metadata_stage = format!("metadata-{source}");
        let metadata = self
            .execute_zos_lob_internal_query(&metadata_stage, &metadata_sql)
            .await?;
        let Some(mut result) = self
            .execute_zos_select_star_lobs_chunked(sql, current_schema.as_deref(), &metadata)
            .await?
        else {
            return Ok(None);
        };

        if query_diagnostics_enabled() {
            result
                .diagnostics
                .push(format!("zos_lob_catalog_route source={source}"));
        }
        Ok(Some(result))
    }

    async fn execute_zos_select_star_lobs_chunked(
        &mut self,
        sql: &str,
        current_schema: Option<&str>,
        metadata: &QueryResult,
    ) -> Result<Option<QueryResult>, Error> {
        let catalog_columns = catalog_columns_from_query_result(metadata);
        self.execute_zos_select_lobs_chunked_with_catalog(
            sql,
            current_schema,
            &catalog_columns,
            "catalog",
        )
        .await
    }

    async fn execute_zos_select_lobs_chunked_with_catalog(
        &mut self,
        sql: &str,
        current_schema: Option<&str>,
        catalog_columns: &[CatalogColumn],
        source: &str,
    ) -> Result<Option<QueryResult>, Error> {
        let Some(parsed) = parse_simple_select_for_zos_lobs(sql, current_schema) else {
            return Ok(None);
        };
        let Some(catalog_columns) = selected_catalog_columns(&parsed, catalog_columns) else {
            return Ok(None);
        };
        if !catalog_columns.iter().any(CatalogColumn::is_lob) {
            return Ok(None);
        }

        if let Some(result) = self
            .execute_zos_select_lobs_bounded_single_pass(&parsed, &catalog_columns, source)
            .await?
        {
            return Ok(Some(result));
        }

        let base_sql = build_zos_lob_base_query_from_columns(&parsed, &catalog_columns)
            .ok_or_else(|| Error::Protocol("failed to build z/OS LOB base query".into()))?;
        let base_result = self
            .execute_zos_lob_internal_query("base", &base_sql)
            .await?;

        let output_columns = zos_lob_output_columns(&catalog_columns, &[], base_result.row_count);
        let output_names = output_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let mut output_values = Vec::with_capacity(base_result.rows.len());
        let mut lob_lengths_by_column =
            vec![vec![None; base_result.rows.len()]; catalog_columns.len()];
        let mut chunk_query_count = 0usize;

        for (row_index, base_row) in base_result.rows.iter().enumerate() {
            let base_values = base_row.values().to_vec();
            let mut values = Vec::with_capacity(catalog_columns.len());
            for (column_index, column) in catalog_columns.iter().enumerate() {
                let base_value = base_values
                    .get(column_index)
                    .cloned()
                    .unwrap_or(db2_proto::types::Db2Value::Null);
                if column.is_clob() || column.is_dbclob() {
                    let Some(lob_len) = db2_value_to_usize(&base_value)? else {
                        values.push(db2_proto::types::Db2Value::Null);
                        continue;
                    };
                    lob_lengths_by_column[column_index][row_index] = Some(lob_len);
                    values.push(db2_proto::types::Db2Value::Clob(String::with_capacity(
                        lob_len,
                    )));
                } else {
                    values.push(normalize_zos_materialized_scalar_value(column, base_value));
                }
            }
            output_values.push(values);
        }

        let chunk_specs = zos_lob_combined_chunk_specs(&catalog_columns, &lob_lengths_by_column);
        let mut spec_index = 0usize;
        while spec_index < chunk_specs.len() {
            let mut spec_window = Vec::new();
            let mut window_bytes = 0usize;
            while spec_index < chunk_specs.len() {
                let spec = chunk_specs[spec_index];
                let spec_bytes = zos_lob_chunk_spec_estimated_bytes(&catalog_columns, spec);
                if !spec_window.is_empty()
                    && window_bytes + spec_bytes > zos_lob_chunk_window_target()
                {
                    break;
                }
                spec_window.push(spec);
                window_bytes += spec_bytes;
                spec_index += 1;
            }

            let rows_per_batch = zos_lob_combined_rows_per_batch(&catalog_columns, &spec_window);
            let mut row_start = 0usize;
            while row_start < output_values.len() {
                let row_end = (row_start + rows_per_batch).min(output_values.len());
                if !zos_lob_spec_window_applies_to_rows(
                    &spec_window,
                    &lob_lengths_by_column,
                    row_start,
                    row_end,
                ) {
                    row_start = row_end;
                    continue;
                }

                let chunk_sql = build_zos_lob_combined_chunk_grid_query(
                    &parsed,
                    &catalog_columns,
                    &spec_window,
                    row_start + 1,
                    row_end,
                );
                ensure_sqlstt_sql_len(&chunk_sql)?;
                let stage = format!(
                    "chunk-grid-combined specs={} rows={}..{} estimated_row_bytes={}",
                    spec_window.len(),
                    row_start + 1,
                    row_end,
                    window_bytes
                );
                let chunk_result = self
                    .execute_zos_lob_internal_query(&stage, &chunk_sql)
                    .await?;
                chunk_query_count += 1;
                append_zos_lob_combined_chunk_grid_rows(
                    &chunk_result,
                    &spec_window,
                    &lob_lengths_by_column,
                    &mut output_values,
                    1,
                )?;
                row_start = row_end;
            }
        }

        let output_names: Arc<[String]> = output_names.into();
        let output_rows = output_values
            .into_iter()
            .map(|values| Row::new_shared(output_names.clone(), values))
            .collect::<Vec<_>>();

        let mut diagnostics = base_result.diagnostics;
        if query_diagnostics_enabled() {
            diagnostics.push(format!(
                "zos_lob_chunked source={} strategy=base-plus-combined-chunk-grid rows={} lob_columns={} chunk_queries={} clob_chunk_limit={} dbclob_chunk_limit={} batch_reply_target={} chunk_window_target={}",
                source,
                output_rows.len(),
                catalog_columns
                    .iter()
                    .filter(|column| column.is_lob())
                    .count(),
                chunk_query_count,
                ZOS_CLOB_CHUNK_LIMIT,
                ZOS_DBCLOB_CHUNK_LIMIT,
                zos_lob_batch_reply_target(),
                zos_lob_chunk_window_target()
            ));
        }

        Ok(Some(QueryResult::with_rows_and_diagnostics(
            output_rows,
            output_columns,
            diagnostics,
        )))
    }

    async fn execute_zos_select_lobs_bounded_single_pass(
        &mut self,
        parsed: &SimpleSelectStar,
        catalog_columns: &[CatalogColumn],
        source: &str,
    ) -> Result<Option<QueryResult>, Error> {
        let Some(row_limit) = parse_fetch_first_row_limit(&parsed.suffix) else {
            return Ok(None);
        };
        let initial_specs = zos_lob_initial_chunk_specs(catalog_columns);
        if initial_specs.is_empty()
            || row_limit > zos_lob_combined_rows_per_batch(catalog_columns, &initial_specs)
        {
            return Ok(None);
        }

        let initial_sql =
            build_zos_lob_initial_combined_grid_query(parsed, catalog_columns, &initial_specs);
        ensure_sqlstt_sql_len(&initial_sql)?;
        let initial_result = self
            .execute_zos_lob_internal_query("single-pass-initial-grid", &initial_sql)
            .await?;
        let (mut output_values, lob_lengths_by_column) = materialize_zos_lob_initial_grid_rows(
            &initial_result,
            catalog_columns,
            &initial_specs,
        )?;

        let mut chunk_query_count = 1usize;
        let remaining_specs = zos_lob_combined_chunk_specs(catalog_columns, &lob_lengths_by_column)
            .into_iter()
            .filter(|spec| !initial_specs.contains(spec))
            .collect::<Vec<_>>();
        self.fetch_zos_lob_combined_chunk_specs(
            parsed,
            catalog_columns,
            &remaining_specs,
            &lob_lengths_by_column,
            &mut output_values,
            &mut chunk_query_count,
        )
        .await?;

        let output_columns =
            zos_lob_output_columns(catalog_columns, &[], output_values.len() as i64);
        let output_names = output_columns
            .iter()
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        let output_names: Arc<[String]> = output_names.into();
        let output_rows = output_values
            .into_iter()
            .map(|values| Row::new_shared(output_names.clone(), values))
            .collect::<Vec<_>>();

        let mut diagnostics = initial_result.diagnostics;
        if query_diagnostics_enabled() {
            diagnostics.push(format!(
                "zos_lob_chunked source={} strategy=bounded-single-pass-combined-grid rows={} lob_columns={} chunk_queries={} initial_specs={} clob_chunk_limit={} dbclob_chunk_limit={} batch_reply_target={} chunk_window_target={}",
                source,
                output_rows.len(),
                catalog_columns
                    .iter()
                    .filter(|column| column.is_lob())
                    .count(),
                chunk_query_count,
                initial_specs.len(),
                ZOS_CLOB_CHUNK_LIMIT,
                ZOS_DBCLOB_CHUNK_LIMIT,
                zos_lob_batch_reply_target(),
                zos_lob_chunk_window_target()
            ));
        }

        Ok(Some(QueryResult::with_rows_and_diagnostics(
            output_rows,
            output_columns,
            diagnostics,
        )))
    }

    async fn fetch_zos_lob_combined_chunk_specs(
        &mut self,
        parsed: &SimpleSelectStar,
        catalog_columns: &[CatalogColumn],
        chunk_specs: &[LobChunkSpec],
        lob_lengths_by_column: &[Vec<Option<usize>>],
        output_values: &mut [Vec<db2_proto::types::Db2Value>],
        chunk_query_count: &mut usize,
    ) -> Result<(), Error> {
        let mut spec_index = 0usize;
        while spec_index < chunk_specs.len() {
            let mut spec_window = Vec::new();
            let mut window_bytes = 0usize;
            while spec_index < chunk_specs.len() {
                let spec = chunk_specs[spec_index];
                let spec_bytes = zos_lob_chunk_spec_estimated_bytes(catalog_columns, spec);
                if !spec_window.is_empty()
                    && window_bytes + spec_bytes > zos_lob_chunk_window_target()
                {
                    break;
                }
                spec_window.push(spec);
                window_bytes += spec_bytes;
                spec_index += 1;
            }

            let rows_per_batch = zos_lob_combined_rows_per_batch(catalog_columns, &spec_window);
            let mut row_start = 0usize;
            while row_start < output_values.len() {
                let row_end = (row_start + rows_per_batch).min(output_values.len());
                if !zos_lob_spec_window_applies_to_rows(
                    &spec_window,
                    lob_lengths_by_column,
                    row_start,
                    row_end,
                ) {
                    row_start = row_end;
                    continue;
                }

                let chunk_sql = build_zos_lob_combined_chunk_grid_query(
                    parsed,
                    catalog_columns,
                    &spec_window,
                    row_start + 1,
                    row_end,
                );
                ensure_sqlstt_sql_len(&chunk_sql)?;
                let stage = format!(
                    "chunk-grid-combined specs={} rows={}..{} estimated_row_bytes={}",
                    spec_window.len(),
                    row_start + 1,
                    row_end,
                    window_bytes
                );
                let chunk_result = self
                    .execute_zos_lob_internal_query(&stage, &chunk_sql)
                    .await?;
                *chunk_query_count += 1;
                append_zos_lob_combined_chunk_grid_rows(
                    &chunk_result,
                    &spec_window,
                    lob_lengths_by_column,
                    output_values,
                    1,
                )?;
                row_start = row_end;
            }
        }

        Ok(())
    }

    async fn retry_zos_lob_chunking_after_decode_error(
        &mut self,
        sql: &str,
        result: Result<QueryResult, Error>,
        column_info: &[ColumnInfo],
        result_descriptors: &[db2_proto::fdoca::ColumnDescriptor],
        source: &str,
    ) -> Result<QueryResult, Error> {
        let Err(original_error) = result else {
            return result;
        };

        let retryable_session_error =
            should_retry_query_after_session_error(sql, &[], &original_error);
        if self.zos_lob_internal_depth > 0
            || !self.server_info.as_ref().is_some_and(is_db2_zos_server)
            || !should_retry_zos_lob_chunking_after_decode_error(&original_error)
        {
            return Err(original_error);
        }

        let current_schema = self.config.current_schema.clone();
        // A failed native z/OS LOB cursor can leave the direct-query package
        // section open. Reconnect before SUBSTR/catalog fallback so those
        // internal queries do not collide with the abandoned cursor (-502).
        self.reset_session_state(false).await;
        self.establish_session().await?;
        let prepare_columns =
            catalog_columns_from_prepare_metadata(column_info, result_descriptors);
        if !prepare_columns.is_empty() {
            match self
                .execute_zos_select_lobs_chunked_with_catalog(
                    sql,
                    current_schema.as_deref(),
                    &prepare_columns,
                    source,
                )
                .await
            {
                Ok(Some(mut retry_result)) => {
                    if query_diagnostics_enabled() {
                        retry_result.diagnostics.push(format!(
                            "zos_lob_retry source={} reason={}",
                            source, original_error
                        ));
                    }
                    return Ok(retry_result);
                }
                Ok(None) => {}
                Err(err) => {
                    return Err(Error::Protocol(format!(
                        "z/OS LOB retry failed after direct decoder error: {}; retry_error={}",
                        original_error, err
                    )));
                }
            }
        }

        if let Some(metadata_sql) =
            build_zos_select_star_metadata_query(sql, current_schema.as_deref())
        {
            let metadata = self
                .execute_zos_lob_internal_query("metadata-retry", &metadata_sql)
                .await?;
            match self
                .execute_zos_select_star_lobs_chunked(sql, current_schema.as_deref(), &metadata)
                .await
            {
                Ok(Some(mut retry_result)) => {
                    if query_diagnostics_enabled() {
                        retry_result.diagnostics.push(format!(
                            "zos_lob_retry source=catalog-after-{} reason={}",
                            source, original_error
                        ));
                    }
                    return Ok(retry_result);
                }
                Ok(None) => {
                    if retryable_session_error {
                        return Err(original_error);
                    }
                    let catalog_columns = catalog_columns_from_query_result(&metadata);
                    return Err(Error::Protocol(format!(
                        "{}; z/OS LOB retry skipped: catalog_columns={} lob_columns={} sql={}",
                        original_error,
                        catalog_columns.len(),
                        catalog_columns
                            .iter()
                            .filter(|column| column.is_lob())
                            .count(),
                        summarize_sql_for_diagnostics(sql)
                    )));
                }
                Err(err) => {
                    return Err(Error::Protocol(format!(
                        "z/OS LOB catalog retry failed after direct decoder error: {}; retry_error={}",
                        original_error, err
                    )));
                }
            }
        }

        if retryable_session_error {
            return Err(original_error);
        }

        Err(Error::Protocol(format!(
            "{}; z/OS LOB retry skipped: query is not a simple single-table SELECT; sql={}",
            original_error,
            summarize_sql_for_diagnostics(sql)
        )))
    }

    async fn execute_zos_lob_internal_query(
        &mut self,
        stage: &str,
        sql: &str,
    ) -> Result<QueryResult, Error> {
        ensure_sqlstt_sql_len(sql)?;
        let step_timeout = if self.config.query_timeout.is_zero() {
            Duration::from_secs(30)
        } else {
            self.config.query_timeout
        };

        let mut attempts = 0usize;
        loop {
            self.zos_lob_internal_depth += 1;
            let result = timeout(step_timeout, Box::pin(self.execute_query(sql, &[]))).await;
            self.zos_lob_internal_depth = self.zos_lob_internal_depth.saturating_sub(1);

            match result {
                Ok(Ok(result)) => return Ok(result),
                Ok(Err(err))
                    if attempts < 4 && should_retry_query_after_session_error(sql, &[], &err) =>
                {
                    attempts += 1;
                    self.reset_session_state(false).await;
                    self.establish_session().await?;
                }
                Ok(Err(err)) => {
                    return Err(wrap_zos_lob_stage_error(stage, sql, err));
                }
                Err(_) => {
                    let sql_preview = summarize_sql_for_diagnostics(sql);
                    self.reset_session_state(false).await;
                    return Err(wrap_zos_lob_stage_error(
                        stage,
                        sql,
                        Error::Timeout(format!(
                            "z/OS LOB {stage} timed out after {:?}; connection was closed to avoid protocol desynchronization; sql={sql_preview}",
                            step_timeout
                        )),
                    ));
                }
            }
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
                diagnostics: Vec::new(),
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
            diagnostics: Vec::new(),
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
        sql: &str,
        column_info: &[ColumnInfo],
        initial_descriptors: Option<&[db2_proto::fdoca::ColumnDescriptor]>,
    ) -> Result<QueryResult, Error> {
        self.process_query_reply(frames, sql, column_info, initial_descriptors)
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
        sql: &str,
        column_info: &[ColumnInfo],
        initial_descriptors: Option<&[db2_proto::fdoca::ColumnDescriptor]>,
    ) -> Result<QueryResult, Error> {
        self.process_query_reply_with_fetch_size(
            frames,
            sql,
            column_info,
            initial_descriptors,
            None,
        )
        .await
    }

    async fn process_query_reply_with_fetch_size(
        &mut self,
        frames: &[DssFrame],
        sql: &str,
        column_info: &[ColumnInfo],
        initial_descriptors: Option<&[db2_proto::fdoca::ColumnDescriptor]>,
        fetch_size_override: Option<u32>,
    ) -> Result<QueryResult, Error> {
        let mut rows = Vec::new();
        let mut sqldard_descriptors = initial_descriptors
            .filter(|descriptors| !descriptors.is_empty())
            .map(|descriptors| descriptors.to_vec());
        let mut qrydsc_descriptors: Option<Vec<db2_proto::fdoca::ColumnDescriptor>> = None;
        let mut query_instance_id: Option<Vec<u8>> = None;
        let mut pending_row_bytes = Vec::new();
        let mut extdta_payloads = Vec::new();
        let mut end_of_query = false;
        let mut zos_lob_cleanup_verified = false;
        let collect_diagnostics = query_diagnostics_enabled();
        let mut diagnostics = if collect_diagnostics {
            frame_diagnostics(frames)
        } else {
            Vec::new()
        };
        let prefer_sqldard_descriptors =
            self.zos_lob_internal_depth > 0 || sqldard_descriptors.is_some();
        if collect_diagnostics {
            if let Some(descriptors) = sqldard_descriptors.as_ref() {
                diagnostics.push(format!(
                    "initial_descriptors count={} {}",
                    descriptors.len(),
                    descriptor_summary(descriptors)
                ));
            }
        }
        if collect_diagnostics && prefer_sqldard_descriptors {
            diagnostics.push("descriptor_preference=SQLDARD".into());
        }

        process_query_frames(
            frames,
            column_info,
            &mut rows,
            &mut sqldard_descriptors,
            &mut qrydsc_descriptors,
            prefer_sqldard_descriptors,
            &mut query_instance_id,
            &mut pending_row_bytes,
            &mut extdta_payloads,
            &mut end_of_query,
            collect_diagnostics,
            &mut diagnostics,
        )?;

        // DB2 LUW can stream additional QRYDTA blocks immediately after OPNQRY.
        // Drain those frames before sending CNTQRY, otherwise the server may reject
        // the fetch request as out-of-sequence while the original reply is still active.
        //
        // Db2 for z/OS non-LOB cursors have proven happier and much faster when
        // we move straight to CNTQRY instead of spending a speculative drain
        // timeout here. Keep the drain for LOB cursors, where EXTDTA may arrive
        // as follow-up frames.
        let should_drain_initial_query_frames = self.should_drain_initial_query_frames(
            column_info,
            sqldard_descriptors.as_ref(),
            qrydsc_descriptors.as_ref(),
            prefer_sqldard_descriptors,
        );
        while !end_of_query && should_drain_initial_query_frames {
            let drain_timeout = self.query_frame_drain_timeout(
                column_info,
                sqldard_descriptors.as_ref(),
                qrydsc_descriptors.as_ref(),
                prefer_sqldard_descriptors,
            );
            let more_frames = match timeout(drain_timeout, self.read_reply_frames()).await {
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
            if collect_diagnostics {
                diagnostics.extend(frame_diagnostics(&more_frames));
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
                prefer_sqldard_descriptors,
                &mut query_instance_id,
                &mut pending_row_bytes,
                &mut extdta_payloads,
                &mut end_of_query,
                collect_diagnostics,
                &mut diagnostics,
            )?;
        }

        if !end_of_query && sqldard_descriptors.is_none() && qrydsc_descriptors.is_none() {
            for _ in 0..3 {
                let pkgnamcsn = self.build_pkgnamcsn_for(self.package_id, self.section_number);
                let cntqry_data = db2_proto::commands::cntqry::build_cntqry(
                    &pkgnamcsn,
                    query_instance_id.as_deref(),
                    db2_proto::commands::opnqry::DEFAULT_QRYBLKSZ,
                    None,
                    None,
                );
                let corr_id = self.next_correlation_id();
                let mut writer = DssWriter::new(corr_id);
                writer.write_request(&cntqry_data, false);
                let send_buf = writer.finish();
                self.send_bytes(&send_buf).await?;

                let more_frames = self.read_reply_frames().await?;
                if collect_diagnostics {
                    diagnostics.extend(frame_diagnostics(&more_frames));
                }
                process_query_frames(
                    &more_frames,
                    column_info,
                    &mut rows,
                    &mut sqldard_descriptors,
                    &mut qrydsc_descriptors,
                    prefer_sqldard_descriptors,
                    &mut query_instance_id,
                    &mut pending_row_bytes,
                    &mut extdta_payloads,
                    &mut end_of_query,
                    collect_diagnostics,
                    &mut diagnostics,
                )?;

                if end_of_query
                    || sqldard_descriptors.is_some()
                    || qrydsc_descriptors.is_some()
                    || !rows.is_empty()
                {
                    break;
                }
            }
        }

        if !end_of_query
            && pending_row_bytes.is_empty()
            && extdta_payloads.is_empty()
            && self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && fetch_size_override.is_some_and(|limit| limit > 0 && rows.len() >= limit as usize)
        {
            let cursor_descriptors = preferred_descriptor_vec(
                sqldard_descriptors.as_ref(),
                qrydsc_descriptors.as_ref(),
                prefer_sqldard_descriptors,
            );
            let can_assume_complete = cursor_descriptors.is_none_or(|descriptors| {
                let cursor_column_info = column_info_for_cursor_fetch(column_info, descriptors);
                !descriptors_need_lob_materialization(&cursor_column_info, descriptors)
            });
            if can_assume_complete {
                end_of_query = true;
                if collect_diagnostics {
                    diagnostics.push(format!(
                        "zos_bounded_fetch_complete rows={} limit={}",
                        rows.len(),
                        fetch_size_override.unwrap_or_default()
                    ));
                }
            }
        }

        // If not end of query, continue fetching explicitly.
        if !end_of_query {
            let cursor_descriptors = preferred_descriptor_vec(
                sqldard_descriptors.as_ref(),
                qrydsc_descriptors.as_ref(),
                prefer_sqldard_descriptors,
            )
            .cloned()
            .filter(|descriptors| !descriptors.is_empty());
            if let Some(descriptors) = cursor_descriptors {
                let cursor_column_info = column_info_for_cursor_fetch(column_info, &descriptors);
                if self.server_info.as_ref().is_some_and(is_db2_zos_server)
                    && self.zos_lob_internal_depth == 0
                    && !use_native_zos_lob_strategy()
                    && descriptors_need_zos_lob_materialization(&cursor_column_info, &descriptors)
                {
                    return Err(Error::Protocol(format!(
                        "z/OS LOB result requires transparent materialization after cursor descriptors; {}",
                        descriptor_summary(&descriptors)
                    )));
                }
                if debug_hex_enabled() {
                    eprintln!(
                        "[db2-wire] opening cursor fallback with {} decoded row(s), pending_tail={}",
                        rows.len(),
                        pending_row_bytes.len()
                    );
                }
                let close_after_next_fetch = fetch_size_override.is_some()
                    && rows.is_empty()
                    && pending_row_bytes.is_empty()
                    && self.zos_lob_internal_depth == 0
                    && self.server_info.as_ref().is_some_and(is_db2_zos_server)
                    && !descriptors_need_lob_materialization(&cursor_column_info, &descriptors)
                    && use_zos_non_lob_close_with_limited_fetch();
                let mut cursor = Cursor::new(
                    cursor_column_info,
                    descriptors,
                    query_instance_id,
                    self.build_pkgnamcsn_for(self.package_id, self.section_number),
                    fetch_size_override.unwrap_or(self.config.fetch_size),
                    close_after_next_fetch,
                );
                cursor.pending_row_bytes = std::mem::take(&mut pending_row_bytes);

                let mut stalled_fetches = 0usize;
                loop {
                    let pending_before = cursor.pending_row_bytes.len();
                    let fetch_started = collect_diagnostics.then(Instant::now);
                    let (more_rows, done, more_extdta_payloads) =
                        cursor.fetch_next_from(self).await?;
                    let pending_after = cursor.pending_row_bytes.len();
                    if let Some(started) = fetch_started {
                        diagnostics.push(format!(
                            "cursor_fetch_ms={:.3} rows={} done={} extdta={} pending_before={} pending_after={} last_fetch=[{}]",
                            started.elapsed().as_secs_f64() * 1000.0,
                            more_rows.len(),
                            done,
                            more_extdta_payloads.len(),
                            pending_before,
                            pending_after,
                            cursor.last_fetch_diagnostics.join("; ")
                        ));
                    }
                    let made_progress = !more_rows.is_empty()
                        || !more_extdta_payloads.is_empty()
                        || done
                        || pending_after != pending_before;
                    if made_progress {
                        stalled_fetches = 0;
                    } else {
                        stalled_fetches += 1;
                    }
                    rows.extend(more_rows);
                    if !more_extdta_payloads.is_empty() {
                        apply_extdta_payloads_to_rows(
                            &mut rows,
                            &cursor.descriptors,
                            &more_extdta_payloads,
                        );
                        extdta_payloads.extend(more_extdta_payloads);
                        if !rows_need_extdta_payloads(&rows, &cursor.descriptors) {
                            let close_after_materialize = use_zos_lob_close_after_materialization();
                            if done {
                                zos_lob_cleanup_verified = true;
                            } else if close_after_materialize
                                && !use_zos_lob_passive_tail_before_close()
                            {
                                if collect_diagnostics {
                                    diagnostics.push(format!(
                                        "cursor_lob_materialized_tail skipped=active_close rows={} extdta={}",
                                        rows.len(),
                                        extdta_payloads.len()
                                    ));
                                }
                            } else {
                                let tail_outcome = cursor.passive_tail_drain_from(self).await?;
                                if tail_outcome.ran() && collect_diagnostics {
                                    diagnostics.push(format!(
                                        "cursor_lob_materialized_tail rows={} extdta={} verified={} reuse_allowed={} trusted_quiet={} quiet_reject_reason={} tail_frames={} tail_reads={} discarded_rows={} discarded_extdta={} discarded_extdta_bytes={} end_of_query={} timed_out={} max_reads_reached={} protocol_error={} pending_tail={} elapsed_ms={:.3} last_fetch=[{}]",
                                        rows.len(),
                                        extdta_payloads.len(),
                                        tail_outcome.verified(),
                                        tail_outcome.reuse_allowed(),
                                        tail_outcome.trusted_quiet,
                                        tail_outcome.quiet_reject_reason,
                                        tail_outcome.frames,
                                        tail_outcome.reads,
                                        tail_outcome.discarded_rows,
                                        tail_outcome.discarded_extdta,
                                        tail_outcome.discarded_extdta_bytes,
                                        tail_outcome.end_of_query,
                                        tail_outcome.timed_out,
                                        tail_outcome.max_reads_reached,
                                        tail_outcome.protocol_error,
                                        tail_outcome.pending_tail,
                                        tail_outcome.elapsed_ms,
                                        cursor.last_fetch_diagnostics.join("; ")
                                    ));
                                }
                                zos_lob_cleanup_verified = tail_outcome.reuse_allowed();
                            }
                            if !zos_lob_cleanup_verified && close_after_materialize {
                                let close_outcome = cursor.close_from(self).await?;
                                zos_lob_cleanup_verified = close_outcome.verified();
                                if collect_diagnostics {
                                    diagnostics.push(format!(
                                        "cursor_lob_materialized_close rows={} extdta={} verified={} close_frames={} close_reads={} discarded_extdta={} timed_out={} last_fetch=[{}]",
                                        rows.len(),
                                        extdta_payloads.len(),
                                        close_outcome.verified(),
                                        close_outcome.frames,
                                        close_outcome.reads,
                                        close_outcome.discarded_extdta,
                                        close_outcome.timed_out,
                                        cursor.last_fetch_diagnostics.join("; ")
                                    ));
                                }
                            }
                            break;
                        }
                    }
                    if done {
                        zos_lob_cleanup_verified = true;
                        break;
                    }
                    if stalled_fetches >= 3 {
                        return Err(Error::Protocol(format!(
                            "query fetch stalled while decoding row data; pending_tail={} progress={} last_fetch=[{}]",
                            pending_after,
                            db2_proto::fdoca::describe_decode_progress(
                                &cursor.pending_row_bytes,
                                &cursor.descriptors
                            ),
                            cursor.last_fetch_diagnostics.join("; ")
                        )));
                    }
                }
                pending_row_bytes = std::mem::take(&mut cursor.pending_row_bytes);
            }
        }

        let active_descriptors = preferred_descriptor_vec(
            sqldard_descriptors.as_ref(),
            qrydsc_descriptors.as_ref(),
            prefer_sqldard_descriptors,
        );
        if let Some(descriptors) = active_descriptors {
            apply_extdta_payloads_to_rows(&mut rows, descriptors, &extdta_payloads);
        }
        let columns = if !column_info.is_empty() {
            if let Some(descriptors) = active_descriptors.filter(|d| d.len() == column_info.len()) {
                column_info_with_descriptor_types(column_info, descriptors)
            } else if let Some(descriptors) = active_descriptors {
                column_info_from_descriptors(descriptors)
            } else {
                column_info.to_vec()
            }
        } else if let Some(descriptors) = active_descriptors {
            column_info_from_descriptors(descriptors)
        } else {
            Vec::new()
        };

        if collect_diagnostics {
            diagnostics.push(format!(
                "decode_final rows={} columns={} pending_tail={} qrydsc_desc={} sqldard_desc={} active_desc={}",
                rows.len(),
                columns.len(),
                pending_row_bytes.len(),
                qrydsc_descriptors.as_ref().map(|v| v.len()).unwrap_or(0),
                sqldard_descriptors.as_ref().map(|v| v.len()).unwrap_or(0),
                active_descriptors.map(|v| v.len()).unwrap_or(0)
            ));
            if let Some(descriptors) = active_descriptors {
                diagnostics.push(format!(
                    "decode_final descriptors {}",
                    descriptor_summary(descriptors)
                ));
                if !pending_row_bytes.is_empty() {
                    diagnostics.push(format!(
                        "decode_final pending_tail_preview={}",
                        format_hex_preview(&pending_row_bytes, 160)
                    ));
                    diagnostics.push(format!(
                        "decode_final progress={}",
                        db2_proto::fdoca::describe_decode_progress(&pending_row_bytes, descriptors)
                    ));
                }
            } else if !pending_row_bytes.is_empty() {
                diagnostics.push(format!(
                    "decode_final pending_without_descriptors len={} preview={}",
                    pending_row_bytes.len(),
                    format_hex_preview(&pending_row_bytes, 160)
                ));
            }
        }

        if debug_hex_enabled() {
            eprintln!(
                "[db2-wire] process_query_reply final columns={} rows={} initial_columns={} qrydsc_desc={} sqldard_desc={} pending={}",
                columns.len(),
                rows.len(),
                column_info.len(),
                qrydsc_descriptors.as_ref().map(|v| v.len()).unwrap_or(0),
                sqldard_descriptors.as_ref().map(|v| v.len()).unwrap_or(0),
                pending_row_bytes.len()
            );
        }

        if end_of_query
            && !pending_row_bytes.is_empty()
            && !rows.is_empty()
            && db2_proto::fdoca::is_ignorable_final_row_tail(&pending_row_bytes)
        {
            if collect_diagnostics {
                diagnostics.push(format!(
                    "decode_final_discarded_trailer len={} preview={}",
                    pending_row_bytes.len(),
                    format_hex_preview(&pending_row_bytes, 160)
                ));
            }
            pending_row_bytes.clear();
        }

        if end_of_query && !pending_row_bytes.is_empty() {
            let progress = active_descriptors
                .map(|descriptors| {
                    db2_proto::fdoca::describe_decode_progress(&pending_row_bytes, descriptors)
                })
                .unwrap_or_else(|| "no active descriptors".to_string());
            return Err(Error::Protocol(format!(
                "query ended with undecoded row data; pending_tail={} progress={}",
                pending_row_bytes.len(),
                progress
            )));
        }

        let columns = column_info_with_select_aliases(sql, columns);
        let rows = rows_with_result_column_names(rows, &columns);
        let mut result = QueryResult::with_rows_and_diagnostics(rows, columns, diagnostics);
        if end_of_query {
            zos_lob_cleanup_verified = true;
        }
        if self.zos_lob_internal_depth == 0
            && self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && result_has_zos_lob_materialization(&result)
        {
            let mut lob_cleanup_committed = false;
            if self.auto_commit && use_zos_lob_commit_after_materialization() {
                let commit_started = collect_diagnostics.then(Instant::now);
                match self.commit().await {
                    Ok(()) => {
                        lob_cleanup_committed = true;
                        if let Some(started) = commit_started {
                            result.diagnostics.push(format!(
                                "zos_lob_commit_after_materialization_ms={:.3} success=true rows={} columns={}",
                                started.elapsed().as_secs_f64() * 1000.0,
                                result.row_count,
                                result.columns.len()
                            ));
                        }
                    }
                    Err(err) => {
                        if let Some(started) = commit_started {
                            result.diagnostics.push(format!(
                                "zos_lob_commit_after_materialization_ms={:.3} success=false error={}",
                                started.elapsed().as_secs_f64() * 1000.0,
                                sanitize_diagnostic_value(&err.to_string())
                            ));
                        }
                    }
                }
            }

            if collect_diagnostics {
                result.diagnostics.push(format!(
                    "zos_lob_cleanup_verified={} close_after_materialize={}",
                    zos_lob_cleanup_verified,
                    use_zos_lob_close_after_materialization()
                ));
            }

            if !lob_cleanup_committed
                && !zos_lob_cleanup_verified
                && use_zos_lob_disconnect_after_materialization()
            {
                if collect_diagnostics {
                    result.diagnostics.push(format!(
                        "zos_lob_disconnect_after_materialization=true reason=cleanup_unverified rows={} columns={}",
                        result.row_count,
                        result.columns.len()
                    ));
                }
                self.reset_session_state(false).await;
            }
        }

        Ok(result)
    }

    fn query_frame_drain_timeout(
        &self,
        column_info: &[ColumnInfo],
        sqldard_descriptors: Option<&Vec<db2_proto::fdoca::ColumnDescriptor>>,
        qrydsc_descriptors: Option<&Vec<db2_proto::fdoca::ColumnDescriptor>>,
        prefer_sqldard_descriptors: bool,
    ) -> Duration {
        let timeout = self.frame_drain_timeout();
        let has_lobs = preferred_descriptor_vec(
            sqldard_descriptors,
            qrydsc_descriptors,
            prefer_sqldard_descriptors,
        )
        .is_some_and(|descriptors| descriptors_need_lob_materialization(column_info, descriptors));

        if has_lobs {
            if use_native_zos_lob_strategy() {
                return timeout.max(native_zos_lob_frame_drain_timeout());
            }
            timeout.max(zos_lob_frame_drain_timeout())
        } else {
            timeout
        }
    }

    fn should_drain_initial_query_frames(
        &self,
        column_info: &[ColumnInfo],
        sqldard_descriptors: Option<&Vec<db2_proto::fdoca::ColumnDescriptor>>,
        qrydsc_descriptors: Option<&Vec<db2_proto::fdoca::ColumnDescriptor>>,
        prefer_sqldard_descriptors: bool,
    ) -> bool {
        let has_lobs = preferred_descriptor_vec(
            sqldard_descriptors,
            qrydsc_descriptors,
            prefer_sqldard_descriptors,
        )
        .is_some_and(|descriptors| descriptors_need_lob_materialization(column_info, descriptors));

        if self.server_info.as_ref().is_some_and(is_db2_zos_server)
            && has_lobs
            && use_native_zos_lob_strategy()
            && skip_zos_native_lob_initial_drain()
        {
            return false;
        }

        if self.server_info.as_ref().is_some_and(is_db2_zos_server) && !has_lobs {
            return false;
        }

        true
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
            diagnostics: if query_diagnostics_enabled() {
                frame_diagnostics(frames)
            } else {
                Vec::new()
            },
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
                zos_lob_internal_depth: 0,
                connection_diagnostics: Vec::new(),
                zos_select_cache: HashMap::new(),
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

    pub async fn take_connection_diagnostics(&self) -> Vec<String> {
        let mut guard = self.inner.lock().await;
        std::mem::take(&mut guard.connection_diagnostics)
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
        let diagnostics_started = query_diagnostics_enabled().then(Instant::now);
        let result = if query_timeout.is_zero() {
            match guard.execute_query_with_reconnect_retry(sql, params).await {
                Ok(result) => Ok(result),
                Err(err) => Err(guard.finalize_operation_error("query", err).await),
            }
        } else {
            match timeout(
                query_timeout,
                guard.execute_query_with_reconnect_retry(sql, params),
            )
            .await
            {
                Ok(result) => match result {
                    Ok(result) => Ok(result),
                    Err(err) => Err(guard.finalize_operation_error("query", err).await),
                },
                Err(_) => Err(guard.disconnect_after_timeout("query", query_timeout).await),
            }
        };
        finish_query_diagnostics(sql, params.len(), diagnostics_started, result)
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
            let use_zos_sqlstt = guard.server_info.as_ref().is_some_and(is_db2_zos_server);
            let sqlstt_data = build_sqlstt_for_server(sql, use_zos_sqlstt);
            let use_zos_cursor_attributes =
                sql_is_query(sql) && use_zos_sqlstt && use_zos_read_only_cursor_attributes();

            let mut writer = DssWriter::new(corr_id);
            writer.write_request_next_same_corr(&prpsqlstt_data, true);
            if use_zos_cursor_attributes {
                let sqlattr_data =
                    db2_proto::commands::sqlattr::build_sqlattr_for_read_only_cursor();
                writer.write_object_same_corr(&sqlattr_data, true);
            }
            writer.write_object(&sqlstt_data, false);

            let send_buf = writer.finish();
            if let Err(err) = guard.send_bytes(&send_buf).await {
                guard.release_prepared_section(section_number);
                return Err(err);
            }

            let frames = match guard.read_prepare_reply_frames().await {
                Ok(frames) => frames,
                Err(err) => {
                    guard.release_prepared_section(section_number);
                    return Err(err);
                }
            };
            let column_metadata = match guard.parse_prepare_reply(&frames) {
                Ok(column_metadata) => column_info_with_select_aliases(sql, column_metadata),
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
        // Release any pool checkout before attempting transport shutdown so
        // checked-out pooled clients do not leak permits if close fails.
        let _checkout = self.detach_pool_checkout().await;

        let mut guard = self.inner.lock().await;
        let mut close_error = None;
        if guard.connected {
            if let Some(transport) = guard.transport.as_mut() {
                if let Err(err) = transport.close().await {
                    close_error = Some(err);
                }
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
        guard.zos_select_cache.clear();
        drop(guard);

        if let Some(err) = close_error {
            return Err(err);
        }

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
#[cfg(test)]
const ZOS_CLOB_INLINE_LIMIT: usize = 32704;
#[cfg(test)]
const ZOS_DBCLOB_INLINE_LIMIT: usize = ZOS_CLOB_INLINE_LIMIT / 2;
const ZOS_CLOB_CHUNK_LIMIT: usize = 16_000;
const ZOS_DBCLOB_CHUNK_LIMIT: usize = ZOS_CLOB_CHUNK_LIMIT / 2;
const ZOS_LOB_BATCH_REPLY_TARGET: usize = 4_000_000;
const ZOS_LOB_CHUNK_WINDOW_TARGET: usize = 160_000;
const ZOS_LOB_FRAME_DRAIN_TIMEOUT_MS: usize = 250;
const ZOS_NATIVE_LOB_FRAME_DRAIN_TIMEOUT_MS: usize = 250;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SimpleSelectStar {
    table_ref: String,
    suffix: String,
    schema: String,
    table: String,
    selected_columns: Option<Vec<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogColumn {
    name: String,
    coltype: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LobChunkSpec {
    column_index: usize,
    chunk_number: usize,
    start: usize,
    len: usize,
}

impl CatalogColumn {
    fn normalized_coltype(&self) -> String {
        self.coltype.trim().to_ascii_uppercase()
    }

    fn is_clob(&self) -> bool {
        self.normalized_coltype().starts_with("CLOB")
    }

    fn is_dbclob(&self) -> bool {
        self.normalized_coltype().starts_with("DBCLOB")
    }

    fn is_lob(&self) -> bool {
        self.is_clob() || self.is_dbclob()
    }

    fn is_rowid(&self) -> bool {
        self.normalized_coltype() == "ROWID"
    }
}

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

pub(crate) fn is_db2_zos_server(server_info: &ServerInfo) -> bool {
    let release = server_info.server_release.trim_start().to_ascii_uppercase();
    let class = server_info.server_class.trim_start().to_ascii_uppercase();
    release.starts_with("DSN")
        || class.starts_with("DSN")
        || class.contains("Z/OS")
        || class.contains("MVS")
}

pub(crate) fn build_sqlstt_for_server(sql: &str, use_zos_format: bool) -> Vec<u8> {
    if use_zos_format {
        db2_proto::commands::sqlstt::build_sqlstt_zos(sql)
    } else {
        db2_proto::commands::sqlstt::build_sqlstt(sql)
    }
}

fn column_info_with_select_aliases(sql: &str, mut columns: Vec<ColumnInfo>) -> Vec<ColumnInfo> {
    if columns.is_empty() {
        return columns;
    }

    let Some(aliases) = select_projection_aliases(sql) else {
        return columns;
    };
    if aliases.len() != columns.len() {
        return columns;
    }

    for (column, alias) in columns.iter_mut().zip(aliases) {
        if let Some(alias) = alias {
            column.name = alias;
        }
    }

    columns
}

fn select_projection_aliases(sql: &str) -> Option<Vec<Option<String>>> {
    let projection = top_level_select_projection(sql)?;
    let items = split_top_level_select_items(projection);
    if items.is_empty() {
        return None;
    }

    Some(
        items
            .into_iter()
            .map(select_projection_output_name)
            .collect(),
    )
}

fn top_level_select_projection(sql: &str) -> Option<&str> {
    let select_idx = find_top_level_keyword(sql, "SELECT", 0)?;
    let select_end = select_idx + "SELECT".len();
    let from_idx = find_top_level_keyword(sql, "FROM", select_end)?;
    Some(&sql[select_end..from_idx])
}

fn split_top_level_select_items(projection: &str) -> Vec<&str> {
    let bytes = projection.as_bytes();
    let mut items = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut depth = 0usize;
    let mut state = SqlScanState::Normal;

    while i < bytes.len() {
        match state {
            SqlScanState::Normal => {
                if starts_with_at(bytes, i, b"--") {
                    state = SqlScanState::LineComment;
                    i += 2;
                    continue;
                }
                if starts_with_at(bytes, i, b"/*") {
                    state = SqlScanState::BlockComment;
                    i += 2;
                    continue;
                }
                match bytes[i] {
                    b'\'' => state = SqlScanState::SingleQuote,
                    b'"' => state = SqlScanState::DoubleQuote,
                    b'(' => depth += 1,
                    b')' => depth = depth.saturating_sub(1),
                    b',' if depth == 0 => {
                        let item = projection[start..i].trim();
                        if !item.is_empty() {
                            items.push(item);
                        }
                        start = i + 1;
                    }
                    _ => {}
                }
                i += 1;
            }
            SqlScanState::SingleQuote => {
                if bytes[i] == b'\'' {
                    if starts_with_at(bytes, i + 1, b"'") {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::DoubleQuote => {
                if bytes[i] == b'"' {
                    if starts_with_at(bytes, i + 1, b"\"") {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::LineComment => {
                if bytes[i] == b'\n' {
                    state = SqlScanState::Normal;
                }
                i += 1;
            }
            SqlScanState::BlockComment => {
                if starts_with_at(bytes, i, b"*/") {
                    state = SqlScanState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    let item = projection[start..].trim();
    if !item.is_empty() {
        items.push(item);
    }

    items
}

fn explicit_select_alias(item: &str) -> Option<String> {
    let as_idx = find_last_top_level_keyword(item, "AS")?;
    parse_alias_identifier(&item[as_idx + "AS".len()..])
}

fn select_projection_output_name(item: &str) -> Option<String> {
    explicit_select_alias(item).or_else(|| simple_projection_column_name(item))
}

fn simple_projection_column_name(item: &str) -> Option<String> {
    let trimmed = item.trim();
    let end = consume_identifier_ref(trimmed, 0)?;
    if skip_ascii_whitespace(trimmed, end) != trimmed.len() {
        return None;
    }

    let pieces = split_table_ref_parts(trimmed)?;
    let name = pieces.last()?.trim();
    if name.is_empty() {
        return None;
    }

    if trimmed.contains('"') {
        Some(name.to_string())
    } else {
        Some(name.to_ascii_uppercase())
    }
}

fn parse_alias_identifier(text: &str) -> Option<String> {
    let trimmed = text.trim_start();
    if trimmed.starts_with('"') {
        return parse_quoted_identifier(trimmed);
    }

    let token_len = trimmed
        .bytes()
        .take_while(|byte| is_sql_identifier_byte(*byte))
        .count();
    if token_len == 0 {
        return None;
    }

    Some(trimmed[..token_len].to_ascii_uppercase())
}

fn parse_quoted_identifier(text: &str) -> Option<String> {
    let mut out = String::new();
    let mut chars = text[1..].chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if chars.peek().is_some_and(|next| *next == '"') {
                chars.next();
                out.push('"');
            } else {
                return Some(out);
            }
        } else {
            out.push(ch);
        }
    }
    None
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SqlScanState {
    Normal,
    SingleQuote,
    DoubleQuote,
    LineComment,
    BlockComment,
}

fn find_top_level_keyword(sql: &str, keyword: &str, start: usize) -> Option<usize> {
    find_top_level_keyword_impl(sql, keyword, start, false)
}

fn find_last_top_level_keyword(sql: &str, keyword: &str) -> Option<usize> {
    find_top_level_keyword_impl(sql, keyword, 0, true)
}

fn sql_prefers_zos_non_lob_excsqlstt_output(sql: &str) -> bool {
    use_zos_non_lob_excsqlstt_output()
        || (use_zos_like_predicate_excsqlstt_output() && sql_has_like_predicate(sql))
}

fn sql_uses_zos_like_predicate_large_package(sql: &str) -> bool {
    use_zos_like_predicate_excsqlstt_output() && sql_has_like_predicate(sql)
}

fn sql_has_like_predicate(sql: &str) -> bool {
    find_keyword_outside_literals(sql, "LIKE").is_some()
}

fn find_keyword_outside_literals(sql: &str, keyword: &str) -> Option<usize> {
    let bytes = sql.as_bytes();
    let keyword = keyword.as_bytes();
    let mut i = 0usize;
    let mut state = SqlScanState::Normal;

    while i < bytes.len() {
        match state {
            SqlScanState::Normal => {
                if starts_with_at(bytes, i, b"--") {
                    state = SqlScanState::LineComment;
                    i += 2;
                    continue;
                }
                if starts_with_at(bytes, i, b"/*") {
                    state = SqlScanState::BlockComment;
                    i += 2;
                    continue;
                }
                match bytes[i] {
                    b'\'' => {
                        state = SqlScanState::SingleQuote;
                        i += 1;
                    }
                    b'"' => {
                        state = SqlScanState::DoubleQuote;
                        i += 1;
                    }
                    _ => {
                        if keyword_at(bytes, keyword, i) {
                            return Some(i);
                        }
                        i += 1;
                    }
                }
            }
            SqlScanState::SingleQuote => {
                if bytes[i] == b'\'' {
                    if starts_with_at(bytes, i + 1, b"'") {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::DoubleQuote => {
                if bytes[i] == b'"' {
                    if starts_with_at(bytes, i + 1, b"\"") {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::LineComment => {
                if bytes[i] == b'\n' {
                    state = SqlScanState::Normal;
                }
                i += 1;
            }
            SqlScanState::BlockComment => {
                if starts_with_at(bytes, i, b"*/") {
                    state = SqlScanState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    None
}

fn find_top_level_keyword_impl(
    sql: &str,
    keyword: &str,
    start: usize,
    last: bool,
) -> Option<usize> {
    let bytes = sql.as_bytes();
    let keyword = keyword.as_bytes();
    let mut i = start.min(bytes.len());
    let mut depth = 0usize;
    let mut state = SqlScanState::Normal;
    let mut found = None;

    while i < bytes.len() {
        match state {
            SqlScanState::Normal => {
                if starts_with_at(bytes, i, b"--") {
                    state = SqlScanState::LineComment;
                    i += 2;
                    continue;
                }
                if starts_with_at(bytes, i, b"/*") {
                    state = SqlScanState::BlockComment;
                    i += 2;
                    continue;
                }
                match bytes[i] {
                    b'\'' => {
                        state = SqlScanState::SingleQuote;
                        i += 1;
                    }
                    b'"' => {
                        state = SqlScanState::DoubleQuote;
                        i += 1;
                    }
                    b'(' => {
                        depth += 1;
                        i += 1;
                    }
                    b')' => {
                        depth = depth.saturating_sub(1);
                        i += 1;
                    }
                    _ => {
                        if depth == 0 && keyword_at(bytes, keyword, i) {
                            found = Some(i);
                            if !last {
                                return found;
                            }
                            i += keyword.len();
                        } else {
                            i += 1;
                        }
                    }
                }
            }
            SqlScanState::SingleQuote => {
                if bytes[i] == b'\'' {
                    if starts_with_at(bytes, i + 1, b"'") {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::DoubleQuote => {
                if bytes[i] == b'"' {
                    if starts_with_at(bytes, i + 1, b"\"") {
                        i += 2;
                    } else {
                        state = SqlScanState::Normal;
                        i += 1;
                    }
                } else {
                    i += 1;
                }
            }
            SqlScanState::LineComment => {
                if bytes[i] == b'\n' {
                    state = SqlScanState::Normal;
                }
                i += 1;
            }
            SqlScanState::BlockComment => {
                if starts_with_at(bytes, i, b"*/") {
                    state = SqlScanState::Normal;
                    i += 2;
                } else {
                    i += 1;
                }
            }
        }
    }

    found
}

fn keyword_at(bytes: &[u8], keyword: &[u8], index: usize) -> bool {
    if index + keyword.len() > bytes.len() {
        return false;
    }
    let prev_ok = index == 0 || !is_sql_identifier_byte(bytes[index - 1]);
    let next_index = index + keyword.len();
    let next_ok = next_index >= bytes.len() || !is_sql_identifier_byte(bytes[next_index]);
    prev_ok
        && next_ok
        && bytes[index..next_index]
            .iter()
            .zip(keyword.iter())
            .all(|(left, right)| left.eq_ignore_ascii_case(right))
}

fn starts_with_at(bytes: &[u8], index: usize, needle: &[u8]) -> bool {
    bytes
        .get(index..index.saturating_add(needle.len()))
        .is_some_and(|slice| slice == needle)
}

fn is_sql_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$' | b'#' | b'@')
}

fn optimize_zos_select_sql(sql: &str) -> Option<String> {
    if !use_zos_select_sql_optimization() || !sql_is_plain_select(sql) {
        return None;
    }

    let trimmed = sql.trim();
    let semicolon = trimmed.ends_with(';');
    let body = trimmed.trim_end_matches(';').trim_end();
    let upper = body.to_ascii_uppercase();

    if upper.contains(" FOR UPDATE")
        || upper.contains(" FOR READ ONLY")
        || upper.contains(" FOR FETCH ONLY")
        || upper.contains(" OPTIMIZE FOR")
    {
        return None;
    }

    let isolation_start = trailing_isolation_clause_start(body);
    let (select_part, trailing_part) = isolation_start
        .map(|idx| (body[..idx].trim_end(), &body[idx..]))
        .unwrap_or((body, ""));

    let mut optimized = String::with_capacity(body.len() + 48);
    optimized.push_str(select_part);
    optimized.push_str(" FOR FETCH ONLY");
    if let Some(limit) = parse_fetch_first_row_limit(select_part) {
        optimized.push_str(" OPTIMIZE FOR ");
        optimized.push_str(&limit.to_string());
        optimized.push_str(" ROWS");
    }
    if !trailing_part.is_empty() {
        optimized.push(' ');
        optimized.push_str(trailing_part.trim_start());
    }
    if semicolon {
        optimized.push(';');
    }

    (optimized != trimmed).then_some(optimized)
}

fn sql_is_plain_select(sql: &str) -> bool {
    let trimmed = sql.trim_start();
    trimmed
        .get(..6)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("SELECT"))
        && trimmed
            .as_bytes()
            .get(6)
            .is_none_or(|byte| !byte.is_ascii_alphanumeric() && *byte != b'_')
}

fn trailing_isolation_clause_start(sql: &str) -> Option<usize> {
    let trimmed_end = sql.trim_end();
    let upper = trimmed_end.to_ascii_uppercase();
    for suffix in [" WITH UR", " WITH CS", " WITH RS", " WITH RR"] {
        if upper.ends_with(suffix) {
            return Some(trimmed_end.len().saturating_sub(suffix.len()) + 1);
        }
    }
    None
}

fn build_zos_select_star_metadata_query(sql: &str, current_schema: Option<&str>) -> Option<String> {
    let parsed = parse_simple_select_for_zos_lobs(sql, current_schema)?;
    if parsed.schema.eq_ignore_ascii_case("SYSIBM")
        && parsed.table.eq_ignore_ascii_case("SYSCOLUMNS")
    {
        return None;
    }
    Some(format!(
        "SELECT NAME, COLTYPE FROM SYSIBM.SYSCOLUMNS WHERE TBCREATOR = '{}' AND TBNAME = '{}' ORDER BY COLNO",
        escape_sql_string_literal(&parsed.schema.to_ascii_uppercase()),
        escape_sql_string_literal(&parsed.table.to_ascii_uppercase())
    ))
}

#[cfg(test)]
fn build_zos_select_star_lob_base_query(
    sql: &str,
    current_schema: Option<&str>,
    metadata: &QueryResult,
) -> Option<String> {
    let parsed = parse_simple_select_for_zos_lobs(sql, current_schema)?;
    let columns = selected_catalog_columns(&parsed, &catalog_columns_from_query_result(metadata))?;
    build_zos_lob_base_query_from_columns(&parsed, &columns)
}

fn build_zos_lob_base_query_from_columns(
    parsed: &SimpleSelectStar,
    columns: &[CatalogColumn],
) -> Option<String> {
    if columns.is_empty() || !columns.iter().any(CatalogColumn::is_lob) {
        return None;
    }

    let projection = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let ident = quote_sql_identifier(&column.name);
            if column.is_lob() {
                format!(
                    "CAST(LENGTH({ident}) AS VARCHAR(32)) AS {}",
                    quote_sql_identifier(&lob_len_alias(index))
                )
            } else if column.is_rowid() {
                format!("HEX({ident}) AS {ident}")
            } else {
                format!(
                    "CAST({ident} AS VARCHAR({})) AS {ident}",
                    zos_base_scalar_cast_len(column)
                )
            }
        })
        .collect::<Vec<_>>();

    let suffix = parsed.suffix.trim();
    if suffix.is_empty() {
        Some(format!(
            "SELECT {} FROM {}",
            projection.join(", "),
            parsed.table_ref
        ))
    } else {
        Some(format!(
            "SELECT {} FROM {} {}",
            projection.join(", "),
            parsed.table_ref,
            suffix
        ))
    }
}

#[cfg(test)]
fn build_zos_lob_row_probe_query(parsed: &SimpleSelectStar) -> String {
    let suffix = parsed.suffix.trim();
    if suffix.is_empty() {
        format!(
            "SELECT 1 AS \"DB2NODE_ROW_EXISTS\" FROM {}",
            parsed.table_ref
        )
    } else {
        format!(
            "SELECT 1 AS \"DB2NODE_ROW_EXISTS\" FROM {} {}",
            parsed.table_ref, suffix
        )
    }
}

#[cfg(test)]
fn build_zos_lob_length_query(
    parsed: &SimpleSelectStar,
    column: &CatalogColumn,
    row_number: usize,
) -> String {
    let ident = quote_sql_identifier(&column.name);
    let source_sql = build_zos_numbered_single_column_source_sql(parsed, &ident);
    format!(
        "SELECT CAST(LENGTH({ident}) AS VARCHAR(32)) AS {} FROM ({source_sql}) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" = {}",
        quote_sql_identifier("DB2NODE_LOB_LEN"),
        row_number.max(1)
    )
}

#[cfg(test)]
fn build_zos_scalar_value_query(
    parsed: &SimpleSelectStar,
    column: &CatalogColumn,
    row_number: usize,
) -> String {
    let ident = quote_sql_identifier(&column.name);
    let projection = if column.is_rowid() {
        format!("HEX({ident}) AS {ident}")
    } else {
        format!(
            "CAST({ident} AS VARCHAR({})) AS {ident}",
            zos_base_scalar_cast_len(column)
        )
    };

    let source_sql = build_zos_numbered_single_column_source_sql(parsed, &ident);
    format!(
        "SELECT {projection} FROM ({source_sql}) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" = {}",
        row_number.max(1)
    )
}

#[cfg(test)]
fn build_zos_lob_chunk_grid_query(
    parsed: &SimpleSelectStar,
    column: &CatalogColumn,
    chunks: &[(usize, usize)],
    row_start: usize,
    row_end: usize,
) -> String {
    let ident = quote_sql_identifier(&column.name);
    let projection = chunks
        .iter()
        .enumerate()
        .map(|(index, (start, len))| {
            build_zos_lob_chunk_projection(&ident, column, *start, *len, &lob_chunk_alias(index))
        })
        .collect::<Vec<_>>();
    let source_sql = build_zos_numbered_single_column_source_sql(parsed, &ident);
    format!(
        "SELECT \"DB2NODE_RN\", {} FROM ({source_sql}) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" BETWEEN {} AND {}",
        projection.join(", "),
        row_start.max(1),
        row_end.max(row_start.max(1))
    )
}

fn build_zos_lob_combined_chunk_grid_query(
    parsed: &SimpleSelectStar,
    columns: &[CatalogColumn],
    specs: &[LobChunkSpec],
    row_start: usize,
    row_end: usize,
) -> String {
    let projection = specs
        .iter()
        .map(|spec| {
            let column = &columns[spec.column_index];
            let ident = quote_sql_identifier(&column.name);
            build_zos_lob_chunk_projection(
                &ident,
                column,
                spec.start,
                spec.len,
                &lob_chunk_column_alias(spec),
            )
        })
        .collect::<Vec<_>>();
    let mut idents = Vec::new();
    for spec in specs {
        let ident = quote_sql_identifier(&columns[spec.column_index].name);
        if !idents.iter().any(|existing| existing == &ident) {
            idents.push(ident);
        }
    }
    let source_sql = build_zos_numbered_multi_column_source_sql(parsed, &idents);
    format!(
        "SELECT \"DB2NODE_RN\", {} FROM ({source_sql}) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" BETWEEN {} AND {}",
        projection.join(", "),
        row_start.max(1),
        row_end.max(row_start.max(1))
    )
}

fn build_zos_lob_initial_combined_grid_query(
    parsed: &SimpleSelectStar,
    columns: &[CatalogColumn],
    specs: &[LobChunkSpec],
) -> String {
    let base_projection = columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let ident = quote_sql_identifier(&column.name);
            if column.is_lob() {
                format!(
                    "CAST(LENGTH({ident}) AS VARCHAR(32)) AS {}",
                    quote_sql_identifier(&lob_len_alias(index))
                )
            } else if column.is_rowid() {
                format!("HEX({ident}) AS {ident}")
            } else {
                format!(
                    "CAST({ident} AS VARCHAR({})) AS {ident}",
                    zos_base_scalar_cast_len(column)
                )
            }
        })
        .collect::<Vec<_>>();
    let chunk_projection = specs
        .iter()
        .map(|spec| {
            let column = &columns[spec.column_index];
            let ident = quote_sql_identifier(&column.name);
            build_zos_lob_chunk_projection(
                &ident,
                column,
                spec.start,
                spec.len,
                &lob_chunk_column_alias(spec),
            )
        })
        .collect::<Vec<_>>();
    let projection = base_projection
        .into_iter()
        .chain(chunk_projection)
        .collect::<Vec<_>>();
    let suffix = parsed.suffix.trim();
    if suffix.is_empty() {
        format!(
            "SELECT ROW_NUMBER() OVER() AS \"DB2NODE_RN\", {} FROM {}",
            projection.join(", "),
            parsed.table_ref
        )
    } else {
        format!(
            "SELECT ROW_NUMBER() OVER() AS \"DB2NODE_RN\", {} FROM {} {}",
            projection.join(", "),
            parsed.table_ref,
            suffix
        )
    }
}

fn build_zos_lob_chunk_projection(
    ident: &str,
    column: &CatalogColumn,
    start: usize,
    len: usize,
    alias: &str,
) -> String {
    let cast_type = if column.is_dbclob() {
        format!("VARGRAPHIC({len})")
    } else {
        format!("VARCHAR({len})")
    };
    format!(
        "CASE WHEN LENGTH({ident}) >= {start} THEN CAST(SUBSTR({ident}, {start}, {len}) AS {cast_type}) ELSE CAST(NULL AS {cast_type}) END AS {}",
        quote_sql_identifier(alias)
    )
}

#[cfg(test)]
fn build_zos_lob_chunk_set_query(
    parsed: &SimpleSelectStar,
    column: &CatalogColumn,
    start: usize,
    len: usize,
    row_start: usize,
    row_end: usize,
) -> String {
    build_zos_lob_chunk_grid_query(parsed, column, &[(start, len)], row_start, row_end)
}

fn zos_lob_chunk_specs(max_lob_len: usize, chunk_limit: usize) -> Vec<(usize, usize)> {
    let mut chunks = Vec::new();
    let mut start = 1usize;
    while start <= max_lob_len {
        let len = chunk_limit.min(max_lob_len - start + 1);
        chunks.push((start, len));
        start += len;
    }
    chunks
}

fn zos_lob_combined_chunk_specs(
    columns: &[CatalogColumn],
    lob_lengths_by_column: &[Vec<Option<usize>>],
) -> Vec<LobChunkSpec> {
    let mut specs = Vec::new();
    for (column_index, column) in columns.iter().enumerate() {
        if !column.is_lob() {
            continue;
        }
        let max_lob_len = lob_lengths_by_column
            .get(column_index)
            .and_then(|lengths| lengths.iter().flatten().copied().max())
            .unwrap_or(0);
        if max_lob_len == 0 {
            continue;
        }

        let chunk_limit = zos_lob_chunk_limit(column);
        for (chunk_index, (start, len)) in zos_lob_chunk_specs(max_lob_len, chunk_limit)
            .into_iter()
            .enumerate()
        {
            specs.push(LobChunkSpec {
                column_index,
                chunk_number: chunk_index + 1,
                start,
                len,
            });
        }
    }
    specs
}

fn zos_lob_initial_chunk_specs(columns: &[CatalogColumn]) -> Vec<LobChunkSpec> {
    let lob_indices = columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| column.is_lob().then_some(index))
        .collect::<Vec<_>>();
    if lob_indices.is_empty() {
        return Vec::new();
    }

    let mut specs = Vec::new();
    let mut window_bytes = 0usize;
    let mut chunk_number = 1usize;
    loop {
        let mut pushed = false;
        for column_index in &lob_indices {
            let column = &columns[*column_index];
            let chunk_limit = zos_lob_chunk_limit(column);
            let spec = LobChunkSpec {
                column_index: *column_index,
                chunk_number,
                start: ((chunk_number - 1) * chunk_limit) + 1,
                len: chunk_limit,
            };
            let spec_bytes = zos_lob_chunk_spec_estimated_bytes(columns, spec);
            if !specs.is_empty() && window_bytes + spec_bytes > zos_lob_chunk_window_target() {
                return specs;
            }
            specs.push(spec);
            window_bytes += spec_bytes;
            pushed = true;
        }
        if !pushed {
            break;
        }
        chunk_number += 1;
    }
    specs
}

fn zos_lob_chunk_limit(column: &CatalogColumn) -> usize {
    if column.is_dbclob() {
        ZOS_DBCLOB_CHUNK_LIMIT
    } else {
        ZOS_CLOB_CHUNK_LIMIT
    }
}

fn zos_lob_batch_reply_target() -> usize {
    env_usize(
        "DB2_ZOS_LOB_BATCH_BYTES",
        ZOS_LOB_BATCH_REPLY_TARGET,
        16_000,
        4_000_000,
    )
}

fn zos_lob_chunk_window_target() -> usize {
    env_usize(
        "DB2_ZOS_LOB_CHUNK_WINDOW_BYTES",
        ZOS_LOB_CHUNK_WINDOW_TARGET,
        8_000,
        512_000,
    )
}

fn use_zos_non_lob_extra_blocks() -> bool {
    env::var("DB2_ZOS_NON_LOB_EXTRA_BLOCKS")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_non_lob_sql_rowset_cap() -> bool {
    env::var("DB2_ZOS_NON_LOB_SQL_ROWSET_CAP")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_non_lob_open_rowset() -> bool {
    env::var("DB2_ZOS_NON_LOB_OPEN_ROWSET")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}

fn use_zos_non_lob_cached_open_fetch_pipeline() -> bool {
    env::var("DB2_ZOS_NON_LOB_CACHED_OPEN_FETCH_PIPELINE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        // CNTQRY is tied to the active OPNQRY reply's query instance id. Keep
        // speculative open+fetch pipelining opt-in so hot non-LOB reads do not
        // pay an idle drain when z/OS waits for a normal CNTQRY.
        .unwrap_or(false)
}

fn use_zos_non_lob_excsqlstt_output() -> bool {
    env::var("DB2_ZOS_NON_LOB_EXCSQLSTT")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}

fn use_zos_like_predicate_excsqlstt_output() -> bool {
    env::var("DB2_ZOS_LIKE_PREDICATE_EXCSQLSTT")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_non_lob_open_data_drain() -> bool {
    env::var("DB2_ZOS_NON_LOB_OPEN_DATA_DRAIN")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}

fn use_zos_non_lob_close_with_limited_fetch() -> bool {
    env::var("DB2_ZOS_NON_LOB_CLOSE_WITH_LIMITED_FETCH")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_select_sql_optimization() -> bool {
    env::var("DB2_ZOS_SELECT_SQL_OPTIMIZATION")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn zos_non_lob_open_drain_timeout() -> Duration {
    Duration::from_millis(env_usize("DB2_ZOS_NON_LOB_OPEN_DRAIN_MS", 2, 0, 25) as u64)
}

pub(crate) fn zos_non_lob_qryblksz() -> u32 {
    let value = env_usize(
        "DB2_ZOS_NON_LOB_QRYBLKSZ",
        ZOS_NON_LOB_QRYBLKSZ_DEFAULT,
        ZOS_NON_LOB_QRYBLKSZ_MIN,
        ZOS_NON_LOB_QRYBLKSZ_MAX,
    );
    normalize_zos_non_lob_qryblksz(value) as u32
}

fn normalize_zos_non_lob_qryblksz(value: usize) -> usize {
    let clamped = value.clamp(ZOS_NON_LOB_QRYBLKSZ_MIN, ZOS_NON_LOB_QRYBLKSZ_MAX);
    let offset = clamped - ZOS_NON_LOB_QRYBLKSZ_MIN;
    let lower =
        ZOS_NON_LOB_QRYBLKSZ_MIN + (offset / ZOS_NON_LOB_QRYBLKSZ_STEP) * ZOS_NON_LOB_QRYBLKSZ_STEP;
    let upper = (lower + ZOS_NON_LOB_QRYBLKSZ_STEP).min(ZOS_NON_LOB_QRYBLKSZ_MAX);

    if clamped - lower <= upper - clamped {
        lower
    } else {
        upper
    }
}

fn zos_non_lob_open_data_drain_timeout() -> Duration {
    Duration::from_millis(env_usize("DB2_ZOS_NON_LOB_OPEN_DATA_DRAIN_MS", 20, 0, 100) as u64)
}

fn zos_non_lob_cached_fetch_drain_timeout() -> Duration {
    Duration::from_millis(env_usize("DB2_ZOS_NON_LOB_CACHED_FETCH_DRAIN_MS", 10, 0, 250) as u64)
}

fn skip_zos_native_lob_initial_drain() -> bool {
    env::var("DB2_ZOS_NATIVE_LOB_INITIAL_DRAIN")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            value == "0" || value == "false" || value == "off" || value == "no"
        })
        .unwrap_or(true)
}

pub(crate) fn use_zos_read_only_cursor_attributes() -> bool {
    env::var("DB2_ZOS_READ_ONLY_CURSOR_ATTR")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn zos_lob_frame_drain_timeout() -> Duration {
    Duration::from_millis(env_usize(
        "DB2_ZOS_LOB_FRAME_DRAIN_MS",
        ZOS_LOB_FRAME_DRAIN_TIMEOUT_MS,
        25,
        2_000,
    ) as u64)
}

pub(crate) fn native_zos_lob_frame_drain_timeout() -> Duration {
    Duration::from_millis(env_usize(
        "DB2_ZOS_NATIVE_LOB_FRAME_DRAIN_MS",
        ZOS_NATIVE_LOB_FRAME_DRAIN_TIMEOUT_MS,
        25,
        2_000,
    ) as u64)
}

fn env_usize(name: &str, default: usize, min: usize, max: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn zos_lob_chunk_spec_estimated_bytes(columns: &[CatalogColumn], spec: LobChunkSpec) -> usize {
    if columns
        .get(spec.column_index)
        .is_some_and(CatalogColumn::is_dbclob)
    {
        spec.len.saturating_mul(2)
    } else {
        spec.len
    }
}

fn zos_lob_combined_rows_per_batch(columns: &[CatalogColumn], specs: &[LobChunkSpec]) -> usize {
    let estimated_bytes_per_row = specs
        .iter()
        .map(|spec| zos_lob_chunk_spec_estimated_bytes(columns, *spec))
        .sum::<usize>();

    (zos_lob_batch_reply_target() / estimated_bytes_per_row.max(1)).max(1)
}

#[cfg(test)]
fn zos_lob_rows_per_batch(column: &CatalogColumn, chunk_chars_per_row: usize) -> usize {
    let estimated_bytes_per_row = if column.is_dbclob() {
        chunk_chars_per_row.saturating_mul(2)
    } else {
        chunk_chars_per_row
    };

    (zos_lob_batch_reply_target() / estimated_bytes_per_row.max(1)).max(1)
}

fn zos_lob_spec_window_applies_to_rows(
    specs: &[LobChunkSpec],
    lob_lengths_by_column: &[Vec<Option<usize>>],
    row_start: usize,
    row_end: usize,
) -> bool {
    specs.iter().any(|spec| {
        lob_lengths_by_column
            .get(spec.column_index)
            .is_some_and(|lengths| {
                lengths[row_start..row_end]
                    .iter()
                    .flatten()
                    .any(|lob_len| *lob_len >= spec.start)
            })
    })
}

#[cfg(test)]
fn append_zos_lob_chunk_grid_rows(
    chunk_result: &QueryResult,
    column_index: usize,
    lob_lengths: &[Option<usize>],
    output_values: &mut [Vec<db2_proto::types::Db2Value>],
    chunks: &[(usize, usize)],
) -> Result<(), Error> {
    for (result_index, row) in chunk_result.rows.iter().enumerate() {
        let row_number = match row.values().first() {
            Some(value) => db2_value_to_usize(value)?.unwrap_or(result_index + 1),
            None => result_index + 1,
        };
        let Some(row_index) = row_number.checked_sub(1) else {
            continue;
        };
        let Some(lob_len) = lob_lengths.get(row_index).and_then(|lob_len| *lob_len) else {
            continue;
        };
        if row_index >= output_values.len() {
            continue;
        }

        for (chunk_index, (start, _len)) in chunks.iter().enumerate() {
            if *start > lob_len {
                continue;
            }
            let chunk = row
                .values()
                .get(chunk_index + 1)
                .and_then(db2_value_to_string)
                .unwrap_or_default();
            match output_values
                .get_mut(row_index)
                .and_then(|values| values.get_mut(column_index))
            {
                Some(db2_proto::types::Db2Value::Clob(text)) => {
                    let remaining = lob_len.saturating_sub(text.chars().count());
                    let chunk = trim_zos_lob_chunk_to_remaining(&chunk, remaining);
                    text.push_str(chunk);
                }
                Some(value @ db2_proto::types::Db2Value::Null) if !chunk.is_empty() => {
                    let chunk = trim_zos_lob_chunk_to_remaining(&chunk, lob_len);
                    if !chunk.is_empty() {
                        *value = db2_proto::types::Db2Value::Clob(chunk.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    Ok(())
}

fn append_zos_lob_combined_chunk_grid_rows(
    chunk_result: &QueryResult,
    specs: &[LobChunkSpec],
    lob_lengths_by_column: &[Vec<Option<usize>>],
    output_values: &mut [Vec<db2_proto::types::Db2Value>],
    first_chunk_value_index: usize,
) -> Result<(), Error> {
    for (result_index, row) in chunk_result.rows.iter().enumerate() {
        let row_number = match row.values().first() {
            Some(value) => db2_value_to_usize(value)?.unwrap_or(result_index + 1),
            None => result_index + 1,
        };
        let Some(row_index) = row_number.checked_sub(1) else {
            continue;
        };
        if row_index >= output_values.len() {
            continue;
        }

        for (spec_index, spec) in specs.iter().enumerate() {
            let Some(lob_len) = lob_lengths_by_column
                .get(spec.column_index)
                .and_then(|lengths| lengths.get(row_index))
                .and_then(|lob_len| *lob_len)
            else {
                continue;
            };
            if spec.start > lob_len {
                continue;
            }
            let chunk = row
                .values()
                .get(first_chunk_value_index + spec_index)
                .and_then(db2_value_as_str)
                .unwrap_or("");
            append_zos_lob_chunk_to_value(
                output_values,
                row_index,
                spec.column_index,
                lob_len,
                spec.start,
                chunk,
            );
        }
    }

    Ok(())
}

fn materialize_zos_lob_initial_grid_rows(
    initial_result: &QueryResult,
    catalog_columns: &[CatalogColumn],
    specs: &[LobChunkSpec],
) -> Result<LobInitialGridRows, Error> {
    let mut output_values = Vec::with_capacity(initial_result.rows.len());
    let mut lob_lengths_by_column =
        vec![vec![None; initial_result.rows.len()]; catalog_columns.len()];

    for (row_index, row) in initial_result.rows.iter().enumerate() {
        let mut values = Vec::with_capacity(catalog_columns.len());
        for (column_index, column) in catalog_columns.iter().enumerate() {
            let value = row
                .values()
                .get(1 + column_index)
                .cloned()
                .unwrap_or(db2_proto::types::Db2Value::Null);
            if column.is_lob() {
                let Some(lob_len) = db2_value_to_usize(&value)? else {
                    values.push(db2_proto::types::Db2Value::Null);
                    continue;
                };
                lob_lengths_by_column[column_index][row_index] = Some(lob_len);
                values.push(db2_proto::types::Db2Value::Clob(String::with_capacity(
                    lob_len,
                )));
            } else {
                values.push(normalize_zos_materialized_scalar_value(column, value));
            }
        }
        output_values.push(values);
    }

    append_zos_lob_combined_chunk_grid_rows(
        initial_result,
        specs,
        &lob_lengths_by_column,
        &mut output_values,
        1 + catalog_columns.len(),
    )?;

    Ok((output_values, lob_lengths_by_column))
}

fn append_zos_lob_chunk_to_value(
    output_values: &mut [Vec<db2_proto::types::Db2Value>],
    row_index: usize,
    column_index: usize,
    lob_len: usize,
    chunk_start: usize,
    chunk: &str,
) {
    let remaining = lob_len.saturating_sub(chunk_start.saturating_sub(1));
    let chunk = trim_zos_lob_chunk_to_remaining(chunk, remaining);
    if chunk.is_empty() {
        return;
    }

    match output_values
        .get_mut(row_index)
        .and_then(|values| values.get_mut(column_index))
    {
        Some(db2_proto::types::Db2Value::Clob(text)) => {
            text.push_str(chunk);
        }
        Some(value @ db2_proto::types::Db2Value::Null) => {
            *value = db2_proto::types::Db2Value::Clob(chunk.to_string());
        }
        _ => {}
    }
}

#[cfg(test)]
fn append_zos_lob_chunk_rows(
    chunk_result: &QueryResult,
    column_index: usize,
    lob_lengths: &[Option<usize>],
    output_values: &mut [Vec<db2_proto::types::Db2Value>],
    start: usize,
) -> Result<(), Error> {
    append_zos_lob_chunk_grid_rows(
        chunk_result,
        column_index,
        lob_lengths,
        output_values,
        &[(start, usize::MAX)],
    )
}

fn trim_zos_lob_chunk_to_remaining(chunk: &str, remaining_chars: usize) -> &str {
    if remaining_chars == 0 {
        return "";
    }

    match chunk.char_indices().nth(remaining_chars) {
        Some((byte_index, _)) => &chunk[..byte_index],
        None => chunk,
    }
}

#[cfg(test)]
fn build_zos_numbered_single_column_source_sql(parsed: &SimpleSelectStar, ident: &str) -> String {
    if parsed.suffix.trim().is_empty() {
        format!(
            "SELECT {ident}, ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM {}",
            parsed.table_ref
        )
    } else {
        format!(
            "SELECT {ident}, ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM {} {}",
            parsed.table_ref, parsed.suffix
        )
    }
}

fn build_zos_numbered_multi_column_source_sql(
    parsed: &SimpleSelectStar,
    idents: &[String],
) -> String {
    let projection = if idents.is_empty() {
        "1".to_string()
    } else {
        idents.join(", ")
    };
    if parsed.suffix.trim().is_empty() {
        format!(
            "SELECT {projection}, ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM {}",
            parsed.table_ref
        )
    } else {
        format!(
            "SELECT {projection}, ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM {} {}",
            parsed.table_ref, parsed.suffix
        )
    }
}

fn lob_len_alias(index: usize) -> String {
    format!("DB2NODE_LOB_LEN_{}", index + 1)
}

#[cfg(test)]
fn lob_chunk_alias(index: usize) -> String {
    format!("DB2NODE_LOB_CHUNK_{}", index + 1)
}

fn lob_chunk_column_alias(spec: &LobChunkSpec) -> String {
    format!(
        "DB2NODE_LOB_C{}_K{}",
        spec.column_index + 1,
        spec.chunk_number
    )
}

fn zos_base_scalar_cast_len(column: &CatalogColumn) -> usize {
    let coltype = column.normalized_coltype();
    if coltype.contains("TIMESTAMP") {
        64
    } else if coltype.contains("DATE") || coltype.contains("TIME") {
        32
    } else if coltype.contains("INT")
        || coltype.contains("DEC")
        || coltype.contains("NUM")
        || coltype.contains("REAL")
        || coltype.contains("FLOAT")
        || coltype.contains("DOUBLE")
    {
        128
    } else {
        4096
    }
}

fn zos_lob_output_columns(
    catalog_columns: &[CatalogColumn],
    base_columns: &[ColumnInfo],
    _row_count: i64,
) -> Vec<ColumnInfo> {
    catalog_columns
        .iter()
        .enumerate()
        .map(|(index, column)| {
            let mut info = base_columns.get(index).cloned().unwrap_or_else(|| {
                ColumnInfo::new(column.name.clone(), catalog_column_type_name(column), true)
            });
            info.name = column.name.clone();
            if column.is_clob() {
                info.type_name = "Clob".into();
                info.nullable = true;
            } else if column.is_dbclob() {
                info.type_name = "DbClob".into();
                info.nullable = true;
            } else if column.is_rowid() {
                info.type_name = "RowId(40)".into();
            }
            info
        })
        .collect()
}

fn catalog_column_type_name(column: &CatalogColumn) -> String {
    let coltype = column.normalized_coltype();
    if column.is_clob() {
        "Clob".into()
    } else if column.is_dbclob() {
        "DbClob".into()
    } else if column.is_rowid() {
        "RowId(40)".into()
    } else if coltype.starts_with("DEC")
        || coltype.starts_with("NUM")
        || coltype.starts_with("FLOAT")
        || coltype.starts_with("REAL")
        || coltype.starts_with("DOUBLE")
    {
        "Decimal".into()
    } else if coltype.starts_with("BIGINT") {
        "BigInt".into()
    } else if coltype.starts_with("SMALLINT") {
        "SmallInt".into()
    } else if coltype.starts_with("INT") {
        "Integer".into()
    } else if coltype.starts_with("TIMESTAMP") {
        "Timestamp".into()
    } else if coltype.starts_with("DATE") {
        "Date".into()
    } else if coltype.starts_with("TIME") {
        "Time".into()
    } else if coltype.starts_with("VARCHAR") || coltype.starts_with("VARGRAPHIC") {
        "VarChar".into()
    } else if coltype.starts_with("CHAR") || coltype.starts_with("GRAPHIC") {
        "Char".into()
    } else {
        column.coltype.trim().to_string()
    }
}

fn normalize_zos_materialized_scalar_value(
    column: &CatalogColumn,
    value: db2_proto::types::Db2Value,
) -> db2_proto::types::Db2Value {
    if !column.is_rowid() {
        return value;
    }

    match db2_value_to_string(&value) {
        Some(hex) if !hex.trim().is_empty() => {
            let hex = hex.trim();
            if hex.starts_with("0x") || hex.starts_with("0X") {
                db2_proto::types::Db2Value::RowId(hex.to_string())
            } else {
                db2_proto::types::Db2Value::RowId(format!("0x{hex}"))
            }
        }
        _ => db2_proto::types::Db2Value::Null,
    }
}

fn db2_value_to_usize(value: &db2_proto::types::Db2Value) -> Result<Option<usize>, Error> {
    match value {
        db2_proto::types::Db2Value::Null => Ok(None),
        db2_proto::types::Db2Value::SmallInt(v) => Ok((*v >= 0).then_some(*v as usize)),
        db2_proto::types::Db2Value::Integer(v) => Ok((*v >= 0).then_some(*v as usize)),
        db2_proto::types::Db2Value::BigInt(v) => Ok((*v >= 0).then_some(*v as usize)),
        db2_proto::types::Db2Value::Char(v) | db2_proto::types::Db2Value::VarChar(v) => v
            .trim()
            .parse::<usize>()
            .map(Some)
            .map_err(|_| Error::Protocol(format!("invalid z/OS LOB length value: {v}"))),
        db2_proto::types::Db2Value::Decimal(v) => v
            .trim()
            .parse::<usize>()
            .map(Some)
            .map_err(|_| Error::Protocol(format!("invalid z/OS LOB length value: {v}"))),
        _ => Err(Error::Protocol(format!(
            "unexpected z/OS LOB length value: {:?}",
            value
        ))),
    }
}

fn db2_value_to_string(value: &db2_proto::types::Db2Value) -> Option<String> {
    match value {
        db2_proto::types::Db2Value::Char(v)
        | db2_proto::types::Db2Value::VarChar(v)
        | db2_proto::types::Db2Value::Clob(v)
        | db2_proto::types::Db2Value::Date(v)
        | db2_proto::types::Db2Value::Time(v)
        | db2_proto::types::Db2Value::Timestamp(v)
        | db2_proto::types::Db2Value::Decimal(v)
        | db2_proto::types::Db2Value::RowId(v)
        | db2_proto::types::Db2Value::Xml(v) => Some(v.clone()),
        db2_proto::types::Db2Value::Null => Some(String::new()),
        _ => None,
    }
}

fn db2_value_as_str(value: &db2_proto::types::Db2Value) -> Option<&str> {
    match value {
        db2_proto::types::Db2Value::Char(v)
        | db2_proto::types::Db2Value::VarChar(v)
        | db2_proto::types::Db2Value::Clob(v)
        | db2_proto::types::Db2Value::Date(v)
        | db2_proto::types::Db2Value::Time(v)
        | db2_proto::types::Db2Value::Timestamp(v)
        | db2_proto::types::Db2Value::Decimal(v)
        | db2_proto::types::Db2Value::RowId(v)
        | db2_proto::types::Db2Value::Xml(v) => Some(v.as_str()),
        db2_proto::types::Db2Value::Null => Some(""),
        _ => None,
    }
}

fn should_retry_zos_lob_chunking_after_decode_error(error: &Error) -> bool {
    match error {
        Error::Protocol(message) => {
            message.contains("query ended with undecoded row data")
                || message.contains("query fetch stalled while decoding row data")
                || message.contains("z/OS LOB result requires transparent materialization")
                || message_indicates_retryable_session_state(message)
        }
        Error::Timeout(message) => {
            message.contains("fetch timed out") && message.contains("has_lobs=true")
        }
        Error::Sql { .. } => error_indicates_stale_session_state(error),
        _ => false,
    }
}

fn result_metadata_needs_zos_lob_route(
    columns: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> bool {
    descriptors_need_lob_materialization(columns, descriptors)
}

fn zos_select_section_cacheable(
    columns: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> bool {
    !result_metadata_needs_zos_lob_route(columns, descriptors)
}

fn result_columns_need_zos_lob_route(columns: &[ColumnInfo]) -> bool {
    !columns.is_empty() && result_metadata_needs_zos_lob_route(columns, &[])
}

fn result_has_zos_lob_materialization(result: &QueryResult) -> bool {
    result_columns_need_zos_lob_route(&result.columns)
        || result.rows.iter().any(|row| {
            row.values().iter().any(|value| {
                matches!(
                    value,
                    db2_proto::types::Db2Value::Blob(_) | db2_proto::types::Db2Value::Clob(_)
                )
            })
        })
}

fn descriptors_need_zos_lob_materialization(
    columns: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> bool {
    descriptors_need_lob_materialization(columns, descriptors)
}

fn descriptors_need_lob_materialization(
    columns: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> bool {
    descriptors.iter().any(|descriptor| {
        is_lob_descriptor(descriptor) || descriptor_has_large_lob_inline_type(descriptor)
    }) || column_info_has_lob_hint(columns)
}

fn descriptor_has_large_lob_inline_type(descriptor: &db2_proto::fdoca::ColumnDescriptor) -> bool {
    match descriptor.db2_type {
        db2_proto::types::Db2Type::VarChar(len)
        | db2_proto::types::Db2Type::VarGraphic(len)
        | db2_proto::types::Db2Type::LobBytes(len)
        | db2_proto::types::Db2Type::LobChar(len) => len & 0x8000 != 0 || len >= 32_704,
        _ => false,
    }
}

fn column_info_has_lob_hint(columns: &[ColumnInfo]) -> bool {
    columns.iter().any(|column| {
        let ty = column.type_name.to_ascii_lowercase();
        let name = column.name.to_ascii_lowercase();
        if name.starts_with("db2node_lob_") {
            return false;
        }
        ty.contains("clob")
            || ty.contains("blob")
            || ty.contains("varchar(327")
            || ty.contains("vargraphic(327")
            || name.contains("lob")
    })
}

fn catalog_columns_from_query_result(metadata: &QueryResult) -> Vec<CatalogColumn> {
    let columns = metadata
        .rows
        .iter()
        .filter_map(|row| {
            let name = row
                .get::<String>("NAME")
                .or_else(|| row.get_by_index::<String>(0))?
                .trim()
                .to_string();
            let coltype = row
                .get::<String>("COLTYPE")
                .or_else(|| row.get_by_index::<String>(1))?
                .trim()
                .to_string();
            (!name.is_empty()).then_some(CatalogColumn { name, coltype })
        })
        .collect::<Vec<_>>();
    move_generated_rowid_column_last(columns)
}

fn move_generated_rowid_column_last(columns: Vec<CatalogColumn>) -> Vec<CatalogColumn> {
    let mut visible = Vec::with_capacity(columns.len());
    let mut generated_rowids = Vec::new();
    for column in columns {
        if is_generated_lob_rowid_column_name(&column.name) {
            generated_rowids.push(column);
        } else {
            visible.push(column);
        }
    }
    visible.extend(generated_rowids);
    visible
}

fn catalog_columns_from_prepare_metadata(
    columns: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Vec<CatalogColumn> {
    columns
        .iter()
        .enumerate()
        .filter_map(|(index, column)| {
            let name = column.name.trim();
            if name.is_empty() || is_generated_column_name(name) {
                return None;
            }
            let coltype = prepare_column_catalog_type(column, descriptors.get(index));
            Some(CatalogColumn {
                name: name.to_string(),
                coltype,
            })
        })
        .collect()
}

fn prepare_column_catalog_type(
    column: &ColumnInfo,
    descriptor: Option<&db2_proto::fdoca::ColumnDescriptor>,
) -> String {
    if column
        .name
        .eq_ignore_ascii_case("DB2_GENERATED_ROWID_FOR_LOBS")
        || column.type_name.to_ascii_uppercase().contains("ROWID")
    {
        return "ROWID".into();
    }

    if let Some(descriptor) = descriptor {
        match descriptor.db2_type {
            db2_proto::types::Db2Type::Clob
            | db2_proto::types::Db2Type::ClobLocator
            | db2_proto::types::Db2Type::LobChar(_) => return "CLOB".into(),
            db2_proto::types::Db2Type::DbClob | db2_proto::types::Db2Type::DbClobLocator => {
                return "DBCLOB".into()
            }
            db2_proto::types::Db2Type::VarChar(len) if len & 0x8000 != 0 || len >= 32_704 => {
                return "CLOB".into();
            }
            db2_proto::types::Db2Type::VarGraphic(len) if len & 0x8000 != 0 || len >= 16_352 => {
                return "DBCLOB".into();
            }
            db2_proto::types::Db2Type::RowId(_) => return "ROWID".into(),
            _ => {}
        }
    }

    let normalized_type = column
        .type_name
        .trim()
        .to_ascii_uppercase()
        .replace(' ', "");
    if normalized_type.contains("DBCLOB") || normalized_type.starts_with("VARGRAPHIC(327") {
        "DBCLOB".into()
    } else if normalized_type.contains("CLOB") || normalized_type.starts_with("VARCHAR(327") {
        "CLOB".into()
    } else {
        "OTHER".into()
    }
}

fn selected_catalog_columns(
    parsed: &SimpleSelectStar,
    catalog_columns: &[CatalogColumn],
) -> Option<Vec<CatalogColumn>> {
    match parsed.selected_columns.as_ref() {
        None => Some(
            catalog_columns
                .iter()
                .filter(|column| !is_generated_lob_rowid_column_name(&column.name))
                .cloned()
                .collect(),
        ),
        Some(selected_names) => {
            let mut selected_columns = Vec::with_capacity(selected_names.len());
            for selected_name in selected_names {
                let column = catalog_columns
                    .iter()
                    .find(|column| column.name.eq_ignore_ascii_case(selected_name))?;
                selected_columns.push(column.clone());
            }
            Some(selected_columns)
        }
    }
}

fn is_generated_lob_rowid_column_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("DB2_GENERATED_ROWID_FOR_LOBS")
}

#[cfg(test)]
fn parse_simple_select_star(sql: &str, current_schema: Option<&str>) -> Option<SimpleSelectStar> {
    let parsed = parse_simple_select_for_zos_lobs(sql, current_schema)?;
    parsed.selected_columns.is_none().then_some(parsed)
}

fn parse_simple_select_for_zos_lobs(
    sql: &str,
    current_schema: Option<&str>,
) -> Option<SimpleSelectStar> {
    let trimmed = sql.trim().trim_end_matches(';').trim();
    let mut offset = skip_ascii_whitespace(trimmed, 0);
    offset = consume_keyword(trimmed, offset, "SELECT")?;
    offset = skip_ascii_whitespace(trimmed, offset);
    let projection_start = offset;
    offset = find_from_keyword(trimmed, projection_start)?;
    let projection = trimmed[projection_start..offset].trim();
    offset = skip_ascii_whitespace(trimmed, offset);
    offset = consume_keyword(trimmed, offset, "FROM")?;
    offset = skip_ascii_whitespace(trimmed, offset);

    let table_start = offset;
    offset = consume_table_ref(trimmed, offset)?;
    let table_ref = trimmed[table_start..offset].trim();
    if table_ref.is_empty() || table_ref.starts_with('(') {
        return None;
    }

    let suffix = trimmed[offset..].trim().to_string();
    let parts = split_table_ref_parts(table_ref)?;
    let (schema, table) = match parts.as_slice() {
        [table] => (current_schema?.trim().to_string(), table.clone()),
        [schema, table] => (schema.clone(), table.clone()),
        _ => return None,
    };

    if schema.is_empty() || table.is_empty() {
        return None;
    }

    Some(SimpleSelectStar {
        table_ref: table_ref.to_string(),
        suffix,
        schema,
        table,
        selected_columns: parse_simple_projection(projection)?,
    })
}

fn parse_fetch_first_row_limit(suffix: &str) -> Option<usize> {
    let tokens = suffix.split_whitespace().collect::<Vec<_>>();
    for window in tokens.windows(4) {
        if window[0].eq_ignore_ascii_case("FETCH")
            && window[1].eq_ignore_ascii_case("FIRST")
            && (window[3].eq_ignore_ascii_case("ROW") || window[3].eq_ignore_ascii_case("ROWS"))
        {
            return window[2].parse::<usize>().ok();
        }
    }
    None
}

fn find_from_keyword(input: &str, mut offset: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    while offset < bytes.len() {
        match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    if bytes[offset] == b'"' {
                        offset += 1;
                        if bytes.get(offset) == Some(&b'"') {
                            offset += 1;
                            continue;
                        }
                        break;
                    }
                    offset += 1;
                }
            }
            byte if byte.eq_ignore_ascii_case(&b'f') => {
                if consume_keyword(input, offset, "FROM").is_some() {
                    return Some(offset);
                }
                offset += 1;
            }
            _ => offset += 1,
        }
    }
    None
}

fn parse_simple_projection(projection: &str) -> Option<Option<Vec<String>>> {
    if projection == "*" {
        return Some(None);
    }

    let parts = split_top_level_commas(projection)?;
    let mut columns = Vec::with_capacity(parts.len());
    for part in parts {
        let identifier = strip_optional_identifier_alias(part.trim())?;
        let pieces = split_table_ref_parts(identifier)?;
        let name = match pieces.as_slice() {
            [column] => column.clone(),
            [_qualifier, column] => column.clone(),
            _ => return None,
        };
        columns.push(name);
    }

    (!columns.is_empty()).then_some(Some(columns))
}

fn split_top_level_commas(input: &str) -> Option<Vec<&str>> {
    let bytes = input.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut offset = 0usize;
    while offset < bytes.len() {
        match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    if bytes[offset] == b'"' {
                        offset += 1;
                        if bytes.get(offset) == Some(&b'"') {
                            offset += 1;
                            continue;
                        }
                        break;
                    }
                    offset += 1;
                }
            }
            b',' => {
                parts.push(input[start..offset].trim());
                start = offset + 1;
                offset += 1;
            }
            b'(' | b')' => return None,
            _ => offset += 1,
        }
    }
    parts.push(input[start..].trim());

    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }
    Some(parts)
}

fn strip_optional_identifier_alias(input: &str) -> Option<&str> {
    let mut offset = consume_identifier_ref(input, 0)?;
    let ident = input[..offset].trim();
    offset = skip_ascii_whitespace(input, offset);
    if offset >= input.len() {
        return Some(ident);
    }

    if let Some(after_as) = consume_keyword(input, offset, "AS") {
        offset = skip_ascii_whitespace(input, after_as);
        consume_identifier_ref(input, offset)
            .filter(|end| skip_ascii_whitespace(input, *end) == input.len())?;
        return Some(ident);
    }

    consume_identifier_ref(input, offset)
        .filter(|end| skip_ascii_whitespace(input, *end) == input.len())?;
    Some(ident)
}

fn consume_identifier_ref(input: &str, mut offset: usize) -> Option<usize> {
    offset = consume_identifier_part(input, offset)?;
    while input.as_bytes().get(offset) == Some(&b'.') {
        offset = consume_identifier_part(input, offset + 1)?;
    }
    Some(offset)
}

fn consume_identifier_part(input: &str, mut offset: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    match bytes.get(offset).copied()? {
        b'"' => {
            offset += 1;
            while offset < bytes.len() {
                if bytes[offset] == b'"' {
                    offset += 1;
                    if bytes.get(offset) == Some(&b'"') {
                        offset += 1;
                        continue;
                    }
                    return Some(offset);
                }
                offset += 1;
            }
            Some(offset)
        }
        byte if byte.is_ascii_alphabetic() || byte == b'_' => {
            offset += 1;
            while offset < bytes.len()
                && (bytes[offset].is_ascii_alphanumeric()
                    || matches!(bytes[offset], b'_' | b'@' | b'#' | b'$'))
            {
                offset += 1;
            }
            Some(offset)
        }
        _ => None,
    }
}

fn skip_ascii_whitespace(input: &str, mut offset: usize) -> usize {
    while input
        .as_bytes()
        .get(offset)
        .is_some_and(u8::is_ascii_whitespace)
    {
        offset += 1;
    }
    offset
}

fn consume_keyword(input: &str, offset: usize, keyword: &str) -> Option<usize> {
    let end = offset.checked_add(keyword.len())?;
    let candidate = input.get(offset..end)?;
    if !candidate.eq_ignore_ascii_case(keyword) {
        return None;
    }
    if input
        .as_bytes()
        .get(end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        return None;
    }
    Some(end)
}

fn consume_table_ref(input: &str, mut offset: usize) -> Option<usize> {
    let bytes = input.as_bytes();
    let mut saw_char = false;
    while offset < bytes.len() {
        match bytes[offset] {
            b'"' => {
                saw_char = true;
                offset += 1;
                while offset < bytes.len() {
                    if bytes[offset] == b'"' {
                        offset += 1;
                        if bytes.get(offset) == Some(&b'"') {
                            offset += 1;
                            continue;
                        }
                        break;
                    }
                    offset += 1;
                }
            }
            byte if byte.is_ascii_whitespace() => break,
            b';' => break,
            _ => {
                saw_char = true;
                offset += 1;
            }
        }
    }
    saw_char.then_some(offset)
}

fn split_table_ref_parts(table_ref: &str) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let bytes = table_ref.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        match bytes[offset] {
            b'"' => {
                offset += 1;
                while offset < bytes.len() {
                    if bytes[offset] == b'"' {
                        offset += 1;
                        if bytes.get(offset) == Some(&b'"') {
                            current.push('"');
                            offset += 1;
                            continue;
                        }
                        break;
                    }
                    current.push(bytes[offset] as char);
                    offset += 1;
                }
            }
            b'.' => {
                parts.push(current.trim().to_string());
                current.clear();
                offset += 1;
            }
            byte => {
                current.push(byte as char);
                offset += 1;
            }
        }
    }
    parts.push(current.trim().to_string());

    if parts.iter().any(|part| part.is_empty()) {
        return None;
    }

    Some(parts)
}

fn escape_sql_string_literal(value: &str) -> String {
    value.replace('\'', "''")
}

fn summarize_sql_for_diagnostics(sql: &str) -> String {
    let mut compact = sql.split_whitespace().collect::<Vec<_>>().join(" ");
    const MAX_LEN: usize = 320;
    if compact.len() > MAX_LEN {
        compact.truncate(MAX_LEN);
        compact.push_str("...");
    }
    compact
}

fn sanitize_diagnostic_value(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join("_")
}

pub(crate) fn use_native_zos_lob_strategy() -> bool {
    env::var("DB2_ZOS_LOB_STRATEGY")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "sql" || value == "substr" || value == "fallback" || value == "off")
        })
        .unwrap_or(true)
        || use_zos_native_lob_only()
}

fn use_zos_native_lob_only() -> bool {
    env::var_os("DB2_ZOS_NATIVE_LOB_ONLY").is_some()
}

fn use_zos_select_cache() -> bool {
    env::var("DB2_ZOS_SELECT_CACHE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_select_metadata_cache() -> bool {
    env::var("DB2_ZOS_SELECT_METADATA_CACHE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_select_cached_empty_retry() -> bool {
    env::var("DB2_ZOS_SELECT_CACHED_EMPTY_RETRY")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn use_zos_lob_commit_after_materialization() -> bool {
    env::var("DB2_ZOS_LOB_COMMIT_AFTER_MATERIALIZE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}

fn use_zos_lob_close_after_materialization() -> bool {
    env::var("DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}

fn use_zos_lob_passive_tail_before_close() -> bool {
    env::var("DB2_ZOS_LOB_PASSIVE_TAIL_BEFORE_CLOSE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
}

fn use_zos_lob_disconnect_after_materialization() -> bool {
    env::var("DB2_ZOS_LOB_DISCONNECT_AFTER_MATERIALIZE")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn lookup_zos_select_metadata(key: &str) -> Option<CachedZosSelectMetadata> {
    ZOS_SELECT_METADATA_CACHE
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|cache| cache.get(key).cloned())
}

fn remove_zos_select_metadata(key: &str) -> bool {
    ZOS_SELECT_METADATA_CACHE
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
        .ok()
        .and_then(|mut cache| cache.remove(key))
        .is_some()
}

fn zos_select_lob_cache_denied(key: &str) -> bool {
    ZOS_SELECT_LOB_CACHE_DENYLIST
        .get_or_init(|| StdMutex::new(HashSet::new()))
        .lock()
        .ok()
        .is_some_and(|cache| cache.contains(key))
}

fn mark_zos_select_lob_cache_denied(key: &str) -> bool {
    ZOS_SELECT_LOB_CACHE_DENYLIST
        .get_or_init(|| StdMutex::new(HashSet::new()))
        .lock()
        .ok()
        .is_some_and(|mut cache| cache.insert(key.to_string()))
}

fn store_zos_select_metadata(
    key: &str,
    column_info: &[ColumnInfo],
    result_descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> bool {
    if column_info.is_empty()
        || result_metadata_needs_zos_lob_route(column_info, result_descriptors)
    {
        return false;
    }

    let Ok(mut cache) = ZOS_SELECT_METADATA_CACHE
        .get_or_init(|| StdMutex::new(HashMap::new()))
        .lock()
    else {
        return false;
    };

    if cache.len() >= ZOS_SELECT_METADATA_CACHE_MAX_ENTRIES && !cache.contains_key(key) {
        if let Some(first_key) = cache.keys().next().cloned() {
            cache.remove(&first_key);
        }
    }

    cache.insert(
        key.to_string(),
        CachedZosSelectMetadata {
            column_info: column_info.to_vec(),
            result_descriptors: result_descriptors.to_vec(),
        },
    );
    true
}

pub(crate) fn query_diagnostics_enabled() -> bool {
    debug_hex_enabled()
        || env::var("DB2_QUERY_DIAGNOSTICS")
            .map(|value| {
                let value = value.trim().to_ascii_lowercase();
                !(value == "0" || value == "false" || value == "off" || value == "no")
            })
            .unwrap_or(false)
}

fn should_reprepare_cached_zos_select(err: &Error) -> bool {
    matches!(
        err,
        Error::Sql {
            sqlcode: -514 | -518 | -805,
            ..
        }
    )
}

#[cfg(test)]
fn rewrite_zos_lob_select(sql: &str, columns: &[ColumnInfo]) -> Option<String> {
    if columns.is_empty() || !sql_is_query(sql) {
        return None;
    }

    let mut has_materialized_lob = false;
    let projection = columns
        .iter()
        .map(|column| {
            let ident = quote_sql_identifier(&column.name);
            match column.type_name.as_str() {
                "Clob" => {
                    has_materialized_lob = true;
                    format!(
                        "CAST(SUBSTR({ident}, 1, {ZOS_CLOB_INLINE_LIMIT}) AS VARCHAR({ZOS_CLOB_INLINE_LIMIT})) AS {ident}"
                    )
                }
                "DbClob" => {
                    has_materialized_lob = true;
                    format!(
                        "CAST(SUBSTR({ident}, 1, {ZOS_DBCLOB_INLINE_LIMIT}) AS VARGRAPHIC({ZOS_DBCLOB_INLINE_LIMIT})) AS {ident}"
                    )
                }
                _ => ident,
            }
        })
        .collect::<Vec<_>>();

    if !has_materialized_lob {
        return None;
    }

    Some(format!(
        "SELECT {} FROM ({}) AS DB2NODE_LOB_SRC",
        projection.join(", "),
        sql.trim().trim_end_matches(';').trim()
    ))
}

fn quote_sql_identifier(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
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

    let scanned_names = db2_proto::replies::sqldard::scan_column_names(&obj.data);
    let has_only_generated_names = dard
        .columns
        .iter()
        .all(|col| is_generated_column_name(&col.name));

    if !dard.columns.is_empty()
        && has_only_generated_names
        && scanned_names.len() == dard.columns.len()
    {
        return dard
            .columns
            .into_iter()
            .zip(scanned_names)
            .map(|(col, name)| column_info_from_sqldard_metadata(col, Some(name)))
            .collect();
    }

    if dard.columns.is_empty() && scanned_names.len() >= 2 {
        return scanned_names
            .into_iter()
            .map(|name| ColumnInfo {
                name,
                type_name: "Unknown".to_string(),
                nullable: true,
                precision: None,
                scale: None,
            })
            .collect();
    }

    dard.columns
        .into_iter()
        .map(|col| column_info_from_sqldard_metadata(col, None))
        .collect()
}

fn column_info_from_sqldard_metadata(
    col: db2_proto::replies::sqldard::ColumnMetadata,
    name_override: Option<String>,
) -> ColumnInfo {
    ColumnInfo {
        name: name_override.unwrap_or_else(|| col.name.clone()),
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
    }
}

fn is_generated_column_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("COL") else {
        return false;
    };
    !rest.is_empty() && rest.bytes().all(|byte| byte.is_ascii_digit())
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
            byte_order: col.byte_order,
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
                    byte_order: db2_proto::fdoca::ByteOrder::LittleEndian,
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
            byte_order: db2_proto::fdoca::ByteOrder::LittleEndian,
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
        904 => Db2Type::RowId(raw_length.min(u16::MAX as u64) as u16),
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
                byte_order: db2_proto::fdoca::ByteOrder::LittleEndian,
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
        Db2Type::Graphic(len) => encode_fixed_graphic(value, *len as usize, descriptor)?,
        Db2Type::VarGraphic(_) | Db2Type::DbClob | Db2Type::LobChar(_) => {
            encode_ld_graphic(value, descriptor)?
        }
        Db2Type::Binary(len) => encode_fixed_binary(value, *len as usize)?,
        Db2Type::VarBinary(_) | Db2Type::Blob | Db2Type::LobBytes(_) => encode_ld_binary(value)?,
        Db2Type::RowId(len) => encode_fixed_string(value, *len as usize, descriptor.ccsid)?,
        Db2Type::Date => encode_exact_string(value, 10, descriptor.ccsid)?,
        Db2Type::Time => encode_exact_string(value, 8, descriptor.ccsid)?,
        Db2Type::Timestamp => encode_timestamp(value, descriptor.ccsid)?,
        Db2Type::Boolean => vec![if expect_bool(value)? { 1 } else { 0 }],
        Db2Type::BlobLocator | Db2Type::ClobLocator | Db2Type::DbClobLocator | Db2Type::Null => {
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
    let text = extract_text(value)?;

    if matches!(ccsid, 37 | 500) {
        Ok(db2_proto::codepage::utf8_to_ebcdic037(text))
    } else {
        Ok(text.as_bytes().to_vec())
    }
}

fn extract_text(value: &db2_proto::types::Db2Value) -> Result<&str, Error> {
    match value {
        db2_proto::types::Db2Value::Char(v)
        | db2_proto::types::Db2Value::VarChar(v)
        | db2_proto::types::Db2Value::Clob(v)
        | db2_proto::types::Db2Value::Date(v)
        | db2_proto::types::Db2Value::Time(v)
        | db2_proto::types::Db2Value::Timestamp(v)
        | db2_proto::types::Db2Value::Decimal(v)
        | db2_proto::types::Db2Value::RowId(v)
        | db2_proto::types::Db2Value::Xml(v) => Ok(v.as_str()),
        _ => Err(Error::Other(format!(
            "expected string-compatible parameter, got {:?}",
            value
        ))),
    }
}

fn encode_fixed_graphic(
    value: &db2_proto::types::Db2Value,
    len: usize,
    descriptor: &db2_proto::fdoca::ColumnDescriptor,
) -> Result<Vec<u8>, Error> {
    let mut bytes = encode_graphic_text_bytes(value)?;
    let target_len = if graphic_length_is_character_count(descriptor) {
        len.saturating_mul(2)
    } else {
        len
    };
    if bytes.len() > target_len {
        bytes.truncate(target_len);
    } else if bytes.len() < target_len {
        bytes.resize(target_len, 0x00);
    }
    Ok(bytes)
}

fn encode_ld_graphic(
    value: &db2_proto::types::Db2Value,
    descriptor: &db2_proto::fdoca::ColumnDescriptor,
) -> Result<Vec<u8>, Error> {
    let bytes = encode_graphic_text_bytes(value)?;
    let length = if graphic_length_is_character_count(descriptor) {
        bytes.len() / 2
    } else {
        bytes.len()
    };
    if length > u16::MAX as usize {
        return Err(Error::Other(
            "graphic string parameter too large for SQLDTA".into(),
        ));
    }
    let mut out = Vec::with_capacity(2 + bytes.len());
    out.extend_from_slice(&(length as u16).to_be_bytes());
    out.extend_from_slice(&bytes);
    Ok(out)
}

fn encode_graphic_text_bytes(value: &db2_proto::types::Db2Value) -> Result<Vec<u8>, Error> {
    Ok(extract_text(value)?
        .encode_utf16()
        .flat_map(u16::to_be_bytes)
        .collect())
}

fn graphic_length_is_character_count(descriptor: &db2_proto::fdoca::ColumnDescriptor) -> bool {
    descriptor.ccsid == 1200
        && matches!(
            descriptor.byte_order,
            db2_proto::fdoca::ByteOrder::LittleEndian
        )
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
        | Db2Type::VarGraphic(len)
        | Db2Type::LobBytes(len)
        | Db2Type::LobChar(len)
        | Db2Type::RowId(len) => *len,
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
        Db2Type::BlobLocator | Db2Type::ClobLocator | Db2Type::DbClobLocator => 4,
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
        Db2Type::VarBinary(_) | Db2Type::Blob | Db2Type::LobBytes(_) => 0x28,
        Db2Type::BlobLocator => 0x18,
        Db2Type::ClobLocator => 0x1A,
        Db2Type::DbClobLocator => 0x1C,
        Db2Type::RowId(_) => 0x26,
        Db2Type::Date => 0x20,
        Db2Type::Time => 0x22,
        Db2Type::Timestamp => 0x24,
        Db2Type::Graphic(_) => 0x36,
        Db2Type::VarGraphic(_) | Db2Type::DbClob | Db2Type::LobChar(_) => 0x38,
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
    prefer_sqldard_descriptors: bool,
    query_instance_id: &mut Option<Vec<u8>>,
    pending_row_bytes: &mut Vec<u8>,
    extdta_payloads: &mut Vec<Vec<u8>>,
    end_of_query: &mut bool,
    collect_diagnostics: bool,
    diagnostics: &mut Vec<String>,
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
                            if collect_diagnostics {
                                diagnostics.push(format!(
                                    "qrydsc_descriptors count={} {}",
                                    descriptors.len(),
                                    descriptor_summary(&descriptors)
                                ));
                            }
                            *qrydsc_descriptors = Some(descriptors);
                            decode_pending_query_data(
                                column_info,
                                rows,
                                sqldard_descriptors,
                                qrydsc_descriptors,
                                prefer_sqldard_descriptors,
                                pending_row_bytes,
                                collect_diagnostics,
                                diagnostics,
                            )?;
                        } else if debug_hex_enabled() {
                            eprintln!("[db2-wire] parsed 0 QRYDSC descriptor(s)");
                        }
                    } else if debug_hex_enabled() {
                        eprintln!("[db2-wire] QRYDSC parse failed");
                    }
                }
                codepoints::QRYDTA => {
                    trace!("Received QRYDTA");
                    let active_descriptors = preferred_descriptor_vec(
                        sqldard_descriptors.as_ref(),
                        qrydsc_descriptors.as_ref(),
                        prefer_sqldard_descriptors,
                    );
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
                        let rows_before = rows.len();
                        let pending_before = pending_row_bytes.len();
                        let decoded_rows = db2_proto::fdoca::decode_rows_with_tail(
                            &obj.data,
                            descs,
                            pending_row_bytes,
                        )
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                        let decoded_count = decoded_rows.len();
                        if let Some(row_width) = decoded_rows.first().map(|values| values.len()) {
                            let col_names: Arc<[String]> =
                                row_column_names(column_info, row_width).into();
                            for values in decoded_rows {
                                rows.push(Row::new_shared(col_names.clone(), values));
                            }
                        }
                        if collect_diagnostics {
                            diagnostics.push(format!(
                                "qrydta_decode data_len={} pending_before={} rows_decoded={} rows_total={} pending_after={} descriptors={} preview={}",
                                obj.data.len(),
                                pending_before,
                                decoded_count,
                                rows.len(),
                                pending_row_bytes.len(),
                                descs.len(),
                                format_hex_preview(&obj.data, 128)
                            ));
                            if decoded_count == 0 || rows.len() == rows_before {
                                let progress_bytes = if pending_row_bytes.is_empty() {
                                    obj.data.as_slice()
                                } else {
                                    pending_row_bytes.as_slice()
                                };
                                diagnostics.push(format!(
                                    "qrydta_decode progress={}",
                                    db2_proto::fdoca::describe_decode_progress(
                                        progress_bytes,
                                        descs
                                    )
                                ));
                                diagnostics.push(format!(
                                    "qrydta_decode descriptors {}",
                                    descriptor_summary(descs)
                                ));
                            }
                        }
                    } else {
                        if debug_hex_enabled() {
                            eprintln!(
                                "[db2-wire] buffering QRYDTA len={} until descriptors arrive",
                                obj.data.len()
                            );
                        }
                        if collect_diagnostics {
                            diagnostics.push(format!(
                                "qrydta_buffered_without_descriptors len={} preview={}",
                                obj.data.len(),
                                format_hex_preview(&obj.data, 128)
                            ));
                        }
                        pending_row_bytes.extend_from_slice(&obj.data);
                    }
                }
                codepoints::EXTDTA => {
                    trace!("Received EXTDTA");
                    if collect_diagnostics {
                        diagnostics.push(format!(
                            "extdta len={} preview={}",
                            obj.data.len(),
                            format_hex_preview(&obj.data, 128)
                        ));
                    }
                    extdta_payloads.push(obj.data);
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
                        if collect_diagnostics {
                            diagnostics.push(format!(
                                "sqldard_descriptors count={} {}",
                                descriptors.len(),
                                descriptor_summary(&descriptors)
                            ));
                        }
                        *sqldard_descriptors = Some(descriptors);
                        decode_pending_query_data(
                            column_info,
                            rows,
                            sqldard_descriptors,
                            qrydsc_descriptors,
                            prefer_sqldard_descriptors,
                            pending_row_bytes,
                            collect_diagnostics,
                            diagnostics,
                        )?;
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

#[allow(clippy::too_many_arguments)]
fn decode_pending_query_data(
    column_info: &[ColumnInfo],
    rows: &mut Vec<Row>,
    sqldard_descriptors: &Option<Vec<db2_proto::fdoca::ColumnDescriptor>>,
    qrydsc_descriptors: &Option<Vec<db2_proto::fdoca::ColumnDescriptor>>,
    prefer_sqldard_descriptors: bool,
    pending_row_bytes: &mut Vec<u8>,
    collect_diagnostics: bool,
    diagnostics: &mut Vec<String>,
) -> Result<(), Error> {
    if pending_row_bytes.is_empty() {
        return Ok(());
    }

    let Some(descs) = preferred_descriptor_vec(
        sqldard_descriptors.as_ref(),
        qrydsc_descriptors.as_ref(),
        prefer_sqldard_descriptors,
    ) else {
        return Ok(());
    };
    if descs.is_empty() {
        return Ok(());
    }

    let pending_before = pending_row_bytes.len();
    let mut buffered = std::mem::take(pending_row_bytes);
    let decoded_rows = db2_proto::fdoca::decode_rows_with_tail(&[], descs, &mut buffered)
        .map_err(|e| Error::Protocol(e.to_string()))?;
    let decoded_count = decoded_rows.len();
    if let Some(row_width) = decoded_rows.first().map(|values| values.len()) {
        let col_names: Arc<[String]> = row_column_names(column_info, row_width).into();
        for values in decoded_rows {
            rows.push(Row::new_shared(col_names.clone(), values));
        }
    }
    *pending_row_bytes = buffered;
    if collect_diagnostics {
        diagnostics.push(format!(
            "pending_qrydta_decode pending_before={} rows_decoded={} rows_total={} pending_after={} descriptors={}",
            pending_before,
            decoded_count,
            rows.len(),
            pending_row_bytes.len(),
            descs.len()
        ));
        if decoded_count == 0 && !pending_row_bytes.is_empty() {
            diagnostics.push(format!(
                "pending_qrydta_decode progress={}",
                db2_proto::fdoca::describe_decode_progress(pending_row_bytes, descs)
            ));
            diagnostics.push(format!(
                "pending_qrydta_decode descriptors {}",
                descriptor_summary(descs)
            ));
        }
    }

    Ok(())
}

fn descriptor_summary(descriptors: &[db2_proto::fdoca::ColumnDescriptor]) -> String {
    let shown = descriptors
        .iter()
        .take(16)
        .map(|desc| {
            format!(
                "#{} drda=0x{:02X} type={:?} len={} nullable={} ccsid={} order={:?}",
                desc.column_index + 1,
                desc.drda_type,
                desc.db2_type,
                desc.length,
                desc.nullable,
                desc.ccsid,
                desc.byte_order
            )
        })
        .collect::<Vec<_>>()
        .join("; ");

    if descriptors.len() > 16 {
        format!("first16=[{}] total={}", shown, descriptors.len())
    } else {
        format!("all=[{}]", shown)
    }
}

fn preferred_descriptor_vec<'a>(
    sqldard_descriptors: Option<&'a Vec<db2_proto::fdoca::ColumnDescriptor>>,
    qrydsc_descriptors: Option<&'a Vec<db2_proto::fdoca::ColumnDescriptor>>,
    prefer_sqldard: bool,
) -> Option<&'a Vec<db2_proto::fdoca::ColumnDescriptor>> {
    if prefer_sqldard {
        sqldard_descriptors.or(qrydsc_descriptors)
    } else {
        qrydsc_descriptors.or(sqldard_descriptors)
    }
}

fn prepare_frames_have_result_metadata(frames: &[DssFrame]) -> bool {
    frames.iter().any(|frame| {
        ClientInner::parse_ddm_objects(&frame.payload)
            .ok()
            .is_some_and(|objects| {
                objects
                    .iter()
                    .any(|obj| obj.code_point == codepoints::SQLDARD)
            })
    })
}

fn frames_have_data_or_terminal_reply(frames: &[DssFrame]) -> bool {
    frames.iter().any(|frame| {
        ClientInner::parse_ddm_objects(&frame.payload)
            .ok()
            .is_some_and(|objects| {
                objects.iter().any(|obj| {
                    matches!(
                        obj.code_point,
                        codepoints::QRYDTA
                            | codepoints::ENDQRYRM
                            | codepoints::SQLCARD
                            | codepoints::SQLERRRM
                            | codepoints::SYNTAXRM
                            | codepoints::PRCCNVRM
                            | codepoints::CMDNSPRM
                            | codepoints::PRMNSPRM
                            | codepoints::VALNSPRM
                            | codepoints::DTAMCHRM
                            | codepoints::QRYNOPRM
                    )
                })
            })
    })
}

fn frames_have_query_data_or_query_end_reply(frames: &[DssFrame]) -> bool {
    frames.iter().any(|frame| {
        ClientInner::parse_ddm_objects(&frame.payload)
            .ok()
            .is_some_and(|objects| {
                objects.iter().any(|obj| {
                    matches!(
                        obj.code_point,
                        codepoints::QRYDTA
                            | codepoints::ENDQRYRM
                            | codepoints::SQLERRRM
                            | codepoints::SYNTAXRM
                            | codepoints::PRCCNVRM
                            | codepoints::CMDNSPRM
                            | codepoints::PRMNSPRM
                            | codepoints::VALNSPRM
                            | codepoints::DTAMCHRM
                            | codepoints::QRYNOPRM
                    )
                })
            })
    })
}

fn frames_have_query_data(frames: &[DssFrame]) -> bool {
    frames.iter().any(|frame| {
        ClientInner::parse_ddm_objects(&frame.payload)
            .ok()
            .is_some_and(|objects| {
                objects
                    .iter()
                    .any(|obj| obj.code_point == codepoints::QRYDTA)
            })
    })
}

fn query_instance_id_from_frames(frames: &[DssFrame]) -> Result<Option<Vec<u8>>, Error> {
    for frame in frames {
        for obj in ClientInner::parse_ddm_objects(&frame.payload)? {
            if obj.code_point == codepoints::OPNQRYRM {
                let reply = db2_proto::replies::opnqryrm::parse_opnqryrm(&obj)
                    .map_err(|err| Error::Protocol(err.to_string()))?;
                if reply.is_success() && reply.query_instance_id.is_some() {
                    return Ok(reply.query_instance_id);
                }
            }
        }
    }

    Ok(None)
}

fn apply_extdta_payloads_to_rows(
    rows: &mut [Row],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
    extdta_payloads: &[Vec<u8>],
) {
    if rows.is_empty() || descriptors.is_empty() || extdta_payloads.is_empty() {
        return;
    }

    let mut extdta_index = 0usize;
    for row in rows {
        for (column_index, value) in row.values_mut().iter_mut().enumerate() {
            let Some(descriptor) = descriptors.get(column_index) else {
                continue;
            };
            if !descriptor_uses_extdta(descriptor) || !value_needs_extdta(value) {
                continue;
            }
            let Some(payload) = extdta_payloads.get(extdta_index) else {
                return;
            };
            extdta_index += 1;
            let payload = extdta_value_payload(payload, descriptor.nullable);
            match descriptor.db2_type {
                db2_proto::types::Db2Type::Blob
                | db2_proto::types::Db2Type::BlobLocator
                | db2_proto::types::Db2Type::LobBytes(_) => {
                    *value = db2_proto::types::Db2Value::Blob(payload.to_vec());
                }
                db2_proto::types::Db2Type::Xml => {
                    *value =
                        db2_proto::types::Db2Value::Xml(decode_extdta_text(payload, descriptor));
                }
                db2_proto::types::Db2Type::Clob
                | db2_proto::types::Db2Type::DbClob
                | db2_proto::types::Db2Type::ClobLocator
                | db2_proto::types::Db2Type::DbClobLocator
                | db2_proto::types::Db2Type::LobChar(_)
                | db2_proto::types::Db2Type::VarChar(_)
                | db2_proto::types::Db2Type::VarGraphic(_) => {
                    *value =
                        db2_proto::types::Db2Value::Clob(decode_extdta_text(payload, descriptor));
                }
                _ => {}
            }
        }
    }
}

fn rows_need_extdta_payloads(
    rows: &[Row],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> bool {
    rows.iter().any(|row| {
        row.values()
            .iter()
            .enumerate()
            .any(|(column_index, value)| {
                descriptors.get(column_index).is_some_and(|descriptor| {
                    descriptor_uses_extdta(descriptor) && value_needs_extdta(value)
                })
            })
    })
}

fn is_lob_descriptor(descriptor: &db2_proto::fdoca::ColumnDescriptor) -> bool {
    matches!(
        descriptor.db2_type,
        db2_proto::types::Db2Type::Blob
            | db2_proto::types::Db2Type::Clob
            | db2_proto::types::Db2Type::DbClob
            | db2_proto::types::Db2Type::BlobLocator
            | db2_proto::types::Db2Type::ClobLocator
            | db2_proto::types::Db2Type::DbClobLocator
            | db2_proto::types::Db2Type::LobBytes(_)
            | db2_proto::types::Db2Type::LobChar(_)
            | db2_proto::types::Db2Type::Xml
    )
}

fn descriptor_uses_extdta(descriptor: &db2_proto::fdoca::ColumnDescriptor) -> bool {
    is_lob_descriptor(descriptor)
        || matches!(
            descriptor.db2_type,
            db2_proto::types::Db2Type::VarChar(len)
                | db2_proto::types::Db2Type::VarGraphic(len)
                | db2_proto::types::Db2Type::LobBytes(len)
                | db2_proto::types::Db2Type::LobChar(len)
                if len >= 32_768
        )
}

fn value_needs_extdta(value: &db2_proto::types::Db2Value) -> bool {
    match value {
        db2_proto::types::Db2Value::Clob(value) => value.starts_with("LOB locator 0x"),
        db2_proto::types::Db2Value::Xml(value) => value.starts_with("LOB locator 0x"),
        db2_proto::types::Db2Value::Blob(value) => value.len() == 4,
        _ => false,
    }
}

fn extdta_value_payload(payload: &[u8], nullable: bool) -> &[u8] {
    let payload = unwrap_extdta_payload(payload);
    if nullable && matches!(payload.first(), Some(0x00 | 0xFF)) {
        unwrap_extdta_payload(&payload[1..])
    } else {
        payload
    }
}

fn decode_extdta_text(payload: &[u8], descriptor: &db2_proto::fdoca::ColumnDescriptor) -> String {
    if matches!(
        descriptor.db2_type,
        db2_proto::types::Db2Type::DbClob | db2_proto::types::Db2Type::DbClobLocator
    ) || descriptor.ccsid == 1200
    {
        return decode_utf16be_lossy(payload);
    }

    String::from_utf8_lossy(payload).to_string()
}

fn decode_utf16be_lossy(payload: &[u8]) -> String {
    let units = payload
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect::<Vec<_>>();
    String::from_utf16_lossy(&units)
}

fn unwrap_extdta_payload(payload: &[u8]) -> &[u8] {
    if payload.len() >= 4 {
        let object_len = u16::from_be_bytes([payload[0], payload[1]]) as usize;
        let code_point = u16::from_be_bytes([payload[2], payload[3]]);
        if object_len == payload.len() && code_point == codepoints::FDODTA {
            return &payload[4..];
        }
    }

    if payload.len() >= 4 {
        let declared_len =
            u32::from_be_bytes([payload[0], payload[1], payload[2], payload[3]]) as usize;
        if declared_len == payload.len() - 4 {
            return &payload[4..];
        }
    }

    payload
}

fn frame_diagnostics(frames: &[DssFrame]) -> Vec<String> {
    let mut diagnostics = Vec::new();
    for (frame_index, frame) in frames.iter().enumerate() {
        match ClientInner::parse_ddm_objects(&frame.payload) {
            Ok(objects) => {
                if objects.is_empty() {
                    diagnostics.push(format!(
                        "frame#{frame_index} type={:?} corr={} payload_len={} objects=0",
                        frame.header.dss_type,
                        frame.header.correlation_id,
                        frame.payload.len()
                    ));
                    continue;
                }

                for obj in objects {
                    let preview = if matches!(
                        obj.code_point,
                        codepoints::SQLDARD
                            | codepoints::QRYDSC
                            | codepoints::QRYDTA
                            | codepoints::EXTDTA
                    ) {
                        format!(" preview={}", format_hex_preview(&obj.data, 96))
                    } else {
                        String::new()
                    };
                    diagnostics.push(format!(
                        "frame#{frame_index} type={:?} corr={} cp={} len={}{}",
                        frame.header.dss_type,
                        frame.header.correlation_id,
                        ddm_codepoint_name(obj.code_point),
                        obj.data.len(),
                        preview
                    ));
                    if obj.code_point == codepoints::SQLDARD {
                        diagnostics.extend(db2_proto::replies::sqldard::diagnose_column_names(
                            &obj.data,
                        ));
                    }
                }
            }
            Err(err) => diagnostics.push(format!(
                "frame#{frame_index} type={:?} corr={} payload_len={} parse_error={}",
                frame.header.dss_type,
                frame.header.correlation_id,
                frame.payload.len(),
                err
            )),
        }
    }
    diagnostics
}

fn ddm_codepoint_name(code_point: u16) -> String {
    let name = match code_point {
        codepoints::SQLCARD => "SQLCARD",
        codepoints::SQLDARD => "SQLDARD",
        codepoints::OPNQRYRM => "OPNQRYRM",
        codepoints::QRYDSC => "QRYDSC",
        codepoints::QRYDTA => "QRYDTA",
        codepoints::EXTDTA => "EXTDTA",
        codepoints::ENDQRYRM => "ENDQRYRM",
        codepoints::QRYNOPRM => "QRYNOPRM",
        codepoints::DTAMCHRM => "DTAMCHRM",
        codepoints::RDBUPDRM => "RDBUPDRM",
        codepoints::SQLERRRM => "SQLERRRM",
        codepoints::SYNTAXRM => "SYNTAXRM",
        codepoints::PRCCNVRM => "PRCCNVRM",
        codepoints::VALNSPRM => "VALNSPRM",
        codepoints::CMDNSPRM => "CMDNSPRM",
        codepoints::PRMNSPRM => "PRMNSPRM",
        _ => "UNKNOWN",
    };
    format!("{name}(0x{code_point:04X})")
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
        codepoints::CMDNSPRM => Some("CMDNSPRM"),
        codepoints::PRMNSPRM => Some("PRMNSPRM"),
        codepoints::VALNSPRM => Some("VALNSPRM"),
        codepoints::SQLERRRM => Some("SQLERRRM"),
        codepoints::CMDCHKRM => Some("CMDCHKRM"),
        codepoints::DTAMCHRM => Some("DTAMCHRM"),
        codepoints::QRYNOPRM => Some("QRYNOPRM"),
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

    (0..value_count).map(|i| format!("COL{}", i + 1)).collect()
}

fn rows_with_result_column_names(rows: Vec<Row>, columns: &[ColumnInfo]) -> Vec<Row> {
    if rows.is_empty() || columns.is_empty() {
        return rows;
    }

    let column_names: Arc<[String]> = columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>()
        .into();
    rows.into_iter()
        .map(|row| {
            if row.len() == columns.len() {
                Row::new_shared(column_names.clone(), row.into_values())
            } else {
                row
            }
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

fn column_info_with_descriptor_types(
    column_info: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Vec<ColumnInfo> {
    column_info
        .iter()
        .zip(descriptors.iter())
        .map(|(column, descriptor)| ColumnInfo {
            name: column.name.clone(),
            type_name: format!("{:?}", descriptor.db2_type),
            nullable: descriptor.nullable,
            precision: if descriptor.precision > 0 {
                Some(descriptor.precision as u16)
            } else {
                column.precision
            },
            scale: if descriptor.scale > 0 {
                Some(descriptor.scale as u16)
            } else {
                column.scale
            },
        })
        .collect()
}

fn column_info_for_cursor_fetch(
    column_info: &[ColumnInfo],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
) -> Vec<ColumnInfo> {
    if column_info.len() == descriptors.len() && !column_info.is_empty() {
        column_info_with_descriptor_types(column_info, descriptors)
    } else if column_info.is_empty() {
        column_info_from_descriptors(descriptors)
    } else {
        column_info.to_vec()
    }
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
pub(crate) fn sql_is_query(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("SELECT")
        || trimmed.starts_with("WITH")
        || trimmed.starts_with("VALUES")
        || trimmed.starts_with("CALL")
}

fn should_retry_query_after_session_error(sql: &str, params: &[&dyn ToSql], err: &Error) -> bool {
    if !params.is_empty() || !sql_is_retryable_read_query(sql) {
        return false;
    }

    match err {
        Error::Connection(message) | Error::Protocol(message) => {
            message_indicates_retryable_session_state(message)
        }
        Error::Sql { .. } => error_indicates_stale_session_state(err),
        Error::Io(_) | Error::Tls(_) => true,
        _ => false,
    }
}

fn error_indicates_stale_session_state(err: &Error) -> bool {
    matches!(
        err,
        Error::Sql {
            sqlcode: -502 | -514 | -518,
            ..
        }
    )
}

fn message_indicates_retryable_session_state(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    message.contains("closed by server")
        || message.contains("qrynoprm")
        || message.contains("sqlcode=-502")
        || message.contains("sqlcode=-514")
        || message.contains("sqlcode=-518")
}

fn sql_is_retryable_read_query(sql: &str) -> bool {
    let trimmed = sql.trim().to_uppercase();
    trimmed.starts_with("SELECT") || trimmed.starts_with("WITH") || trimmed.starts_with("VALUES")
}

fn can_retry_zos_lob_query_from_catalog(
    sql: &str,
    params: &[&dyn ToSql],
    zos_lob_internal_depth: usize,
    current_schema: Option<&str>,
    server_info: Option<&ServerInfo>,
) -> bool {
    params.is_empty()
        && zos_lob_internal_depth == 0
        && server_info.is_some_and(is_db2_zos_server)
        && !use_zos_native_lob_only()
        && sql_is_retryable_read_query(sql)
        && build_zos_select_star_metadata_query(sql, current_schema).is_some()
}

fn wrap_zos_lob_stage_error(stage: &str, sql: &str, err: Error) -> Error {
    Error::Protocol(format!(
        "z/OS LOB {stage} failed: {}; sql={}",
        err,
        summarize_sql_for_diagnostics(sql)
    ))
}

fn finish_query_diagnostics(
    sql: &str,
    param_count: usize,
    started: Option<Instant>,
    result: Result<QueryResult, Error>,
) -> Result<QueryResult, Error> {
    let mut result = result?;

    if let Some(started) = started {
        result.diagnostics.push(format!(
            "driver_query_total_ms={:.3} rows={} columns={} params={} sql={}",
            started.elapsed().as_secs_f64() * 1000.0,
            result.row_count,
            result.columns.len(),
            param_count,
            summarize_sql_for_diagnostics(sql)
        ));
    }

    emit_query_diagnostics(sql, &result.diagnostics);
    Ok(result)
}

fn emit_query_diagnostics(sql: &str, diagnostics: &[String]) {
    if diagnostics.is_empty() || !query_diagnostics_stderr_enabled() {
        return;
    }

    eprintln!(
        "[db2-diagnostics] sql={} entries={}",
        summarize_sql_for_diagnostics(sql),
        diagnostics.len()
    );
    for line in diagnostics {
        eprintln!("[db2-diagnostics] {line}");
    }
}

fn query_diagnostics_stderr_enabled() -> bool {
    env::var("DB2_QUERY_DIAGNOSTICS")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(false)
        || env::var("DB2_QUERY_DIAGNOSTICS_STDERR")
            .map(|value| {
                let value = value.trim().to_ascii_lowercase();
                !(value == "0" || value == "false" || value == "off" || value == "no")
            })
            .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use db2_proto::types::Db2Value;

    #[test]
    fn build_zos_select_star_metadata_query_uses_zos_catalog() {
        let query = build_zos_select_star_metadata_query(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY",
            None,
        )
        .unwrap();

        assert_eq!(
            query,
            "SELECT NAME, COLTYPE FROM SYSIBM.SYSCOLUMNS WHERE TBCREATOR = 'SCM_M6T2' AND TBNAME = 'TBL_H8Q4' ORDER BY COLNO"
        );
    }

    #[test]
    fn build_zos_select_metadata_query_supports_explicit_projection_with_in_list() {
        let query = build_zos_select_star_metadata_query(
            "SELECT INSP_RPT_ID, INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT WHERE INSP_RPT_ID IN (1,2,3)",
            None,
        )
        .unwrap();

        assert_eq!(
            query,
            "SELECT NAME, COLTYPE FROM SYSIBM.SYSCOLUMNS WHERE TBCREATOR = 'FIREINSP' AND TBNAME = 'INSP_RPT' ORDER BY COLNO"
        );
    }

    #[test]
    fn build_zos_select_star_metadata_query_does_not_recurse_on_catalog_query() {
        assert!(build_zos_select_star_metadata_query(
            "SELECT NAME, COLTYPE FROM SYSIBM.SYSCOLUMNS WHERE TBCREATOR = 'SCM_M6T2'",
            None,
        )
        .is_none());
    }

    #[test]
    fn parse_simple_select_star_supports_current_schema_and_quoted_names() {
        let parsed = parse_simple_select_star(
            " select * from \"ScmM6t2\".\"TBL_H8Q4\" fetch first 1 row only ",
            Some("IGNORED"),
        )
        .unwrap();

        assert_eq!(parsed.schema, "ScmM6t2");
        assert_eq!(parsed.table, "TBL_H8Q4");
        assert_eq!(parsed.suffix, "fetch first 1 row only");

        let parsed = parse_simple_select_star("SELECT * FROM TBL_H8Q4", Some("SCM_M6T2")).unwrap();
        assert_eq!(parsed.schema, "SCM_M6T2");
        assert_eq!(parsed.table, "TBL_H8Q4");
    }

    #[test]
    fn select_projection_aliases_extract_explicit_aliases() {
        let aliases = select_projection_aliases(
            r#"
            WITH src AS (SELECT COUNT(*) AS INNER_TOTAL FROM TBL_Q7R2)
            SELECT COUNT(*) AS total_count,
                   CAST(1 AS INTEGER) AS "One Value",
                   NAME
            FROM src
            "#,
        )
        .unwrap();

        assert_eq!(
            aliases,
            vec![
                Some("TOTAL_COUNT".to_string()),
                Some("One Value".to_string()),
                Some("NAME".to_string()),
            ]
        );
    }

    #[test]
    fn select_projection_aliases_extract_qualified_column_names() {
        let aliases = select_projection_aliases(
            r#"
            SELECT r.INSP_RPT_ID,
                   r.INSP_RPT_DETL_DOC
            FROM FIREINSP.INSP_RPT r
            JOIN FIREINSP.INSP_CTL c ON r.INSP_RPT_ID = c.INSP_RPT_ID
            ORDER BY r.INSP_RPT_ID
            "#,
        )
        .unwrap();

        assert_eq!(
            aliases,
            vec![
                Some("INSP_RPT_ID".to_string()),
                Some("INSP_RPT_DETL_DOC".to_string()),
            ]
        );
    }

    #[test]
    fn column_info_with_select_aliases_replaces_generated_names() {
        let columns = vec![
            ColumnInfo::new("COL1".to_string(), "BigInt".to_string(), false),
            ColumnInfo::new("NAME".to_string(), "VarChar(20)".to_string(), true),
        ];

        let columns = column_info_with_select_aliases(
            "SELECT COUNT(*) AS total_count, NAME FROM TBL_Q7R2",
            columns,
        );

        assert_eq!(columns[0].name, "TOTAL_COUNT");
        assert_eq!(columns[1].name, "NAME");
    }

    #[test]
    fn column_info_with_select_aliases_replaces_qualified_column_projection_names() {
        let columns = vec![
            ColumnInfo::new("COL1".to_string(), "Decimal".to_string(), false),
            ColumnInfo::new("COL2".to_string(), "Clob".to_string(), true),
        ];

        let columns = column_info_with_select_aliases(
            "SELECT r.INSP_RPT_ID, r.INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT r",
            columns,
        );

        assert_eq!(columns[0].name, "INSP_RPT_ID");
        assert_eq!(columns[1].name, "INSP_RPT_DETL_DOC");
    }

    #[test]
    fn column_info_with_select_aliases_replaces_expression_aliases() {
        let columns = vec![
            ColumnInfo::new("COL1".to_string(), "Integer".to_string(), false),
            ColumnInfo::new("COL2".to_string(), "Integer".to_string(), false),
            ColumnInfo::new("COL3".to_string(), "BigInt".to_string(), false),
        ];

        let columns = column_info_with_select_aliases(
            r#"
            SELECT YEAR(c.INSP_CMPLT_DT) AS YR,
                   MONTH(c.INSP_CMPLT_DT) AS MO,
                   COUNT(*) AS CNT
            FROM FIREINSP.INSP_RPT r
            JOIN FIREINSP.INSP_CTL c ON r.INSP_RPT_ID = c.INSP_RPT_ID
            GROUP BY YEAR(c.INSP_CMPLT_DT), MONTH(c.INSP_CMPLT_DT)
            ORDER BY YR, MO
            "#,
            columns,
        );

        assert_eq!(columns[0].name, "YR");
        assert_eq!(columns[1].name, "MO");
        assert_eq!(columns[2].name, "CNT");
    }

    #[test]
    fn rows_with_result_column_names_updates_row_lookup_names() {
        let rows = vec![Row::new(
            vec!["COL1".to_string(), "COL2".to_string(), "COL3".to_string()],
            vec![
                Db2Value::Integer(2026),
                Db2Value::Integer(3),
                Db2Value::Integer(76238),
            ],
        )];
        let columns = vec![
            ColumnInfo::new("YR".to_string(), "Integer".to_string(), false),
            ColumnInfo::new("MO".to_string(), "Integer".to_string(), false),
            ColumnInfo::new("CNT".to_string(), "Integer".to_string(), false),
        ];

        let rows = rows_with_result_column_names(rows, &columns);

        assert_eq!(rows[0].get::<i32>("YR"), Some(2026));
        assert_eq!(rows[0].get::<i32>("MO"), Some(3));
        assert_eq!(rows[0].get::<i32>("CNT"), Some(76238));
        assert_eq!(rows[0].get::<i32>("COL1"), None);
    }

    #[test]
    fn sql_has_like_predicate_ignores_literals_comments_and_identifiers() {
        assert!(sql_has_like_predicate(
            "SELECT COUNT(*) FROM T WHERE DOC NOT LIKE '%<DueDate>20%'"
        ));
        assert!(sql_has_like_predicate(
            "SELECT * FROM T WHERE (DOC LIKE '%abc%')"
        ));
        assert!(!sql_has_like_predicate("SELECT 'LIKE' AS TEXT FROM T"));
        assert!(!sql_has_like_predicate("SELECT LIKELY_COLUMN FROM T"));
        assert!(!sql_has_like_predicate(
            "SELECT * FROM T -- WHERE DOC LIKE '%x%'\nWHERE ID = 1"
        ));
    }

    #[test]
    fn parse_fetch_first_row_limit_works_for_full_select() {
        assert_eq!(
            parse_fetch_first_row_limit("SELECT * FROM SCM_M6T2.TBL_Q7R2 FETCH FIRST 3 ROWS ONLY"),
            Some(3)
        );
        assert_eq!(parse_fetch_first_row_limit("SELECT * FROM T"), None);
    }

    #[test]
    fn optimize_zos_select_sql_adds_read_only_and_optimize_for_fetch_first() {
        assert_eq!(
            optimize_zos_select_sql(
                "SELECT * FROM SCM_M6T2.TBL_Q7R2 FETCH FIRST 3 ROWS ONLY"
            )
            .as_deref(),
            Some(
                "SELECT * FROM SCM_M6T2.TBL_Q7R2 FETCH FIRST 3 ROWS ONLY FOR FETCH ONLY OPTIMIZE FOR 3 ROWS"
            )
        );
    }

    #[test]
    fn optimize_zos_select_sql_preserves_isolation_clause() {
        assert_eq!(
            optimize_zos_select_sql("SELECT * FROM T FETCH FIRST 2 ROWS ONLY WITH UR").as_deref(),
            Some("SELECT * FROM T FETCH FIRST 2 ROWS ONLY FOR FETCH ONLY OPTIMIZE FOR 2 ROWS WITH UR")
        );
    }

    #[test]
    fn optimize_zos_select_sql_skips_existing_cursor_clauses() {
        assert!(optimize_zos_select_sql("SELECT * FROM T FOR READ ONLY").is_none());
        assert!(optimize_zos_select_sql("SELECT * FROM T OPTIMIZE FOR 1 ROW").is_none());
    }

    #[test]
    fn normalize_zos_non_lob_qryblksz_uses_odbc_query_data_sizes() {
        assert_eq!(normalize_zos_non_lob_qryblksz(1), 32_767);
        assert_eq!(normalize_zos_non_lob_qryblksz(65_000), 65_535);
        assert_eq!(normalize_zos_non_lob_qryblksz(262_144), 262_143);
        assert_eq!(normalize_zos_non_lob_qryblksz(2_000_000), 1_048_575);
    }

    #[test]
    fn build_zos_select_star_lob_base_query_uses_catalog_metadata() {
        let metadata = QueryResult::with_rows(
            vec![
                Row::new(
                    vec!["NAME".to_string(), "COLTYPE".to_string()],
                    vec![
                        Db2Value::VarChar("COL_E2K9_ID".to_string()),
                        Db2Value::Char("DECIMAL ".to_string()),
                    ],
                ),
                Row::new(
                    vec!["NAME".to_string(), "COLTYPE".to_string()],
                    vec![
                        Db2Value::VarChar("COL_F6N3_DOC".to_string()),
                        Db2Value::Char("CLOB    ".to_string()),
                    ],
                ),
                Row::new(
                    vec!["NAME".to_string(), "COLTYPE".to_string()],
                    vec![
                        Db2Value::VarChar("DB2_GENERATED_ROWID_FOR_LOBS".to_string()),
                        Db2Value::Char("ROWID   ".to_string()),
                    ],
                ),
            ],
            Vec::new(),
        );

        let rewritten = build_zos_select_star_lob_base_query(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY",
            None,
            &metadata,
        )
        .unwrap();

        assert!(rewritten.contains("\"COL_E2K9_ID\""));
        assert!(rewritten
            .contains("CAST(LENGTH(\"COL_F6N3_DOC\") AS VARCHAR(32)) AS \"DB2NODE_LOB_LEN_2\""));
        assert!(!rewritten.contains("DB2_GENERATED_ROWID_FOR_LOBS"));
        assert!(rewritten.ends_with("FETCH FIRST 3 ROWS ONLY"));
    }

    #[test]
    fn build_zos_lob_base_query_supports_simple_projection() {
        let metadata = QueryResult::with_rows(
            vec![
                Row::new(
                    vec!["COL1".to_string(), "COL2".to_string()],
                    vec![
                        Db2Value::VarChar("COL_E2K9_ID".to_string()),
                        Db2Value::Char("DECIMAL ".to_string()),
                    ],
                ),
                Row::new(
                    vec!["COL1".to_string(), "COL2".to_string()],
                    vec![
                        Db2Value::VarChar("COL_F6N3_DOC".to_string()),
                        Db2Value::Char("CLOB(1M)".to_string()),
                    ],
                ),
                Row::new(
                    vec!["COL1".to_string(), "COL2".to_string()],
                    vec![
                        Db2Value::VarChar("DB2_GENERATED_ROWID_FOR_LOBS".to_string()),
                        Db2Value::Char("ROWID   ".to_string()),
                    ],
                ),
            ],
            Vec::new(),
        );

        let rewritten = build_zos_select_star_lob_base_query(
            "SELECT COL_F6N3_DOC, DB2_GENERATED_ROWID_FOR_LOBS FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 1 ROW ONLY",
            None,
            &metadata,
        )
        .unwrap();

        assert!(rewritten
            .contains("CAST(LENGTH(\"COL_F6N3_DOC\") AS VARCHAR(32)) AS \"DB2NODE_LOB_LEN_1\""));
        assert!(rewritten
            .contains("HEX(\"DB2_GENERATED_ROWID_FOR_LOBS\") AS \"DB2_GENERATED_ROWID_FOR_LOBS\""));
        assert!(!rewritten.contains("\"COL_E2K9_ID\""));
    }

    #[test]
    fn catalog_columns_from_query_result_falls_back_to_positions() {
        let metadata = QueryResult::with_rows(
            vec![Row::new(
                vec!["COL1".to_string(), "COL2".to_string()],
                vec![
                    Db2Value::VarChar("COL_F6N3_DOC".to_string()),
                    Db2Value::Char("CLOB    ".to_string()),
                ],
            )],
            Vec::new(),
        );

        assert_eq!(
            catalog_columns_from_query_result(&metadata),
            vec![CatalogColumn {
                name: "COL_F6N3_DOC".to_string(),
                coltype: "CLOB".to_string()
            }]
        );
    }

    #[test]
    fn catalog_columns_from_query_result_moves_generated_rowid_last() {
        let metadata = QueryResult::with_rows(
            vec![
                Row::new(
                    vec!["NAME".to_string(), "COLTYPE".to_string()],
                    vec![
                        Db2Value::VarChar("DB2_GENERATED_ROWID_FOR_LOBS".to_string()),
                        Db2Value::Char("ROWID   ".to_string()),
                    ],
                ),
                Row::new(
                    vec!["NAME".to_string(), "COLTYPE".to_string()],
                    vec![
                        Db2Value::VarChar("COL_E2K9_ID".to_string()),
                        Db2Value::Char("DECIMAL ".to_string()),
                    ],
                ),
                Row::new(
                    vec!["NAME".to_string(), "COLTYPE".to_string()],
                    vec![
                        Db2Value::VarChar("COL_F6N3_DOC".to_string()),
                        Db2Value::Char("CLOB    ".to_string()),
                    ],
                ),
            ],
            Vec::new(),
        );

        let columns = catalog_columns_from_query_result(&metadata);
        assert_eq!(
            columns
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            vec![
                "COL_E2K9_ID",
                "COL_F6N3_DOC",
                "DB2_GENERATED_ROWID_FOR_LOBS"
            ]
        );
    }

    #[test]
    fn selected_catalog_columns_omits_generated_rowid_for_select_star() {
        let parsed = parse_simple_select_star(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 1 ROW ONLY",
            None,
        )
        .unwrap();
        let columns = vec![
            CatalogColumn {
                name: "COL_E2K9_ID".to_string(),
                coltype: "DECIMAL".to_string(),
            },
            CatalogColumn {
                name: "COL_F6N3_DOC".to_string(),
                coltype: "CLOB".to_string(),
            },
            CatalogColumn {
                name: "DB2_GENERATED_ROWID_FOR_LOBS".to_string(),
                coltype: "ROWID".to_string(),
            },
        ];

        let selected = selected_catalog_columns(&parsed, &columns).unwrap();

        assert_eq!(
            selected
                .iter()
                .map(|column| column.name.as_str())
                .collect::<Vec<_>>(),
            vec!["COL_E2K9_ID", "COL_F6N3_DOC"]
        );
    }

    #[test]
    fn build_zos_lob_per_column_queries_materialize_values() {
        let parsed = parse_simple_select_star(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY",
            None,
        )
        .unwrap();
        let decimal_column = CatalogColumn {
            name: "COL_E2K9_ID".to_string(),
            coltype: "DECIMAL".to_string(),
        };
        let clob_column = CatalogColumn {
            name: "COL_F6N3_DOC".to_string(),
            coltype: "CLOB".to_string(),
        };
        let rowid_column = CatalogColumn {
            name: "DB2_GENERATED_ROWID_FOR_LOBS".to_string(),
            coltype: "ROWID".to_string(),
        };

        assert_eq!(
            build_zos_lob_row_probe_query(&parsed),
            "SELECT 1 AS \"DB2NODE_ROW_EXISTS\" FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY"
        );
        assert_eq!(
            build_zos_lob_length_query(&parsed, &clob_column, 1),
            "SELECT CAST(LENGTH(\"COL_F6N3_DOC\") AS VARCHAR(32)) AS \"DB2NODE_LOB_LEN\" FROM (SELECT \"COL_F6N3_DOC\", ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" = 1"
        );
        assert_eq!(
            build_zos_scalar_value_query(&parsed, &decimal_column, 2),
            "SELECT CAST(\"COL_E2K9_ID\" AS VARCHAR(128)) AS \"COL_E2K9_ID\" FROM (SELECT \"COL_E2K9_ID\", ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" = 2"
        );
        assert_eq!(
            build_zos_scalar_value_query(&parsed, &rowid_column, 1),
            "SELECT HEX(\"DB2_GENERATED_ROWID_FOR_LOBS\") AS \"DB2_GENERATED_ROWID_FOR_LOBS\" FROM (SELECT \"DB2_GENERATED_ROWID_FOR_LOBS\", ROW_NUMBER() OVER() AS \"DB2NODE_RN\" FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY) AS DB2NODE_LOB_SRC WHERE \"DB2NODE_RN\" = 1"
        );
    }

    #[test]
    fn build_zos_lob_chunk_set_query_fetches_one_piece_for_all_rows() {
        let parsed = parse_simple_select_star(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY",
            None,
        )
        .unwrap();
        let column = CatalogColumn {
            name: "COL_F6N3_DOC".to_string(),
            coltype: "CLOB".to_string(),
        };

        let sql = build_zos_lob_chunk_set_query(&parsed, &column, 16001, 16000, 3, 6);

        assert!(sql.starts_with("SELECT \"DB2NODE_RN\", "));
        assert!(sql.contains("CAST(SUBSTR(\"COL_F6N3_DOC\", 16001, 16000) AS VARCHAR(16000))"));
        assert!(sql.contains("ROW_NUMBER() OVER() AS \"DB2NODE_RN\""));
        assert!(sql.contains("WHERE \"DB2NODE_RN\" BETWEEN 3 AND 6"));
        assert!(!sql.contains("OFFSET"));
    }

    #[test]
    fn build_zos_lob_chunk_grid_query_fetches_multiple_pieces_for_row_batch() {
        let parsed = parse_simple_select_star(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 10 ROWS ONLY",
            None,
        )
        .unwrap();
        let column = CatalogColumn {
            name: "COL_F6N3_DOC".to_string(),
            coltype: "CLOB".to_string(),
        };

        let sql = build_zos_lob_chunk_grid_query(
            &parsed,
            &column,
            &[(1, 16_000), (16_001, 16_000), (32_001, 16_000)],
            1,
            3,
        );

        assert!(sql.starts_with("SELECT \"DB2NODE_RN\", "));
        assert!(sql.contains(
            "CASE WHEN LENGTH(\"COL_F6N3_DOC\") >= 1 THEN CAST(SUBSTR(\"COL_F6N3_DOC\", 1, 16000) AS VARCHAR(16000)) ELSE CAST(NULL AS VARCHAR(16000)) END AS \"DB2NODE_LOB_CHUNK_1\""
        ));
        assert!(sql.contains(
            "CASE WHEN LENGTH(\"COL_F6N3_DOC\") >= 16001 THEN CAST(SUBSTR(\"COL_F6N3_DOC\", 16001, 16000) AS VARCHAR(16000)) ELSE CAST(NULL AS VARCHAR(16000)) END AS \"DB2NODE_LOB_CHUNK_2\""
        ));
        assert!(sql.contains(
            "CASE WHEN LENGTH(\"COL_F6N3_DOC\") >= 32001 THEN CAST(SUBSTR(\"COL_F6N3_DOC\", 32001, 16000) AS VARCHAR(16000)) ELSE CAST(NULL AS VARCHAR(16000)) END AS \"DB2NODE_LOB_CHUNK_3\""
        ));
        assert!(sql.contains("WHERE \"DB2NODE_RN\" BETWEEN 1 AND 3"));
        assert!(!sql.contains("OFFSET"));
    }

    #[test]
    fn build_zos_lob_combined_chunk_grid_query_fetches_multiple_lob_columns() {
        let parsed = parse_simple_select_star(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 10 ROWS ONLY",
            None,
        )
        .unwrap();
        let columns = vec![
            CatalogColumn {
                name: "COL_E2K9_ID".to_string(),
                coltype: "DECIMAL".to_string(),
            },
            CatalogColumn {
                name: "COL_F6N3_DOC".to_string(),
                coltype: "CLOB".to_string(),
            },
            CatalogColumn {
                name: "COL_G1R7_DOC".to_string(),
                coltype: "CLOB".to_string(),
            },
        ];
        let specs = vec![
            LobChunkSpec {
                column_index: 1,
                chunk_number: 1,
                start: 1,
                len: 16_000,
            },
            LobChunkSpec {
                column_index: 2,
                chunk_number: 1,
                start: 1,
                len: 16_000,
            },
        ];

        let sql = build_zos_lob_combined_chunk_grid_query(&parsed, &columns, &specs, 1, 10);

        assert!(sql.starts_with("SELECT \"DB2NODE_RN\", "));
        assert!(sql.contains(
            "CASE WHEN LENGTH(\"COL_F6N3_DOC\") >= 1 THEN CAST(SUBSTR(\"COL_F6N3_DOC\", 1, 16000) AS VARCHAR(16000)) ELSE CAST(NULL AS VARCHAR(16000)) END AS \"DB2NODE_LOB_C2_K1\""
        ));
        assert!(sql.contains(
            "CASE WHEN LENGTH(\"COL_G1R7_DOC\") >= 1 THEN CAST(SUBSTR(\"COL_G1R7_DOC\", 1, 16000) AS VARCHAR(16000)) ELSE CAST(NULL AS VARCHAR(16000)) END AS \"DB2NODE_LOB_C3_K1\""
        ));
        assert!(sql.contains(
            "SELECT \"COL_F6N3_DOC\", \"COL_G1R7_DOC\", ROW_NUMBER() OVER() AS \"DB2NODE_RN\""
        ));
        assert!(sql.contains("WHERE \"DB2NODE_RN\" BETWEEN 1 AND 10"));
        assert!(!sql.contains("OFFSET"));
    }

    #[test]
    fn build_zos_lob_initial_combined_grid_query_carries_lengths_scalars_and_chunks() {
        let parsed = parse_simple_select_star(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 10 ROWS ONLY",
            None,
        )
        .unwrap();
        let columns = vec![
            CatalogColumn {
                name: "COL_E2K9_ID".to_string(),
                coltype: "DECIMAL".to_string(),
            },
            CatalogColumn {
                name: "COL_F6N3_DOC".to_string(),
                coltype: "CLOB".to_string(),
            },
        ];
        let specs = vec![LobChunkSpec {
            column_index: 1,
            chunk_number: 1,
            start: 1,
            len: 16_000,
        }];

        let sql = build_zos_lob_initial_combined_grid_query(&parsed, &columns, &specs);

        assert!(sql.starts_with("SELECT ROW_NUMBER() OVER() AS \"DB2NODE_RN\", "));
        assert!(sql.contains("CAST(\"COL_E2K9_ID\" AS VARCHAR(128)) AS \"COL_E2K9_ID\""));
        assert!(
            sql.contains("CAST(LENGTH(\"COL_F6N3_DOC\") AS VARCHAR(32)) AS \"DB2NODE_LOB_LEN_2\"")
        );
        assert!(sql.contains(
            "CASE WHEN LENGTH(\"COL_F6N3_DOC\") >= 1 THEN CAST(SUBSTR(\"COL_F6N3_DOC\", 1, 16000) AS VARCHAR(16000)) ELSE CAST(NULL AS VARCHAR(16000)) END AS \"DB2NODE_LOB_C2_K1\""
        ));
        assert!(sql.contains("FETCH FIRST 10 ROWS ONLY"));
        assert!(!sql.contains("OFFSET"));
    }

    #[test]
    fn zos_lob_rows_per_batch_caps_estimated_reply_bytes() {
        let clob_column = CatalogColumn {
            name: "COL_F6N3_DOC".to_string(),
            coltype: "CLOB".to_string(),
        };
        let dbclob_column = CatalogColumn {
            name: "DOC_TEXT".to_string(),
            coltype: "DBCLOB".to_string(),
        };

        assert_eq!(zos_lob_rows_per_batch(&clob_column, 16_000), 250);
        assert_eq!(zos_lob_rows_per_batch(&dbclob_column, 8_000), 250);
        assert_eq!(zos_lob_rows_per_batch(&clob_column, 64_000), 62);
    }

    #[test]
    fn column_info_lob_hint_ignores_internal_lob_aliases() {
        assert!(!column_info_has_lob_hint(&[ColumnInfo::new(
            "DB2NODE_LOB_CHUNK_1".to_string(),
            "VarGraphic(16000)".to_string(),
            false,
        )]));
        assert!(column_info_has_lob_hint(&[ColumnInfo::new(
            "COL_F6N3_DOC".to_string(),
            "CLOB".to_string(),
            true,
        )]));
    }

    #[test]
    fn zos_select_metadata_cache_stores_descriptorless_non_lob_columns() {
        let key = format!("unit:descriptorless-non-lob:{}", line!());
        let columns = vec![
            ColumnInfo::with_precision(
                "COL_A3F9_ID".to_string(),
                "Decimal { precision: 11, scale: 0 }".to_string(),
                false,
                11,
                0,
            ),
            ColumnInfo::new(
                "COL_J9V2_TXT".to_string(),
                "VARCHAR(4000)".to_string(),
                true,
            ),
        ];

        assert!(store_zos_select_metadata(&key, &columns, &[]));

        let cached = lookup_zos_select_metadata(&key).expect("descriptorless metadata cached");
        assert_eq!(cached.column_info.len(), 2);
        assert_eq!(cached.column_info[1].name, "COL_J9V2_TXT");
        assert_eq!(cached.column_info[1].type_name, "VARCHAR(4000)");
        assert!(cached.result_descriptors.is_empty());
    }

    #[test]
    fn zos_select_metadata_cache_skips_descriptorless_lob_hints() {
        let key = format!("unit:descriptorless-lob:{}", line!());
        let columns = vec![ColumnInfo::new(
            "COL_F6N3_DOC".to_string(),
            "VarChar(32777)".to_string(),
            true,
        )];

        assert!(!store_zos_select_metadata(&key, &columns, &[]));
        assert!(lookup_zos_select_metadata(&key).is_none());
    }

    #[test]
    fn zos_select_metadata_cache_allows_descriptorless_generic_text() {
        let key = format!("unit:descriptorless-generic-text:{}", line!());
        let columns = vec![ColumnInfo::new(
            "COL_F6N3_DOC".to_string(),
            "VARCHAR".to_string(),
            true,
        )];

        assert!(store_zos_select_metadata(&key, &columns, &[]));
        assert!(lookup_zos_select_metadata(&key).is_some());
    }

    #[test]
    fn zos_select_section_cache_allows_descriptorless_non_lob_columns() {
        let columns = vec![
            ColumnInfo::with_precision(
                "COL_A3F9_ID".to_string(),
                "Decimal { precision: 11, scale: 0 }".to_string(),
                false,
                11,
                0,
            ),
            ColumnInfo::new(
                "COL_J9V2_TXT".to_string(),
                "VARCHAR(4000)".to_string(),
                true,
            ),
        ];

        assert!(zos_select_section_cacheable(&columns, &[]));
    }

    #[test]
    fn zos_select_section_cache_skips_descriptorless_lob_hints() {
        let columns = vec![
            ColumnInfo::new(
                "COL_E2K9_ID".to_string(),
                "Decimal { precision: 11, scale: 0 }".to_string(),
                false,
            ),
            ColumnInfo::new(
                "COL_F6N3_DOC".to_string(),
                "VarChar(32777)".to_string(),
                true,
            ),
        ];

        assert!(!zos_select_section_cacheable(&columns, &[]));
    }

    #[test]
    fn zos_select_section_cache_allows_descriptorless_generic_text() {
        let prepare_columns = vec![ColumnInfo::new(
            "COL_F6N3_DOC".to_string(),
            "VARCHAR".to_string(),
            true,
        )];

        assert!(zos_select_section_cacheable(&prepare_columns, &[]));
    }

    #[test]
    fn result_columns_lob_route_detects_opened_native_lob_metadata() {
        let opened_columns = vec![ColumnInfo::new(
            "COL_F6N3_DOC".to_string(),
            "VarChar(32777)".to_string(),
            true,
        )];

        assert!(result_columns_need_zos_lob_route(&opened_columns));
    }

    #[test]
    fn result_lob_materialization_detects_extdta_clob_values() {
        let result = QueryResult::with_rows(
            vec![Row::new(
                vec!["COL_F6N3_DOC".to_string()],
                vec![Db2Value::Clob("materialized clob".to_string())],
            )],
            vec![ColumnInfo::new(
                "COL_F6N3_DOC".to_string(),
                "VARCHAR".to_string(),
                true,
            )],
        );

        assert!(result_has_zos_lob_materialization(&result));
    }

    #[test]
    fn zos_select_metadata_cache_can_evict_lobs_discovered_after_open() {
        let key = format!("unit:opened-lob-evict:{}", line!());
        let prepare_columns = vec![ColumnInfo::new(
            "COL_F6N3_DOC".to_string(),
            "VARCHAR(4000)".to_string(),
            true,
        )];

        assert!(store_zos_select_metadata(&key, &prepare_columns, &[]));
        assert!(lookup_zos_select_metadata(&key).is_some());
        assert!(remove_zos_select_metadata(&key));
        assert!(lookup_zos_select_metadata(&key).is_none());
    }

    #[test]
    fn zos_select_lob_cache_denylist_marks_sql_after_open_lob_discovery() {
        let key = format!("unit:lob-denylist:{}", line!());

        assert!(!zos_select_lob_cache_denied(&key));
        assert!(mark_zos_select_lob_cache_denied(&key));
        assert!(zos_select_lob_cache_denied(&key));
    }

    #[test]
    fn append_zos_lob_chunk_rows_trims_padded_final_chunk() {
        let chunk_result = QueryResult::with_rows(
            vec![Row::new(
                vec!["DB2NODE_RN".to_string(), "DB2NODE_LOB_CHUNK".to_string()],
                vec![
                    Db2Value::Integer(1),
                    Db2Value::VarChar("def     ".to_string()),
                ],
            )],
            Vec::new(),
        );
        let mut output_values = vec![vec![Db2Value::Clob("abc".to_string())]];

        append_zos_lob_chunk_rows(&chunk_result, 0, &[Some(6)], &mut output_values, 4).unwrap();

        assert_eq!(output_values[0][0], Db2Value::Clob("abcdef".to_string()));
    }

    #[test]
    fn append_zos_lob_chunk_grid_rows_appends_multiple_chunks() {
        let chunk_result = QueryResult::with_rows(
            vec![Row::new(
                vec![
                    "DB2NODE_RN".to_string(),
                    "DB2NODE_LOB_CHUNK_1".to_string(),
                    "DB2NODE_LOB_CHUNK_2".to_string(),
                ],
                vec![
                    Db2Value::Integer(1),
                    Db2Value::VarChar("abc".to_string()),
                    Db2Value::VarChar("def     ".to_string()),
                ],
            )],
            Vec::new(),
        );
        let mut output_values = vec![vec![Db2Value::Clob(String::new())]];

        append_zos_lob_chunk_grid_rows(
            &chunk_result,
            0,
            &[Some(6)],
            &mut output_values,
            &[(1, 3), (4, 3)],
        )
        .unwrap();

        assert_eq!(output_values[0][0], Db2Value::Clob("abcdef".to_string()));
    }

    #[test]
    fn append_zos_lob_combined_chunk_grid_rows_appends_multiple_columns() {
        let chunk_result = QueryResult::with_rows(
            vec![Row::new(
                vec![
                    "DB2NODE_RN".to_string(),
                    "DB2NODE_LOB_C2_K1".to_string(),
                    "DB2NODE_LOB_C3_K1".to_string(),
                ],
                vec![
                    Db2Value::Integer(1),
                    Db2Value::VarChar("abc".to_string()),
                    Db2Value::VarChar("xyz".to_string()),
                ],
            )],
            Vec::new(),
        );
        let specs = vec![
            LobChunkSpec {
                column_index: 1,
                chunk_number: 1,
                start: 1,
                len: 3,
            },
            LobChunkSpec {
                column_index: 2,
                chunk_number: 1,
                start: 1,
                len: 3,
            },
        ];
        let lob_lengths_by_column = vec![vec![None], vec![Some(3)], vec![Some(3)]];
        let mut output_values = vec![vec![
            Db2Value::Decimal("1".to_string()),
            Db2Value::Clob(String::new()),
            Db2Value::Clob(String::new()),
        ]];

        append_zos_lob_combined_chunk_grid_rows(
            &chunk_result,
            &specs,
            &lob_lengths_by_column,
            &mut output_values,
            1,
        )
        .unwrap();

        assert_eq!(output_values[0][1], Db2Value::Clob("abc".to_string()));
        assert_eq!(output_values[0][2], Db2Value::Clob("xyz".to_string()));
    }

    #[test]
    fn trim_zos_lob_chunk_to_remaining_preserves_utf8_boundary() {
        assert_eq!(trim_zos_lob_chunk_to_remaining("aébc", 2), "aé");
        assert_eq!(trim_zos_lob_chunk_to_remaining("abc", 4), "abc");
        assert_eq!(trim_zos_lob_chunk_to_remaining("abc", 0), "");
    }

    #[test]
    fn rewrite_zos_lob_select_materializes_clob_columns() {
        let columns = vec![
            ColumnInfo::new("COL_E2K9_ID".to_string(), "Decimal".to_string(), false),
            ColumnInfo::new("COL_F6N3_DOC".to_string(), "Clob".to_string(), true),
            ColumnInfo::new(
                "DB2_GENERATED_ROWID_FOR_LOBS".to_string(),
                "RowId(40)".to_string(),
                false,
            ),
        ];

        let rewritten = rewrite_zos_lob_select(
            "SELECT * FROM SCM_M6T2.TBL_H8Q4 FETCH FIRST 3 ROWS ONLY",
            &columns,
        )
        .unwrap();

        assert!(rewritten.contains(
            "CAST(SUBSTR(\"COL_F6N3_DOC\", 1, 32704) AS VARCHAR(32704)) AS \"COL_F6N3_DOC\""
        ));
        assert!(rewritten.contains("\"DB2_GENERATED_ROWID_FOR_LOBS\""));
        assert!(rewritten.ends_with("AS DB2NODE_LOB_SRC"));
    }

    #[test]
    fn rewrite_zos_lob_select_ignores_non_lob_columns() {
        let columns = vec![ColumnInfo::new(
            "COL_A3F9_ID".to_string(),
            "Decimal".to_string(),
            false,
        )];

        assert!(rewrite_zos_lob_select("SELECT COL_A3F9_ID FROM T", &columns).is_none());
    }

    #[test]
    fn column_info_for_cursor_fetch_uses_qrydsc_types_when_sqldard_types_are_unknown() {
        let columns = vec![
            ColumnInfo::new("COL_E2K9_ID".to_string(), "Unknown".to_string(), true),
            ColumnInfo::new("COL_F6N3_DOC".to_string(), "Unknown".to_string(), true),
        ];
        let descriptors = vec![
            db2_proto::fdoca::ColumnDescriptor {
                column_index: 0,
                drda_type: 0x0E,
                length: 6,
                precision: 11,
                scale: 0,
                nullable: false,
                ccsid: 0,
                db2_type: db2_proto::types::Db2Type::Decimal {
                    precision: 11,
                    scale: 0,
                },
                byte_order: db2_proto::fdoca::ByteOrder::BigEndian,
            },
            db2_proto::fdoca::ColumnDescriptor {
                column_index: 1,
                drda_type: 0x3E,
                length: 32_704,
                precision: 0,
                scale: 0,
                nullable: false,
                ccsid: 0,
                db2_type: db2_proto::types::Db2Type::VarGraphic(32_704),
                byte_order: db2_proto::fdoca::ByteOrder::BigEndian,
            },
        ];

        let merged = column_info_for_cursor_fetch(&columns, &descriptors);

        assert_eq!(merged[0].name, "COL_E2K9_ID");
        assert_eq!(merged[0].type_name, "Decimal { precision: 11, scale: 0 }");
        assert_eq!(merged[1].name, "COL_F6N3_DOC");
        assert_eq!(merged[1].type_name, "VarGraphic(32704)");
    }

    #[test]
    fn retry_read_query_after_invalid_zos_cursor_state() {
        let params: [&dyn ToSql; 0] = [];
        let err = Error::Sql {
            sqlstate: "26501".to_string(),
            sqlcode: -514,
            message: "SQL_CURSH200C2".to_string(),
        };

        assert!(should_retry_query_after_session_error(
            "SELECT INSP_RPT_ID, INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT",
            &params,
            &err,
        ));
        assert!(error_indicates_stale_session_state(&err));
        assert!(should_retry_zos_lob_chunking_after_decode_error(&err));
    }

    #[test]
    fn z_os_detection_accepts_server_class_when_release_is_generic() {
        let server_info = ServerInfo {
            server_release: "SQL12010".to_string(),
            server_class: "DB2 for z/OS".to_string(),
            ..Default::default()
        };

        assert!(is_db2_zos_server(&server_info));
    }

    #[test]
    fn retry_invalid_cursor_state_only_for_unparameterized_reads() {
        let value = 1i32;
        let params: [&dyn ToSql; 1] = [&value];
        let err = Error::Sql {
            sqlstate: "26501".to_string(),
            sqlcode: -514,
            message: "SQL_CURSH200C2".to_string(),
        };

        assert!(!should_retry_query_after_session_error(
            "SELECT * FROM FIREINSP.INSP_RPT WHERE INSP_RPT_ID = ?",
            &params,
            &err,
        ));
        assert!(!should_retry_query_after_session_error(
            "UPDATE FIREINSP.INSP_RPT SET INSP_RPT_ID = INSP_RPT_ID",
            &[],
            &err,
        ));
    }

    #[test]
    fn retry_read_query_after_wrapped_invalid_cursor_state() {
        let params: [&dyn ToSql; 0] = [];
        let err = Error::Protocol(
            "z/OS LOB base failed: SQL Error [SQLSTATE=26501, SQLCODE=-514]: SQL_CURSH200C2"
                .to_string(),
        );

        assert!(should_retry_query_after_session_error(
            "SELECT INSP_RPT_ID, INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT",
            &params,
            &err,
        ));
    }

    #[test]
    fn retry_after_stale_zos_lob_select_can_use_catalog_route() {
        let params: [&dyn ToSql; 0] = [];
        let server_info = ServerInfo {
            server_release: "DSN12015".to_string(),
            ..Default::default()
        };
        let value = 1i32;
        let param_refs: [&dyn ToSql; 1] = [&value];

        assert!(can_retry_zos_lob_query_from_catalog(
            "SELECT INSP_RPT_ID, INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT WHERE INSP_RPT_ID IN (1,2)",
            &params,
            0,
            None,
            Some(&server_info),
        ));
        assert!(!can_retry_zos_lob_query_from_catalog(
            "SELECT INSP_RPT_ID, INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT WHERE INSP_RPT_ID = ?",
            &param_refs,
            0,
            None,
            Some(&server_info),
        ));
        assert!(!can_retry_zos_lob_query_from_catalog(
            "SELECT * FROM A JOIN B ON A.ID = B.ID",
            &params,
            0,
            None,
            Some(&server_info),
        ));
    }

    #[test]
    fn retry_read_query_does_not_retry_unrelated_protocol_errors() {
        let params: [&dyn ToSql; 0] = [];
        let err = Error::Protocol("query ended with undecoded row data".to_string());

        assert!(!should_retry_query_after_session_error(
            "SELECT INSP_RPT_ID, INSP_RPT_DETL_DOC FROM FIREINSP.INSP_RPT",
            &params,
            &err,
        ));
    }
}
