/// DDM (Distributed Data Management) object construction and parsing.
///
/// DDM structure:
///   - length: u16 BE (total length including this 4-byte header)
///   - code_point: u16 BE
///   - data: remaining bytes, which may contain nested parameters
///
/// Each nested parameter:
///   - length: u16 BE (total length including this 4-byte header)
///   - code_point: u16 BE
///   - data: remaining bytes
use crate::{ProtoError, Result};

const DDM_HEADER_LEN: usize = 4;
const DDM_INLINE_LEN_LIMIT: usize = 0x7FFF;
const EXTENDED_DDM_FIRST_CHUNK_LEN: usize = 32_757;
const EXTENDED_DDM_CONTINUATION_CHUNK_LEN: usize = 32_764;
const EXTENDED_DDM_CONTINUATION_MARKER_LEN: usize = 2;

/// A parsed DDM parameter (nested within a DDM object).
#[derive(Debug, Clone, PartialEq)]
pub struct DdmParam {
    pub code_point: u16,
    pub data: Vec<u8>,
}

impl DdmParam {
    /// Get data as u16 (big-endian).
    pub fn as_u16(&self) -> Option<u16> {
        if self.data.len() >= 2 {
            Some(u16::from_be_bytes([self.data[0], self.data[1]]))
        } else {
            None
        }
    }

    /// Get data as u32 (big-endian).
    pub fn as_u32(&self) -> Option<u32> {
        if self.data.len() >= 4 {
            Some(u32::from_be_bytes([
                self.data[0],
                self.data[1],
                self.data[2],
                self.data[3],
            ]))
        } else {
            None
        }
    }

    /// Get data as i32 (big-endian).
    pub fn as_i32(&self) -> Option<i32> {
        if self.data.len() >= 4 {
            Some(i32::from_be_bytes([
                self.data[0],
                self.data[1],
                self.data[2],
                self.data[3],
            ]))
        } else {
            None
        }
    }

    /// Get data as a UTF-8 string.
    pub fn as_utf8(&self) -> Option<String> {
        String::from_utf8(self.data.clone()).ok()
    }

    /// Get data as an EBCDIC 037 decoded string.
    pub fn as_ebcdic(&self) -> String {
        crate::codepage::ebcdic037_to_utf8(&self.data)
    }
}

/// A parsed DDM object.
#[derive(Debug, Clone, PartialEq)]
pub struct DdmObject {
    pub code_point: u16,
    pub data: Vec<u8>,
}

impl DdmObject {
    /// Parse one DDM object from the front of `bytes`.
    /// Returns the parsed object and the number of bytes consumed.
    pub fn parse(bytes: &[u8]) -> Result<(Self, usize)> {
        if bytes.len() < DDM_HEADER_LEN {
            return Err(ProtoError::BufferTooShort {
                expected: DDM_HEADER_LEN,
                actual: bytes.len(),
            });
        }
        let raw_length = u16::from_be_bytes([bytes[0], bytes[1]]);
        let code_point = u16::from_be_bytes([bytes[2], bytes[3]]);

        if raw_length == 0x8004 {
            let data = strip_extended_ddm_continuation_markers(&bytes[4..]);
            return Ok((Self { code_point, data }, bytes.len()));
        }

        let (length, header_len) = parse_marshaled_length(bytes)?;
        if length < header_len {
            return Err(ProtoError::Other(format!(
                "DDM length {} is less than header size {} for code point 0x{:04X}",
                length, header_len, code_point
            )));
        }

        if bytes.len() < length {
            return Err(ProtoError::BufferTooShort {
                expected: length,
                actual: bytes.len(),
            });
        }

        let data = bytes[header_len..length].to_vec();
        Ok((Self { code_point, data }, length))
    }

    /// Parse nested parameters from this DDM object's data.
    /// Not all DDM objects contain nested parameters; some contain raw data.
    pub fn parameters(&self) -> Vec<DdmParam> {
        let mut params = Vec::new();
        let mut offset = 0;
        while offset + DDM_HEADER_LEN <= self.data.len() {
            let raw_len = u16::from_be_bytes([self.data[offset], self.data[offset + 1]]);
            let (param_len, header_len) = match parse_marshaled_length(&self.data[offset..]) {
                Ok(result) => result,
                Err(_) => break,
            };
            if param_len < header_len || offset + param_len > self.data.len() {
                break;
            }
            let cp = u16::from_be_bytes([self.data[offset + 2], self.data[offset + 3]]);
            let data = if raw_len == 0x8004 {
                strip_extended_ddm_continuation_markers(
                    &self.data[offset + DDM_HEADER_LEN..offset + param_len],
                )
            } else {
                self.data[offset + header_len..offset + param_len].to_vec()
            };
            params.push(DdmParam {
                code_point: cp,
                data,
            });
            offset += param_len;
        }
        params
    }

    /// Find a specific parameter by code point.
    pub fn find_param(&self, code_point: u16) -> Option<DdmParam> {
        self.parameters()
            .into_iter()
            .find(|p| p.code_point == code_point)
    }

    /// Total serialized length of this DDM object.
    pub fn total_length(&self) -> usize {
        serialized_header_len(self.data.len()) + self.data.len()
    }
}

fn parse_marshaled_length(bytes: &[u8]) -> Result<(usize, usize)> {
    if bytes.len() < DDM_HEADER_LEN {
        return Err(ProtoError::BufferTooShort {
            expected: DDM_HEADER_LEN,
            actual: bytes.len(),
        });
    }

    let raw_length = u16::from_be_bytes([bytes[0], bytes[1]]);
    if raw_length == 0x8004 {
        return Ok((bytes.len(), DDM_HEADER_LEN));
    }

    if (raw_length & 0x8000) == 0 {
        return Ok((raw_length as usize, DDM_HEADER_LEN));
    }

    let header_len = (raw_length & 0x7FFF) as usize;
    if header_len < DDM_HEADER_LEN {
        return Err(ProtoError::Other(format!(
            "extended DDM header length {} is less than minimum {}",
            header_len, DDM_HEADER_LEN
        )));
    }

    if bytes.len() < header_len {
        return Err(ProtoError::BufferTooShort {
            expected: header_len,
            actual: bytes.len(),
        });
    }

    let mut payload_len = 0usize;
    for byte in &bytes[DDM_HEADER_LEN..header_len] {
        payload_len = (payload_len << 8) | (*byte as usize);
    }

    Ok((header_len + payload_len, header_len))
}

fn extended_length_byte_count(payload_len: usize) -> usize {
    let total_len = payload_len.saturating_add(DDM_HEADER_LEN);
    if total_len <= DDM_INLINE_LEN_LIMIT {
        0
    } else if total_len <= 0x7FFF_FFFF {
        4
    } else if total_len <= 0x7FFF_FFFF_FFFF {
        6
    } else {
        8
    }
}

fn serialized_header_len(payload_len: usize) -> usize {
    DDM_HEADER_LEN + extended_length_byte_count(payload_len)
}

fn append_marshaled_object(out: &mut Vec<u8>, code_point: u16, data: &[u8]) {
    let ext_count = extended_length_byte_count(data.len());
    if ext_count == 0 {
        let len = (data.len() + DDM_HEADER_LEN) as u16;
        out.extend_from_slice(&len.to_be_bytes());
        out.extend_from_slice(&code_point.to_be_bytes());
        out.extend_from_slice(data);
        return;
    }

    let raw_length = (0x8000 | (DDM_HEADER_LEN + ext_count) as u16).to_be_bytes();
    out.extend_from_slice(&raw_length);
    out.extend_from_slice(&code_point.to_be_bytes());

    let payload_len = data.len() as u64;
    for shift_index in (0..ext_count).rev() {
        let shift = shift_index * 8;
        out.push(((payload_len >> shift) & 0xFF) as u8);
    }

    out.extend_from_slice(data);
}

fn strip_extended_ddm_continuation_markers(data: &[u8]) -> Vec<u8> {
    if data.len() <= EXTENDED_DDM_FIRST_CHUNK_LEN + EXTENDED_DDM_CONTINUATION_MARKER_LEN {
        return data.to_vec();
    }

    let first_marker_offset = EXTENDED_DDM_FIRST_CHUNK_LEN;
    if first_marker_offset + 2 > data.len()
        || !matches!(
            u16::from_be_bytes([data[first_marker_offset], data[first_marker_offset + 1]]),
            0x7FFE | 0x7FFF
        )
    {
        return data.to_vec();
    }

    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&data[..EXTENDED_DDM_FIRST_CHUNK_LEN]);

    let mut offset = EXTENDED_DDM_FIRST_CHUNK_LEN;
    while offset < data.len() {
        if offset + 2 <= data.len()
            && matches!(
                u16::from_be_bytes([data[offset], data[offset + 1]]),
                0x7FFE | 0x7FFF
            )
        {
            offset += EXTENDED_DDM_CONTINUATION_MARKER_LEN;
        }

        let end = (offset + EXTENDED_DDM_CONTINUATION_CHUNK_LEN).min(data.len());
        out.extend_from_slice(&data[offset..end]);
        offset = end;
    }

    out
}

/// Parse multiple consecutive DDM objects from a byte buffer.
pub fn parse_ddm_objects(bytes: &[u8]) -> Result<Vec<DdmObject>> {
    let mut objects = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        if bytes.len() - offset < 4 {
            break;
        }
        let (obj, consumed) = DdmObject::parse(&bytes[offset..])?;
        objects.push(obj);
        offset += consumed;
    }
    Ok(objects)
}

/// Builder for constructing DDM objects.
#[derive(Debug, Clone)]
pub struct DdmBuilder {
    code_point: u16,
    data: Vec<u8>,
}

impl DdmBuilder {
    /// Create a new builder for a DDM object with the given command code point.
    pub fn new(code_point: u16) -> Self {
        Self {
            code_point,
            data: Vec::new(),
        }
    }

    /// Add a sub-parameter with a code point and raw data.
    pub fn add_code_point(&mut self, cp: u16, data: &[u8]) -> &mut Self {
        append_marshaled_object(&mut self.data, cp, data);
        self
    }

    /// Add a string parameter. Encodes as UTF-8 bytes.
    pub fn add_string(&mut self, cp: u16, s: &str) -> &mut Self {
        self.add_code_point(cp, s.as_bytes())
    }

    /// Add an EBCDIC-encoded string parameter (code page 037).
    pub fn add_ebcdic_string(&mut self, cp: u16, s: &str) -> &mut Self {
        let ebcdic = crate::codepage::utf8_to_ebcdic037(s);
        self.add_code_point(cp, &ebcdic)
    }

    /// Add a u16 parameter (big-endian).
    pub fn add_u16(&mut self, cp: u16, val: u16) -> &mut Self {
        self.add_code_point(cp, &val.to_be_bytes())
    }

    /// Add a u32 parameter (big-endian).
    pub fn add_u32(&mut self, cp: u16, val: u32) -> &mut Self {
        self.add_code_point(cp, &val.to_be_bytes())
    }

    /// Append raw bytes directly to the data section (no sub-code-point header).
    pub fn add_raw(&mut self, data: &[u8]) -> &mut Self {
        self.data.extend_from_slice(data);
        self
    }

    /// Build the complete DDM object as a byte vector.
    /// The result includes the 4-byte DDM header (length + code point).
    pub fn build(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.total_length());
        append_marshaled_object(&mut out, self.code_point, &self.data);
        out
    }

    fn total_length(&self) -> usize {
        serialized_header_len(self.data.len()) + self.data.len()
    }
}

/// Build a single DDM parameter as bytes: length(2) + code_point(2) + data.
pub fn build_param(code_point: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(serialized_header_len(data.len()) + data.len());
    append_marshaled_object(&mut out, code_point, data);
    out
}

/// Build a DDM parameter with a u16 value.
pub fn build_param_u16(code_point: u16, value: u16) -> Vec<u8> {
    build_param(code_point, &value.to_be_bytes())
}

/// Build a complete DDM object: length(2) + code_point(2) + payload.
pub fn build_ddm_object(code_point: u16, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(serialized_header_len(payload.len()) + payload.len());
    append_marshaled_object(&mut out, code_point, payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_and_parse() {
        let mut builder = DdmBuilder::new(0x1041);
        builder.add_string(0x115E, "TestClient");
        builder.add_u16(0x11A2, 3);
        let bytes = builder.build();

        let (obj, consumed) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(obj.code_point, 0x1041);

        let params = obj.parameters();
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].code_point, 0x115E);
        assert_eq!(params[0].as_utf8().unwrap(), "TestClient");
        assert_eq!(params[1].code_point, 0x11A2);
        assert_eq!(params[1].as_u16().unwrap(), 3);
    }

    #[test]
    fn test_parse_multiple() {
        let mut b1 = DdmBuilder::new(0x0001);
        b1.add_string(0x0002, "hello");
        let mut b2 = DdmBuilder::new(0x0003);
        b2.add_u32(0x0004, 42);

        let mut data = b1.build();
        data.extend_from_slice(&b2.build());

        let objects = parse_ddm_objects(&data).unwrap();
        assert_eq!(objects.len(), 2);
        assert_eq!(objects[0].code_point, 0x0001);
        assert_eq!(objects[1].code_point, 0x0003);
    }

    #[test]
    fn test_build_and_parse_extended_length_object() {
        let data = vec![0xAB; 40_000];
        let bytes = build_ddm_object(0x2414, &data);

        assert_eq!(&bytes[0..2], &0x8008u16.to_be_bytes());
        assert_eq!(&bytes[2..4], &0x2414u16.to_be_bytes());
        assert_eq!(&bytes[4..8], &(data.len() as u32).to_be_bytes());

        let (obj, consumed) = DdmObject::parse(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert_eq!(obj.code_point, 0x2414);
        assert_eq!(obj.data, data);
    }

    #[test]
    fn test_parse_extended_length_parameter() {
        let data = vec![0xCD; 40_000];
        let mut builder = DdmBuilder::new(0x1041);
        builder.add_code_point(0x115E, &data);

        let (obj, _) = DdmObject::parse(&builder.build()).unwrap();
        let params = obj.parameters();
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].code_point, 0x115E);
        assert_eq!(params[0].data, data);
    }
}
