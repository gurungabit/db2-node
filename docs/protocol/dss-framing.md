# DSS Framing (Data Stream Structure)

Every DRDA message on the wire is wrapped in a DSS envelope. This is the lowest protocol layer.

## Header Format

```
+----------------------------------------------+
|  DSS Header (6 bytes)                        |
|  +--------+--------+--------+--------------+ |
|  | Length  | Magic  | Format | Correlation  | |
|  | 2 bytes| 0xD0   | 1 byte | ID (2 bytes) | |
|  | (BE)   |        |        |              | |
|  +--------+--------+--------+--------------+ |
|  Payload: DDM Object(s)                      |
+----------------------------------------------+
```

### Fields

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 2 bytes | Length | Total DSS length including header (big-endian `u16`) |
| 2 | 1 byte | Magic | Always `0xD0` |
| 3 | 1 byte | Format | Type and flags (see below) |
| 4 | 2 bytes | Correlation ID | Links requests to replies (big-endian `u16`) |

### Format Byte

The format byte encodes both the DSS type and control flags:

| Bit | Mask | Meaning |
|-----|------|---------|
| 0 | `0x01` | **Chained** — more DSS frames follow in this request |
| 1 | `0x02` | **Continue on error** — process next DSS even if this one fails |
| 2 | `0x04` | **Same correlation** — continuation of a previous DSS |
| 3-4 | `0x38` | **DSS type** (see below) |

### DSS Types

| Value | Type | Description |
|-------|------|-------------|
| 1 | Request | Client-to-server command |
| 2 | Reply | Server-to-client response |
| 3 | Object | Data object (e.g., SQLSTT carrying SQL text) |
| 4 | Communication | Protocol-level message |

## Rust Representation

```rust
pub struct DssHeader {
    pub length: u16,
    pub magic: u8,         // always 0xD0
    pub format: DssFormat,
    pub correlation_id: u16,
}

pub struct DssFormat {
    pub chained: bool,
    pub continue_on_error: bool,
    pub same_correlation: bool,
    pub dss_type: DssType,
}

pub enum DssType {
    Request = 1,
    Reply = 2,
    Object = 3,
    Communication = 4,
}
```

## Chaining

DSS chaining is a critical optimization. By setting the `chained` bit, multiple commands can be sent in a single TCP write, reducing round trips.

For example, the connection handshake chains EXCSAT + ACCSEC in one write:

```
[DSS(EXCSAT, chained=true)] [DSS(ACCSEC, chained=false)]
```

The server responds with both replies in one TCP read:

```
[DSS(EXSATRD)] [DSS(ACCSECRD)]
```

## Maximum DSS Size

A single DSS can be up to 32,767 bytes (max `u16` value). For larger payloads (e.g., SQL text > 32KB), use DSS continuation — set the `same_correlation` flag on the continuation DSS.

## Validation Rules

When parsing a DSS:
1. Verify the magic byte is `0xD0`
2. Verify length is at least 6 (header size)
3. Verify the DSS type is valid (1-4)
4. Track correlation IDs to match requests with replies
