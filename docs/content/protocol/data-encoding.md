---
title: "Data Encoding"
weight: 5
---

# Data Encoding

This page covers how DB2 encodes data on the wire: the FD:OCA format, type system, CCSID negotiation, and EBCDIC handling.

## TYPDEFNAM (Type Definition Name)

During ACCRDB, the client and server negotiate a Type Definition Name that controls data encoding. Common values:

| Name | Description |
|------|-------------|
| `QTDSQLX86` | Intel x86 — little-endian integers, IEEE float, UTF-8 |
| `QTDSQLASC` | ASCII-based systems |
| `QTDSQLBC` | Big-endian systems (z/OS) |

db2-node always requests `QTDSQLX86` to ensure standard encoding.

## TYPDEFOVR (Type Definition Overrides)

Override the CCSID (Coded Character Set Identifier) to request UTF-8 encoding:

| Parameter | CCSID | Encoding |
|-----------|-------|----------|
| CCSIDSBC (single-byte) | 1208 | UTF-8 |
| CCSIDDBC (double-byte) | 1200 | UTF-16 |
| CCSIDMBC (mixed-byte) | 1208 | UTF-8 |

By negotiating CCSID 1208, all string data is exchanged in UTF-8, avoiding EBCDIC conversion.

## Row Data Format (QRYDTA)

Rows in QRYDTA are encoded as:

```
[Null indicator(s)] [Column 1 data] [Column 2 data] ...
```

### Null Indicators

Each nullable column has a 1-byte null indicator:
- `0xFF` = NULL
- `0x00` = not null (data follows)

### Column Data Encoding

| DB2 Type | Wire Format |
|----------|-------------|
| SMALLINT | 2 bytes, big-endian signed i16 |
| INTEGER | 4 bytes, big-endian signed i32 |
| BIGINT | 8 bytes, big-endian signed i64 |
| REAL | 4 bytes, IEEE 754 float |
| DOUBLE | 8 bytes, IEEE 754 double |
| DECIMAL | Packed BCD (Binary Coded Decimal) |
| CHAR(n) | Fixed `n` bytes, space-padded |
| VARCHAR(n) | 2-byte length prefix (BE) + UTF-8 bytes |
| DATE | 10 bytes: `YYYY-MM-DD` |
| TIME | 8 bytes: `HH.MM.SS` |
| TIMESTAMP | 26 bytes: `YYYY-MM-DD-HH.MM.SS.ffffff` |
| BOOLEAN | 1 byte: `0x00` = false, `0x01` = true |

### Packed BCD (Decimal)

DECIMAL values use Packed Binary Coded Decimal encoding:
- Each byte holds two decimal digits (4 bits each)
- The last nibble is the sign: `0xC` = positive, `0xD` = negative, `0xF` = unsigned
- Example: `12345` with precision 5 = `0x01 0x23 0x45 0x0C`

## FD:OCA (Formatted Data Object Content Architecture)

FD:OCA defines the structure of result set data. The QRYDSC (Query Description) contains FD:OCA triplets that describe:

1. **Data type** — The DRDA type code for each column
2. **Length** — Fixed or variable length
3. **Nullability** — Whether the column can be null
4. **CCSID** — Character set for string columns

### FD:OCA Triplet Format

```
[Type byte] [Length] [Data...]
```

The triplet types define how to interpret the column data in QRYDTA.

## EBCDIC Handling

DB2 on z/OS and AS/400 uses EBCDIC encoding. DB2 on Linux/Windows uses ASCII/UTF-8.

### Strategy

1. Always negotiate UTF-8 (CCSID 1208) during connection via TYPDEFOVR
2. Maintain a minimal EBCDIC 037 conversion table as fallback
3. The RDBNAM (database name) in ACCSEC/ACCRDB may require EBCDIC encoding on older servers

### EBCDIC Code Page 037

Code Page 037 is the US/Canada EBCDIC variant. Key mappings:

| ASCII | EBCDIC 037 |
|-------|-----------|
| `A` | `0xC1` |
| `Z` | `0xE9` |
| `0` | `0xF0` |
| `9` | `0xF9` |
| Space | `0x40` |

db2-node includes a complete 256-byte bidirectional lookup table for EBCDIC 037 conversion.

## DB2 SQL Type Codes

| DRDA Code | DB2 Type |
|-----------|----------|
| `0x05` | SMALLINT |
| `0x03` | INTEGER |
| `0x17` | BIGINT |
| `0x07` | REAL |
| `0x0F` | DOUBLE |
| `0x0B` | DECIMAL |
| `0x21` | DATE |
| `0x23` | TIME |
| `0x25` | TIMESTAMP |
