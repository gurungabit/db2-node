---
title: "DDM Objects"
weight: 2
---

# DDM Objects (Distributed Data Management)

Inside each DSS frame, DDM objects carry the actual protocol commands and data. DDM uses a recursive TLV (Type-Length-Value) structure.

## Object Format

```
+-------------------------------------+
|  DDM Header (4 bytes)               |
|  +------------+-------------------+ |
|  |  Length     |  Code Point       | |
|  |  2 bytes   |  2 bytes          | |
|  |  (BE)      |  (identifies cmd) | |
|  +------------+-------------------+ |
|  Nested DDM parameters...           |
|  Each parameter:                    |
|    Length (2) + CodePoint (2) + Data |
+-------------------------------------+
```

### Fields

| Offset | Size | Field | Description |
|--------|------|-------|-------------|
| 0 | 2 bytes | Length | Total object length including header (big-endian) |
| 2 | 2 bytes | Code Point | Identifies the command or parameter type |
| 4 | variable | Data | Nested parameters or raw data |

## Nested Structure

DDM objects are recursively nested. A command like EXCSAT contains multiple parameters, each with their own length, code point, and value:

```
EXCSAT (0x1041)
  +-- EXTNAM (0x115E): "db2-node-client"
  +-- SRVNAM (0x116D): "db2-node"
  +-- SRVRLSLV (0x115A): "db2node00100"
  +-- SRVCLSNM (0x1147): "db2-node"
  +-- MGRLVLLS (0x1404):
        +-- AGENT (0x1403): level 7
        +-- SQLAM (0x2407): level 7
        +-- RDB (0x240F): level 7
        +-- SECMGR (0x1440): level 7
        +-- CMNTCPIP (0x1474): level 5
```

## Building DDM Objects

The DDM builder creates properly framed objects:

```rust
let excsat = DdmBuilder::new(EXCSAT)
    .add_string(EXTNAM, "db2-node-client")
    .add_string(SRVNAM, "db2-node")
    .add_string(SRVRLSLV, "db2node00100")
    .add_string(SRVCLSNM, "db2-node")
    .add_bytes(MGRLVLLS, &mgrlvlls_bytes)
    .build();
```

## Parsing DDM Objects

Parsing walks the byte buffer, reading length + code point pairs:

```rust
let ddm = DdmObject::parse(&bytes)?;
assert_eq!(ddm.code_point, EXCSAT);

for param in ddm.parameters() {
    match param.code_point {
        EXTNAM => { /* handle external name */ }
        SRVNAM => { /* handle server name */ }
        MGRLVLLS => { /* handle manager levels */ }
        _ => { /* unknown parameter, skip */ }
    }
}
```

## Manager Level List (MGRLVLLS)

The MGRLVLLS parameter is specially encoded as pairs of `(code_point: u16, level: u16)`:

```
MGRLVLLS data:
  0x1403 0x0007   -- AGENT level 7
  0x2407 0x0007   -- SQLAM level 7
  0x240F 0x0007   -- RDB level 7
  0x1440 0x0007   -- SECMGR level 7
  0x1474 0x0005   -- CMNTCPIP level 5
```

These levels negotiate protocol capabilities between client and server. Higher levels enable newer features.
