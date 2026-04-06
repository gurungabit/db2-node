use std::env;
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
    descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
    query_instance_id: Option<Vec<u8>>,
    fetch_size: u32,
    fetch_calls: usize,
    pub(crate) pending_row_bytes: Vec<u8>,
    closed: bool,
}

impl Cursor {
    /// Create a new cursor for fetching results.
    pub fn new(
        column_info: Vec<ColumnInfo>,
        descriptors: Vec<db2_proto::fdoca::ColumnDescriptor>,
        query_instance_id: Option<Vec<u8>>,
        fetch_size: u32,
    ) -> Self {
        Cursor {
            column_info,
            descriptors,
            query_instance_id,
            fetch_size,
            fetch_calls: 0,
            pending_row_bytes: Vec::new(),
            closed: false,
        }
    }

    /// Fetch the next batch of rows from the server via the given ClientInner.
    /// Returns (rows, end_of_query).
    pub async fn fetch_next_from(
        &mut self,
        inner: &mut ClientInner,
    ) -> Result<(Vec<Row>, bool), Error> {
        if self.closed {
            return Ok((Vec::new(), true));
        }

        let corr_id = inner.next_correlation_id();
        let pkgnamcsn = db2_proto::commands::build_pkgnamcsn(
            &inner.config.database,
            db2_proto::commands::DEFAULT_RDBCOLID,
            inner.package_id,
            &db2_proto::commands::DEFAULT_PKGCNSTKN,
            inner.section_number,
        );

        let cntqry_data = db2_proto::commands::cntqry::build_cntqry(
            &pkgnamcsn,
            self.query_instance_id.as_deref(),
            db2_proto::commands::opnqry::DEFAULT_QRYBLKSZ,
            None,
            None,
        );
        self.fetch_calls += 1;

        let mut writer = DssWriter::new(corr_id);
        writer.write_request(&cntqry_data, false);
        let send_buf = writer.finish();
        if debug_hex_enabled() && self.fetch_calls <= 5 {
            eprintln!(
                "[db2-wire] sending CNTQRY corr={} section={} fetch_size={} bytes={} pending_tail={}",
                corr_id,
                inner.section_number,
                self.fetch_size,
                send_buf.len(),
                self.pending_row_bytes.len()
            );
        }
        inner.send_bytes(&send_buf).await?;

        let frames = inner.read_reply_frames().await?;
        if debug_hex_enabled() && self.fetch_calls <= 5 {
            eprintln!("[db2-wire] CNTQRY received {} frame(s)", frames.len());
        }
        let mut rows = Vec::new();
        let mut end_of_query = false;

        for frame in &frames {
            for obj in ClientInner::parse_ddm_objects(&frame.payload)? {
                if debug_hex_enabled() && self.fetch_calls <= 5 {
                    eprintln!(
                        "[db2-wire] CNTQRY object cp=0x{:04X} len={}",
                        obj.code_point,
                        obj.data.len()
                    );
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
                        for values in decoded_rows {
                            let col_names: Vec<String> =
                                self.column_info.iter().map(|c| c.name.clone()).collect();
                            rows.push(Row::new(col_names, values));
                        }
                    }
                    codepoints::ENDQRYRM => {
                        trace!("Cursor: end of query");
                        end_of_query = true;
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

        if debug_hex_enabled() && self.fetch_calls <= 5 {
            eprintln!(
                "[db2-wire] CNTQRY fetch#{} rows={} end={} pending_tail={}",
                self.fetch_calls,
                rows.len(),
                end_of_query,
                self.pending_row_bytes.len()
            );
        }

        Ok((rows, end_of_query))
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
