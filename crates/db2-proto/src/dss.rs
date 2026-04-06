/// DSS (Data Stream Structure) framing for DRDA protocol.
///
/// A DSS header is 6 bytes:
///   - length: u16 BE (total length of DSS segment including header)
///   - magic:  u8 (always 0xD0)
///   - format: u8 (encodes DSS type and flags)
///   - correlation_id: u16 BE
///
/// Format byte layout:
///   Bits 0-3: DSS type (1=Request, 2=Reply, 3=Object, 4=Communication)
///   Bit 4:    same correlation (continue previous correlation)
///   Bit 5:    (reserved)
///   Bit 6:    chained (more DSS segments follow in this chain)
///   Bit 7:    (reserved)
///
/// In practice the observed format bytes:
///   0x01 = Request, not chained
///   0x41 = Request, chained
///   0x02 = Reply
///   0x03 = Object, not chained
///   0x43 = Object, chained
///   0x13 = Object, same correlation (continuation)
use crate::{ProtoError, Result};

pub const DSS_MAGIC: u8 = 0xD0;
pub const DSS_HEADER_LEN: usize = 6;
/// Maximum length of a single DSS segment.
pub const DSS_MAX_SEGMENT_LEN: usize = 32767;
const DSS_FIRST_PAYLOAD_LEN: usize = DSS_MAX_SEGMENT_LEN - DSS_HEADER_LEN;
const DSS_CONTINUATION_HEADER_LEN: usize = 2;
const DSS_CONTINUATION_PAYLOAD_LEN: usize = DSS_MAX_SEGMENT_LEN - DSS_CONTINUATION_HEADER_LEN;

fn decode_dss_length(raw_length: u16) -> u16 {
    // LUW returns 0xFFFF for max-sized Object DSS segments carrying large QRYDTA blocks.
    // The next DSS header starts 2 bytes earlier than a literal 65535-byte consume,
    // so treat this as a 65533-byte segment.
    if raw_length == 0xFFFF {
        0xFFFD
    } else {
        raw_length
    }
}

/// DSS type identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DssType {
    Request,       // 1
    Reply,         // 2
    Object,        // 3
    Communication, // 4
}

impl DssType {
    pub fn from_byte(b: u8) -> Result<Self> {
        match b & 0x0F {
            1 => Ok(DssType::Request),
            2 => Ok(DssType::Reply),
            3 => Ok(DssType::Object),
            4 => Ok(DssType::Communication),
            other => Err(ProtoError::InvalidDssType(other)),
        }
    }

    pub fn to_byte(self) -> u8 {
        match self {
            DssType::Request => 1,
            DssType::Reply => 2,
            DssType::Object => 3,
            DssType::Communication => 4,
        }
    }
}

/// Flags extracted from the DSS format byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DssFlags {
    pub chained: bool,
    pub continue_on_error: bool,
    pub same_correlation: bool,
}

impl DssFlags {
    pub fn none() -> Self {
        Self {
            chained: false,
            continue_on_error: false,
            same_correlation: false,
        }
    }

    pub fn to_byte(self) -> u8 {
        let mut b = 0u8;
        if self.chained {
            b |= 0x40;
        }
        if self.continue_on_error {
            b |= 0x20;
        }
        if self.same_correlation {
            b |= 0x10;
        }
        b
    }

    pub fn from_byte(b: u8) -> Self {
        Self {
            chained: (b & 0x40) != 0,
            continue_on_error: (b & 0x20) != 0,
            same_correlation: (b & 0x10) != 0,
        }
    }
}

/// A parsed DSS header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DssHeader {
    pub length: u16,
    pub dss_type: DssType,
    pub flags: DssFlags,
    pub correlation_id: u16,
}

impl DssHeader {
    /// Parse a DSS header from a 6-byte slice.
    pub fn parse(data: &[u8]) -> Result<Self> {
        if data.len() < DSS_HEADER_LEN {
            return Err(ProtoError::BufferTooShort {
                expected: DSS_HEADER_LEN,
                actual: data.len(),
            });
        }
        let raw_length = u16::from_be_bytes([data[0], data[1]]);
        let length = decode_dss_length(raw_length);
        let magic = data[2];
        if magic != DSS_MAGIC {
            return Err(ProtoError::InvalidMagic(magic));
        }
        let format = data[3];
        let dss_type = DssType::from_byte(format)?;
        let flags = DssFlags::from_byte(format);
        let correlation_id = u16::from_be_bytes([data[4], data[5]]);
        Ok(Self {
            length,
            dss_type,
            flags,
            correlation_id,
        })
    }

    /// Serialize this DSS header to 6 bytes.
    pub fn serialize(&self) -> [u8; DSS_HEADER_LEN] {
        let len_bytes = self.length.to_be_bytes();
        let format = self.dss_type.to_byte() | self.flags.to_byte();
        let corr = self.correlation_id.to_be_bytes();
        [
            len_bytes[0],
            len_bytes[1],
            DSS_MAGIC,
            format,
            corr[0],
            corr[1],
        ]
    }
}

/// Writer that builds DSS-framed messages around DDM payloads.
///
/// Handles chaining multiple DDM objects in a single request chain,
/// and DSS continuation for payloads > 32767 bytes.
#[derive(Debug, Clone)]
pub struct DssWriter {
    correlation_id: u16,
    buffer: Vec<u8>,
}

impl DssWriter {
    pub fn new(initial_correlation_id: u16) -> Self {
        Self {
            correlation_id: initial_correlation_id,
            buffer: Vec::new(),
        }
    }

    /// Return the accumulated bytes and reset the internal buffer.
    pub fn finish(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.buffer)
    }

    /// Get a reference to the current buffer.
    pub fn data(&self) -> &[u8] {
        &self.buffer
    }

    /// Current correlation ID.
    pub fn correlation_id(&self) -> u16 {
        self.correlation_id
    }

    /// Set the correlation ID explicitly.
    pub fn set_correlation_id(&mut self, id: u16) {
        self.correlation_id = id;
    }

    /// Increment and return next correlation ID.
    pub fn next_correlation_id(&mut self) -> u16 {
        self.correlation_id = self.correlation_id.wrapping_add(1);
        self.correlation_id
    }

    /// Write a single DDM payload wrapped in one or more DSS segments.
    /// If `chained` is true, the chained flag is set on the first DSS segment.
    pub fn write_dss_full(
        &mut self,
        dss_type: DssType,
        chained: bool,
        same_correlation: bool,
        payload: &[u8],
    ) {
        let total = payload.len() + DSS_HEADER_LEN;

        if total <= DSS_MAX_SEGMENT_LEN {
            // Single segment
            let header = DssHeader {
                length: total as u16,
                dss_type,
                flags: DssFlags {
                    chained,
                    continue_on_error: false,
                    same_correlation,
                },
                correlation_id: self.correlation_id,
            };
            self.buffer.extend_from_slice(&header.serialize());
            self.buffer.extend_from_slice(payload);
        } else {
            let header = DssHeader {
                length: 0xFFFF,
                dss_type,
                flags: DssFlags {
                    chained,
                    continue_on_error: false,
                    same_correlation,
                },
                correlation_id: self.correlation_id,
            };
            self.buffer.extend_from_slice(&header.serialize());

            let mut offset = 0usize;
            let first_chunk_len = DSS_FIRST_PAYLOAD_LEN.min(payload.len());
            self.buffer.extend_from_slice(&payload[..first_chunk_len]);
            offset += first_chunk_len;

            while offset < payload.len() {
                let remaining = payload.len() - offset;
                let chunk_len = remaining.min(DSS_CONTINUATION_PAYLOAD_LEN);
                let has_more = remaining > DSS_CONTINUATION_PAYLOAD_LEN;
                let continuation_len = if has_more && chunk_len == DSS_CONTINUATION_PAYLOAD_LEN {
                    0xFFFF
                } else {
                    (chunk_len + DSS_CONTINUATION_HEADER_LEN) as u16
                };
                self.buffer
                    .extend_from_slice(&continuation_len.to_be_bytes());
                self.buffer
                    .extend_from_slice(&payload[offset..offset + chunk_len]);
                offset += chunk_len;
            }
        }
    }

    /// Write a DSS without same_correlation flag (backwards compatible).
    pub fn write_dss(&mut self, dss_type: DssType, chained: bool, payload: &[u8]) {
        self.write_dss_full(dss_type, chained, false, payload);
    }

    /// Convenience: write a Request DSS.
    pub fn write_request(&mut self, payload: &[u8], chained: bool) {
        self.write_dss(DssType::Request, chained, payload);
    }

    /// Convenience: write an Object DSS (used for SQLSTT, SQLDTA, etc.).
    pub fn write_object(&mut self, payload: &[u8], chained: bool) {
        self.write_dss(DssType::Object, chained, payload);
    }

    /// Write an Object DSS with same_correlation flag set.
    /// Used for SQLSTT, SQLDTA that continue a previous Request DSS.
    pub fn write_object_same_corr(&mut self, payload: &[u8], chained: bool) {
        self.write_dss_full(DssType::Object, chained, true, payload);
    }

    /// Write a Request DSS with same_correlation flag set.
    /// Used when the next DSS will share this correlation ID (e.g., PRPSQLSTT before SQLSTT).
    pub fn write_request_next_same_corr(&mut self, payload: &[u8], chained: bool) {
        self.write_dss_full(DssType::Request, chained, true, payload);
    }
}

/// Build a DSS Request frame around a payload.
pub fn build_dss_request(correlation_id: u16, chained: bool, payload: &[u8]) -> Vec<u8> {
    let mut writer = DssWriter::new(correlation_id);
    writer.write_request(payload, chained);
    writer.finish()
}

/// Build a DSS Object frame around a payload.
pub fn build_dss_object(correlation_id: u16, chained: bool, payload: &[u8]) -> Vec<u8> {
    let mut writer = DssWriter::new(correlation_id);
    writer.write_object(payload, chained);
    writer.finish()
}

/// A parsed DSS frame: header + payload bytes.
#[derive(Debug, Clone)]
pub struct DssFrame {
    pub header: DssHeader,
    pub payload: Vec<u8>,
}

/// Reader that parses DSS frames from a byte buffer.
///
/// Handles DSS continuation segments transparently by concatenating
/// continuation segment payloads into a single logical frame.
#[derive(Debug, Clone)]
pub struct DssReader {
    buffer: Vec<u8>,
    position: usize,
}

impl DssReader {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            buffer: data,
            position: 0,
        }
    }

    /// Add more data to the internal buffer.
    pub fn extend(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Return remaining bytes in the buffer.
    pub fn remaining(&self) -> usize {
        self.buffer.len().saturating_sub(self.position)
    }

    /// Check if we have enough data for at least one DSS header.
    pub fn has_complete_frame(&self) -> bool {
        if self.remaining() < DSS_HEADER_LEN {
            return false;
        }
        let raw_length =
            u16::from_be_bytes([self.buffer[self.position], self.buffer[self.position + 1]]);
        let len = decode_dss_length(raw_length) as usize;
        self.remaining() >= len
    }

    /// Parse the next DSS frame, handling continuation segments.
    /// Returns None if not enough data is available.
    pub fn next_frame(&mut self) -> Result<Option<DssFrame>> {
        if !self.has_complete_frame() {
            return Ok(None);
        }

        let header = DssHeader::parse(&self.buffer[self.position..])?;
        let seg_len = header.length as usize;

        if self.remaining() < seg_len {
            return Ok(None);
        }

        let payload_start = self.position + DSS_HEADER_LEN;
        let payload_end = self.position + seg_len;
        let mut payload = self.buffer[payload_start..payload_end].to_vec();
        let first_header = header.clone();
        self.position += seg_len;

        // DSS continuation: only merge segments that are TRUE continuations
        // (same_correlation + continuation of a large DDM > 32KB).
        // Do NOT merge separate logical frames that happen to share correlation IDs.
        // True continuations have same_correlation=true AND contain raw data
        // without a new DDM header (no code_point at the start of the payload).
        loop {
            if self.remaining() < DSS_HEADER_LEN {
                break;
            }
            let peek = DssHeader::parse(&self.buffer[self.position..])?;
            if !peek.flags.same_correlation {
                break;
            }
            // Check if this is a true continuation (same type, no new DDM header)
            // or a separate message. A new DDM header starts with a length field
            // that would be > 4 and a valid code_point.
            // For safety, only merge if the first DSS type was Object and the
            // continuation is also Object, AND the total payload so far exceeds
            // a threshold suggesting it's a multi-segment DDM.
            let continuation_type_ok = peek.dss_type == first_header.dss_type
                || matches!(
                    (first_header.dss_type, peek.dss_type),
                    (DssType::Reply, DssType::Object) | (DssType::Request, DssType::Object)
                );
            if !continuation_type_ok {
                break; // Different logical DSS type = separate message
            }
            // Don't merge if the payload starts with what looks like a DDM header
            let cont_start = self.position + DSS_HEADER_LEN;
            let cont_len = peek.length as usize;
            if self.remaining() < cont_len {
                break;
            }
            // Check if the continuation payload starts with a valid DDM object header
            // (2-byte length + 2-byte code_point where code_point > 0x0100)
            if cont_len > DSS_HEADER_LEN + 4 {
                let payload_start = &self.buffer[cont_start..];
                if payload_start.len() >= 4 {
                    let ddm_len = u16::from_be_bytes([payload_start[0], payload_start[1]]) as usize;
                    let ddm_cp = u16::from_be_bytes([payload_start[2], payload_start[3]]);
                    // If this looks like a DDM object (length matches, code_point in range)
                    // then it's a separate message, not a continuation
                    if ddm_cp > 0x0100 && ddm_len <= (cont_len - DSS_HEADER_LEN + 4) {
                        break;
                    }
                }
            }
            let cont_end = self.position + cont_len;
            let mut cont_payload = &self.buffer[cont_start..cont_end];
            // Large extended DDM objects (for example QRYDTA with a 0x8004
            // header) prepend a 2-byte continuation marker on follow-on DSS
            // segments. Keep it out of the logical payload or downstream row
            // decoders will see stray bytes at every segment boundary.
            if payload.len() >= 4
                && payload[0] == 0x80
                && payload[1] == 0x04
                && cont_payload.len() >= 2
                && matches!(
                    u16::from_be_bytes([cont_payload[0], cont_payload[1]]),
                    0x7FFE | 0x7FFF
                )
            {
                cont_payload = &cont_payload[2..];
            }
            payload.extend_from_slice(cont_payload);
            self.position += cont_len;
        }

        Ok(Some(DssFrame {
            header: first_header,
            payload,
        }))
    }

    /// Parse all available frames.
    pub fn read_all_frames(&mut self) -> Result<Vec<DssFrame>> {
        let mut frames = Vec::new();
        while let Some(frame) = self.next_frame()? {
            frames.push(frame);
        }
        Ok(frames)
    }

    /// Consume the reader and return any unprocessed bytes.
    pub fn into_remaining(self) -> Vec<u8> {
        if self.position >= self.buffer.len() {
            Vec::new()
        } else {
            self.buffer[self.position..].to_vec()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_header_roundtrip() {
        let hdr = DssHeader {
            length: 100,
            dss_type: DssType::Request,
            flags: DssFlags {
                chained: true,
                continue_on_error: false,
                same_correlation: false,
            },
            correlation_id: 1,
        };
        let bytes = hdr.serialize();
        assert_eq!(bytes[2], DSS_MAGIC);
        assert_eq!(bytes[3], 0x41); // chained request
        let parsed = DssHeader::parse(&bytes).unwrap();
        assert_eq!(parsed.length, 100);
        assert_eq!(parsed.dss_type, DssType::Request);
        assert!(parsed.flags.chained);
        assert_eq!(parsed.correlation_id, 1);
    }

    #[test]
    fn test_writer_reader_roundtrip() {
        let payload = vec![0x00, 0x08, 0x10, 0x41, 0xDE, 0xAD, 0xBE, 0xEF];
        let mut writer = DssWriter::new(1);
        writer.write_request(&payload, false);
        let data = writer.finish();

        let mut reader = DssReader::new(data);
        let frame = reader.next_frame().unwrap().unwrap();
        assert_eq!(frame.header.dss_type, DssType::Request);
        assert_eq!(frame.payload, payload);
    }

    #[test]
    fn test_writer_large_payload_uses_continuation_headers() {
        let payload = vec![0x5A; 70_000];
        let mut writer = DssWriter::new(7);
        writer.write_object(&payload, false);
        let data = writer.finish();

        assert_eq!(&data[0..2], &0xFFFFu16.to_be_bytes());
        assert_eq!(data[2], DSS_MAGIC);
        assert_eq!(data[3], DssType::Object.to_byte());
        assert_eq!(&data[4..6], &7u16.to_be_bytes());
        assert_eq!(
            &data[6..6 + DSS_FIRST_PAYLOAD_LEN],
            &payload[..DSS_FIRST_PAYLOAD_LEN]
        );

        let first_continuation_offset = DSS_HEADER_LEN + DSS_FIRST_PAYLOAD_LEN;
        assert_eq!(
            u16::from_be_bytes([
                data[first_continuation_offset],
                data[first_continuation_offset + 1],
            ]),
            0xFFFF
        );

        let second_continuation_offset =
            first_continuation_offset + DSS_CONTINUATION_HEADER_LEN + DSS_CONTINUATION_PAYLOAD_LEN;
        let tail_len = payload.len() - DSS_FIRST_PAYLOAD_LEN - DSS_CONTINUATION_PAYLOAD_LEN;
        assert_eq!(
            u16::from_be_bytes([
                data[second_continuation_offset],
                data[second_continuation_offset + 1],
            ]) as usize,
            tail_len + DSS_CONTINUATION_HEADER_LEN
        );
        assert_eq!(
            data.len(),
            DSS_HEADER_LEN + payload.len() + (2 * DSS_CONTINUATION_HEADER_LEN)
        );
    }
}
