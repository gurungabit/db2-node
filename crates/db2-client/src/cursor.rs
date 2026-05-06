use std::env;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;
use tracing::trace;

use crate::column::ColumnInfo;
use crate::connection::ClientInner;
use crate::error::Error;
use crate::row::Row;
use db2_proto::codepoints;
use db2_proto::dss::DssWriter;

/// Internal cursor for iterating over query result sets.
///
/// The cursor sends CNTQRY requests to fetch additional rows and
/// detects ENDQRYRM to know when the result set is exhausted.
pub(crate) struct Cursor {
    column_info: Vec<ColumnInfo>,
    column_names: Arc<[String]>,
    pub(crate) descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
    query_instance_id: Option<Vec<u8>>,
    pkgnamcsn: Vec<u8>,
    fetch_size: u32,
    close_after_next_fetch: bool,
    fetch_calls: usize,
    pub(crate) pending_row_bytes: Vec<u8>,
    pub(crate) last_fetch_diagnostics: Vec<String>,
    closed: bool,
}

impl Cursor {
    /// Create a new cursor for fetching results.
    pub fn new(
        column_info: Vec<ColumnInfo>,
        descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
        query_instance_id: Option<Vec<u8>>,
        pkgnamcsn: Vec<u8>,
        fetch_size: u32,
        close_after_next_fetch: bool,
    ) -> Self {
        let column_names = column_info
            .iter()
            .map(|c| c.name.clone())
            .collect::<Vec<_>>()
            .into();

        Cursor {
            column_info,
            column_names,
            descriptors,
            query_instance_id,
            pkgnamcsn,
            fetch_size,
            close_after_next_fetch,
            fetch_calls: 0,
            pending_row_bytes: Vec::new(),
            last_fetch_diagnostics: Vec::new(),
            closed: false,
        }
    }

    /// Fetch the next batch of rows from the server via the given ClientInner.
    /// Returns (rows, end_of_query, externalized LOB payloads).
    pub async fn fetch_next_from(
        &mut self,
        inner: &mut ClientInner,
    ) -> Result<(Vec<Row>, bool, Vec<Vec<u8>>), Error> {
        if self.closed {
            return Ok((Vec::new(), true, Vec::new()));
        }

        let corr_id = inner.next_correlation_id();
        let has_lobs = descriptors_need_lob_fetch(&self.descriptors)
            || column_info_needs_lob_fetch(&self.column_info);
        let collect_diagnostics = crate::connection::query_diagnostics_enabled();

        self.last_fetch_diagnostics.clear();
        let cntqry_data = if has_lobs && crate::connection::use_native_zos_lob_strategy() {
            if collect_diagnostics {
                self.last_fetch_diagnostics.push(
                    "cntqry_request has_lobs=true native_limited_block=true rdbnam=false maxblkext=-1 qryrowset=none rtnextdta=RTNEXTALL"
                        .to_string(),
                );
            }
            db2_proto::commands::cntqry::build_cntqry_with_rtnextdta(
                &self.pkgnamcsn,
                self.query_instance_id.as_deref(),
                db2_proto::commands::opnqry::DEFAULT_QRYBLKSZ,
                Some(-1),
                None,
                Some(codepoints::RTNEXTALL),
            )
        } else {
            let use_extended_materialized_blocks = inner.zos_lob_internal_depth > 0
                && inner
                    .server_info
                    .as_ref()
                    .map_or(false, crate::connection::is_db2_zos_server);
            let use_zos_non_lob_fetch = !has_lobs
                && inner.zos_lob_internal_depth == 0
                && inner
                    .server_info
                    .as_ref()
                    .map_or(false, crate::connection::is_db2_zos_server);
            let use_zos_non_lob_extra_blocks =
                use_zos_non_lob_fetch && use_zos_non_lob_cntqry_extra_blocks();
            let use_extra_blocks = use_extended_materialized_blocks || use_zos_non_lob_extra_blocks;
            let qryblksz = if use_zos_non_lob_fetch {
                crate::connection::zos_non_lob_qryblksz()
            } else {
                db2_proto::commands::opnqry::DEFAULT_QRYBLKSZ
            };
            let qryrowset = if use_zos_non_lob_extra_blocks {
                zos_non_lob_cntqry_rowset(self.fetch_size)
            } else {
                None
            };
            if collect_diagnostics {
                self.last_fetch_diagnostics.push(format!(
                    "cntqry_request has_lobs={} native_lobs=false rdbnam=false maxblkext={} qryrowset={} rtnextdta=none non_lob_extra_blocks={} qryblksz={}",
                    has_lobs,
                    if use_extra_blocks { "-1" } else { "none" },
                    qryrowset
                        .map(|value| value.to_string())
                        .unwrap_or_else(|| "none".to_string()),
                    use_zos_non_lob_extra_blocks,
                    qryblksz
                ));
            }
            db2_proto::commands::cntqry::build_cntqry(
                &self.pkgnamcsn,
                self.query_instance_id.as_deref(),
                qryblksz,
                use_extra_blocks.then_some(-1),
                qryrowset,
            )
        };
        self.fetch_calls += 1;

        let close_after_this_fetch = self.close_after_next_fetch && !has_lobs;
        if close_after_this_fetch {
            self.close_after_next_fetch = false;
        }
        let close_corr_id = close_after_this_fetch.then(|| inner.next_correlation_id());

        let mut writer = DssWriter::new(corr_id);
        if close_after_this_fetch {
            let clsqry_data = db2_proto::commands::clsqry::build_clsqry(&self.pkgnamcsn);
            writer.write_request(&cntqry_data, true);
            writer.set_correlation_id(close_corr_id.expect("close correlation id"));
            writer.write_request(&clsqry_data, false);
        } else {
            writer.write_request(&cntqry_data, false);
        }
        let send_buf = writer.finish();
        if collect_diagnostics && has_lobs {
            self.last_fetch_diagnostics.push(format!(
                "cntqry_send bytes={} preview={}",
                send_buf.len(),
                format_hex_preview(&send_buf, 160)
            ));
        }
        if debug_hex_enabled() && self.fetch_calls <= 5 {
            eprintln!(
                "[db2-wire] sending CNTQRY corr={} section={} fetch_size={} has_lobs={} bytes={} pending_tail={}",
                corr_id,
                inner.section_number,
                self.fetch_size,
                has_lobs,
                send_buf.len(),
                self.pending_row_bytes.len()
            );
        }
        inner.send_bytes(&send_buf).await?;

        let read_timeout = if inner.config.query_timeout.is_zero() {
            Duration::from_secs(30)
        } else {
            inner.config.query_timeout
        };
        let frames = match timeout(read_timeout, inner.read_reply_frames()).await {
            Ok(result) => result?,
            Err(_) => {
                return Err(Error::Timeout(format!(
                    "fetch timed out after {:?}; has_lobs={} pending_tail={} column_types=[{}] last_fetch=[{}]",
                    read_timeout,
                    has_lobs,
                    self.pending_row_bytes.len(),
                    column_type_summary(&self.column_info),
                    self.last_fetch_diagnostics.join("; ")
                )));
            }
        };
        if debug_hex_enabled() && self.fetch_calls <= 5 {
            eprintln!("[db2-wire] CNTQRY received {} frame(s)", frames.len());
        }
        let mut rows = Vec::new();
        let mut extdta_payloads = Vec::new();
        let mut end_of_query = false;
        let mut close_reply_seen = close_corr_id.is_some_and(|close_corr_id| {
            frames
                .iter()
                .any(|frame| frame.header.correlation_id == close_corr_id)
        });

        self.process_fetch_frames(
            &frames,
            &mut rows,
            &mut extdta_payloads,
            &mut end_of_query,
            collect_diagnostics,
        )?;
        apply_extdta_payloads_to_rows(&mut rows, &self.descriptors, &extdta_payloads);

        if close_after_this_fetch && !close_reply_seen {
            let drain_timeout = zos_non_lob_fetch_end_drain_timeout();
            if !drain_timeout.is_zero() {
                let previous_end_of_query = end_of_query;
                end_of_query = true;
                match timeout(drain_timeout, inner.read_reply_frames()).await {
                    Ok(Ok(more_frames)) => {
                        close_reply_seen = close_corr_id.is_some_and(|close_corr_id| {
                            more_frames
                                .iter()
                                .any(|frame| frame.header.correlation_id == close_corr_id)
                        });
                        if collect_diagnostics {
                            self.last_fetch_diagnostics.push(format!(
                                "non_lob_close_drain frames={} close_reply_seen={}",
                                more_frames.len(),
                                close_reply_seen
                            ));
                        }
                        self.process_fetch_frames(
                            &more_frames,
                            &mut rows,
                            &mut extdta_payloads,
                            &mut end_of_query,
                            collect_diagnostics,
                        )?;
                    }
                    Ok(Err(err)) => return Err(err),
                    Err(_) => {
                        end_of_query = previous_end_of_query;
                        if collect_diagnostics {
                            self.last_fetch_diagnostics
                                .push("non_lob_close_drain timed_out".to_string());
                        }
                    }
                }
            }
        }

        if has_lobs && crate::connection::use_native_zos_lob_strategy() {
            while native_fetch_needs_more_frames(
                &rows,
                &self.descriptors,
                &self.pending_row_bytes,
                end_of_query,
            ) {
                let more_frames = match timeout(
                    crate::connection::native_zos_lob_frame_drain_timeout(),
                    inner.read_reply_frames(),
                )
                .await
                {
                    Ok(Ok(frames)) => frames,
                    Ok(Err(err)) => return Err(err),
                    Err(_) => {
                        if collect_diagnostics {
                            self.last_fetch_diagnostics.push(format!(
                                "native_lob_adaptive_drain timed_out rows={} extdta={} pending_tail={} unresolved={}",
                                rows.len(),
                                extdta_payloads.len(),
                                self.pending_row_bytes.len(),
                                rows_need_extdta_payloads(&rows, &self.descriptors)
                            ));
                        }
                        break;
                    }
                };
                if more_frames.is_empty() {
                    break;
                }
                if collect_diagnostics {
                    self.last_fetch_diagnostics.push(format!(
                        "native_lob_adaptive_drain extra_frames={}",
                        more_frames.len()
                    ));
                }
                self.process_fetch_frames(
                    &more_frames,
                    &mut rows,
                    &mut extdta_payloads,
                    &mut end_of_query,
                    collect_diagnostics,
                )?;
                apply_extdta_payloads_to_rows(&mut rows, &self.descriptors, &extdta_payloads);
            }
        }

        if should_drain_zos_non_lob_fetch_end(inner, has_lobs, &rows, end_of_query) {
            let drain_timeout = zos_non_lob_fetch_end_drain_timeout();
            loop {
                let more_frames = match timeout(drain_timeout, inner.read_reply_frames()).await {
                    Ok(Ok(frames)) => frames,
                    Ok(Err(err)) => return Err(err),
                    Err(_) => {
                        if collect_diagnostics {
                            self.last_fetch_diagnostics.push(format!(
                                "non_lob_end_drain timed_out rows={} pending_tail={}",
                                rows.len(),
                                self.pending_row_bytes.len()
                            ));
                        }
                        break;
                    }
                };
                if more_frames.is_empty() {
                    break;
                }
                if collect_diagnostics {
                    self.last_fetch_diagnostics.push(format!(
                        "non_lob_end_drain extra_frames={}",
                        more_frames.len()
                    ));
                }
                self.process_fetch_frames(
                    &more_frames,
                    &mut rows,
                    &mut extdta_payloads,
                    &mut end_of_query,
                    collect_diagnostics,
                )?;
                if end_of_query {
                    break;
                }
            }
        }

        if close_after_this_fetch && !end_of_query && !rows.is_empty() {
            end_of_query = true;
            self.closed = true;
            if collect_diagnostics {
                self.last_fetch_diagnostics
                    .push("non_lob_fetch_closed_with_bounded_rowset=true".to_string());
            }
        }

        if debug_hex_enabled() && self.fetch_calls <= 5 {
            eprintln!(
                "[db2-wire] CNTQRY fetch#{} rows={} end={} pending_tail={}",
                self.fetch_calls,
                rows.len(),
                end_of_query,
                self.pending_row_bytes.len()
            );
        }

        Ok((rows, end_of_query, extdta_payloads))
    }

    pub(crate) async fn close_from(&mut self, inner: &mut ClientInner) -> Result<(), Error> {
        if self.closed {
            return Ok(());
        }

        let collect_diagnostics = crate::connection::query_diagnostics_enabled();
        let corr_id = inner.next_correlation_id();
        let clsqry_data = db2_proto::commands::clsqry::build_clsqry(&self.pkgnamcsn);
        let mut writer = DssWriter::new(corr_id);
        writer.write_request(&clsqry_data, false);
        let send_buf = writer.finish();
        inner.send_bytes(&send_buf).await?;

        let drain_timeout = zos_lob_close_drain_timeout();
        match timeout(drain_timeout, inner.read_reply_frames()).await {
            Ok(Ok(frames)) => {
                if collect_diagnostics {
                    self.last_fetch_diagnostics.push(format!(
                        "lob_close_drain frames={} corr={} bytes={}",
                        frames.len(),
                        corr_id,
                        send_buf.len()
                    ));
                }
                let mut rows = Vec::new();
                let mut extdta_payloads = Vec::new();
                let mut end_of_query = false;
                self.process_fetch_frames(
                    &frames,
                    &mut rows,
                    &mut extdta_payloads,
                    &mut end_of_query,
                    collect_diagnostics,
                )?;
            }
            Ok(Err(err)) => return Err(err),
            Err(_) => {
                if collect_diagnostics {
                    self.last_fetch_diagnostics.push(format!(
                        "lob_close_drain timed_out corr={} bytes={}",
                        corr_id,
                        send_buf.len()
                    ));
                }
            }
        }

        self.closed = true;
        Ok(())
    }

    fn process_fetch_frames(
        &mut self,
        frames: &[db2_proto::dss::DssFrame],
        rows: &mut Vec<Row>,
        extdta_payloads: &mut Vec<Vec<u8>>,
        end_of_query: &mut bool,
        collect_diagnostics: bool,
    ) -> Result<(), Error> {
        for (frame_index, frame) in frames.iter().enumerate() {
            let objects = ClientInner::parse_ddm_objects(&frame.payload)?;
            if objects.is_empty() {
                if collect_diagnostics {
                    self.last_fetch_diagnostics.push(format!(
                        "frame#{} corr={} objects=0 payload_len={}",
                        frame_index,
                        frame.header.correlation_id,
                        frame.payload.len()
                    ));
                }
                continue;
            }
            for obj in objects {
                if collect_diagnostics {
                    self.last_fetch_diagnostics.push(format!(
                        "frame#{} corr={} cp=0x{:04X} len={} preview={}",
                        frame_index,
                        frame.header.correlation_id,
                        obj.code_point,
                        obj.data.len(),
                        format_hex_preview(&obj.data, 96)
                    ));
                }
                if debug_hex_enabled() && self.fetch_calls <= 5 {
                    eprintln!(
                        "[db2-wire] CNTQRY object cp=0x{:04X} len={}",
                        obj.code_point,
                        obj.data.len()
                    );
                }
                if *end_of_query && obj.code_point == codepoints::QRYNOPRM {
                    trace!("Cursor: ignoring late QRYNOPRM after ENDQRYRM");
                    continue;
                }
                if let Some(err) = crate::connection::protocol_reply_error(&obj, "fetch") {
                    if collect_diagnostics {
                        return Err(Error::Protocol(format!(
                            "{}; last_fetch=[{}]",
                            err,
                            self.last_fetch_diagnostics.join("; ")
                        )));
                    }
                    return Err(err);
                }
                match obj.code_point {
                    codepoints::QRYDTA => {
                        trace!("Cursor: received QRYDTA");
                        if debug_hex_enabled() && self.fetch_calls <= 5 {
                            eprintln!(
                                "[db2-wire] CNTQRY QRYDTA preview={} descriptors={:?}",
                                format_hex_preview(&obj.data, 128),
                                self.descriptors
                            );
                            if obj.data.len() > 16_000 {
                                let mid = (obj.data.len() / 2).saturating_sub(64);
                                let end = (mid + 128).min(obj.data.len());
                                eprintln!(
                                    "[db2-wire] CNTQRY QRYDTA mid[{}..{}]={}",
                                    mid,
                                    end,
                                    format_hex_preview(&obj.data[mid..end], 128)
                                );
                            }
                        }
                        let decoded_rows = db2_proto::fdoca::decode_rows_with_tail(
                            &obj.data,
                            &self.descriptors,
                            &mut self.pending_row_bytes,
                        )
                        .map_err(|e| Error::Protocol(e.to_string()))?;
                        if !decoded_rows.is_empty() {
                            for values in decoded_rows {
                                rows.push(Row::new_shared(self.column_names.clone(), values));
                            }
                        }
                    }
                    codepoints::EXTDTA => {
                        trace!("Cursor: received EXTDTA");
                        extdta_payloads.push(obj.data);
                    }
                    codepoints::ENDQRYRM => {
                        trace!("Cursor: end of query");
                        *end_of_query = true;
                        self.closed = true;
                    }
                    codepoints::SQLCARD => {
                        trace!("Cursor: received SQLCARD");
                        let card = db2_proto::replies::sqlcard::parse_sqlcard(&obj)
                            .map_err(|e| Error::Protocol(e.to_string()))?;
                        if card.is_error() {
                            if debug_hex_enabled() {
                                eprintln!(
                                    "[db2-wire] CNTQRY SQLCARD error sqlcode={} sqlstate={}",
                                    card.sqlcode, card.sqlstate
                                );
                            }
                            return Err(Error::Sql {
                                sqlstate: card.sqlstate,
                                sqlcode: card.sqlcode,
                                message: format!("Error during fetch: {}", card.sqlerrmc),
                            });
                        }
                    }
                    _ => {
                        trace!("Cursor: ignoring reply codepoint 0x{:04X}", obj.code_point);
                    }
                }
            }
        }

        Ok(())
    }
}

fn should_drain_zos_non_lob_fetch_end(
    inner: &ClientInner,
    has_lobs: bool,
    rows: &[Row],
    end_of_query: bool,
) -> bool {
    !has_lobs
        && !rows.is_empty()
        && !end_of_query
        && inner.zos_lob_internal_depth == 0
        && inner
            .server_info
            .as_ref()
            .map_or(false, crate::connection::is_db2_zos_server)
        && !zos_non_lob_fetch_end_drain_timeout().is_zero()
}

fn use_zos_non_lob_cntqry_extra_blocks() -> bool {
    env::var("DB2_ZOS_NON_LOB_CNTQRY_EXTRA_BLOCKS")
        .map(|value| {
            let value = value.trim().to_ascii_lowercase();
            !(value == "0" || value == "false" || value == "off" || value == "no")
        })
        .unwrap_or(true)
}

fn zos_non_lob_cntqry_rowset(fetch_size: u32) -> Option<u32> {
    let Ok(value) = env::var("DB2_ZOS_NON_LOB_CNTQRY_ROWSET") else {
        return None;
    };
    let value = value
        .parse::<u64>()
        .unwrap_or_else(|_| u64::from(fetch_size.clamp(1, 32_767)))
        .clamp(0, 32_767);
    (value > 0).then_some(value as u32)
}

fn zos_non_lob_fetch_end_drain_timeout() -> Duration {
    Duration::from_millis(env_u64("DB2_ZOS_NON_LOB_FETCH_END_DRAIN_MS", 2, 0, 25))
}

fn zos_lob_close_drain_timeout() -> Duration {
    Duration::from_millis(env_u64("DB2_ZOS_LOB_CLOSE_DRAIN_MS", 25, 0, 250))
}

fn env_u64(name: &str, default: u64, min: u64, max: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .map(|value| value.clamp(min, max))
        .unwrap_or(default)
}

fn native_fetch_needs_more_frames(
    rows: &[Row],
    descriptors: &[db2_proto::fdoca::ColumnDescriptor],
    pending_row_bytes: &[u8],
    end_of_query: bool,
) -> bool {
    rows_need_extdta_payloads(rows, descriptors)
        || (!end_of_query && (rows.is_empty() || !pending_row_bytes.is_empty()))
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
                db2_proto::types::Db2Type::Clob
                | db2_proto::types::Db2Type::DbClob
                | db2_proto::types::Db2Type::ClobLocator
                | db2_proto::types::Db2Type::DbClobLocator
                | db2_proto::types::Db2Type::LobChar(_)
                | db2_proto::types::Db2Type::VarChar(_)
                | db2_proto::types::Db2Type::VarGraphic(_) => {
                    *value = db2_proto::types::Db2Value::Clob(
                        String::from_utf8_lossy(payload).to_string(),
                    );
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

fn descriptors_need_lob_fetch(descriptors: &[db2_proto::fdoca::ColumnDescriptor]) -> bool {
    descriptors.iter().any(|descriptor| {
        is_lob_descriptor(descriptor) || is_lob_like_inline_descriptor(descriptor)
    })
}

fn column_info_needs_lob_fetch(columns: &[ColumnInfo]) -> bool {
    columns.iter().any(|column| {
        let ty = column.type_name.to_ascii_lowercase();
        let name = column.name.to_ascii_lowercase();
        ty.contains("clob")
            || ty.contains("blob")
            || ty == "unknown"
            || ty.contains("varchar(32704)")
            || ty.contains("vargraphic(32704)")
            || name.contains("lob")
    })
}

fn column_type_summary(columns: &[ColumnInfo]) -> String {
    columns
        .iter()
        .map(|column| format!("{}:{}", column.name, column.type_name))
        .collect::<Vec<_>>()
        .join(",")
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
    )
}

fn is_lob_like_inline_descriptor(descriptor: &db2_proto::fdoca::ColumnDescriptor) -> bool {
    match descriptor.db2_type {
        db2_proto::types::Db2Type::VarChar(len)
        | db2_proto::types::Db2Type::VarGraphic(len)
        | db2_proto::types::Db2Type::LobBytes(len)
        | db2_proto::types::Db2Type::LobChar(len) => len >= 32_704,
        _ => false,
    }
}

fn descriptor_uses_extdta(descriptor: &db2_proto::fdoca::ColumnDescriptor) -> bool {
    is_lob_descriptor(descriptor) || is_lob_like_inline_descriptor(descriptor)
}

fn value_needs_extdta(value: &db2_proto::types::Db2Value) -> bool {
    match value {
        db2_proto::types::Db2Value::Clob(value) => value.starts_with("LOB locator 0x"),
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
