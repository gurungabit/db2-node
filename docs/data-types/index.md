# Data Type Support

`db2-node` maps Db2 wire values into plain JavaScript values. The Rust protocol layer recognizes Db2 for z/OS built-in scalar types, including the LOB families documented by IBM: CLOB, DBCLOB, and BLOB. See IBM's [Data types in Db2 for z/OS](https://www.ibm.com/docs/en/db2-for-zos/12.0.0?topic=elements-data-types) reference for the server-side type definitions.

## JavaScript Mappings

| Db2 for z/OS type | JavaScript value | Notes |
|-------------------|------------------|-------|
| `SMALLINT` | `number` | 16-bit signed integer |
| `INTEGER` | `number` | 32-bit signed integer |
| `BIGINT` | `number` | Returned through Node's numeric path; use casts to character/decimal if exact JS-safe integer handling is required above `Number.MAX_SAFE_INTEGER`. |
| `FLOAT(n)`, `REAL`, `DOUBLE` | `number` | Non-finite wire values are normalized to `0` at the JS boundary. |
| `DECIMAL`, `NUMERIC` | `string` | Preserves precision and scale. |
| `DECFLOAT(16)`, `DECFLOAT(34)` | `string` | Preserves decimal floating-point text. |
| `CHAR`, `VARCHAR`, long character data | `string` | CCSID-aware decoding for UTF-8 and common EBCDIC z/OS flows. |
| `GRAPHIC`, `VARGRAPHIC` | `string` | Public `typeName` is shown as `CHAR(n)` / `VARCHAR(n)` with `db2TypeName` preserving `GRAPHIC(n)` / `VARGRAPHIC(n)`. |
| `CLOB` | `string` | z/OS external LOB materialization is handled by the production cleanup modes. |
| `DBCLOB` | `string` | Decoded as character data, with `db2TypeName: "DBCLOB"` in metadata. |
| `BINARY`, `VARBINARY` | `Buffer` | Binary values are returned as Node buffers. |
| `BLOB` | `Buffer` | Inline and materialized BLOB values are returned as Node buffers. |
| `DATE` | `string` | Db2 text form, for example `2026-05-08`. |
| `TIME` | `string` | Db2 text form. |
| `TIMESTAMP(p) WITHOUT TIME ZONE` | `string` | Descriptor length controls precision. |
| `TIMESTAMP(p) WITH TIME ZONE` | `string` | Returned as Db2's text form when the server exposes it through DRDA timestamp metadata. |
| `ROWID` | `string` | Hex string such as `0xDEADBEEF...`. |
| `XML` | `string` | XML document/content text. |
| `BOOLEAN` | `boolean` | Supported when the server supports Boolean columns. |
| Distinct types | Source type mapping | Db2 distinct types use the representation of their source built-in type. |

All nullable columns return `null` when the server value is SQL `NULL`.

## Parameters

Parameterized queries use server-provided input descriptors when Db2 supplies them. When the driver must infer a parameter type, JavaScript values map as follows:

| JavaScript input | Inferred Db2 value |
|------------------|--------------------|
| `number` integer in 32-bit range | `INTEGER` |
| larger integer `number` | `BIGINT` |
| fractional `number` | `DOUBLE` |
| `string` | `VARCHAR` |
| `boolean` | `BOOLEAN` |
| `null` | SQL `NULL` when Db2 provides input metadata |
| `Buffer`, `Uint8Array`, or number array | binary value suitable for `BINARY`, `VARBINARY`, or `BLOB` parameters |

For exact `DECIMAL`, `DECFLOAT`, `DATE`, `TIME`, `TIMESTAMP`, XML, and LOB parameter typing, cast the parameter in SQL:

```ts
await client.query('VALUES CAST(? AS DECFLOAT(34))', ['123456789.00001'])
await client.query('INSERT INTO docs (body) VALUES (CAST(? AS CLOB(1M)))', [xmlText])
await client.query('INSERT INTO files (payload) VALUES (CAST(? AS BLOB(1M)))', [buffer])
```

## z/OS LOB Safety

For Db2 for z/OS, LOB queries can leave external data frames queued after the requested rows are materialized. The default production mode disconnects and warm-replaces a connection when cleanup cannot be verified. Active close mode (`DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE=1`) sends `CLSQRY`, drains remaining `EXTDTA`, and only reuses the socket after Db2 acknowledges the close.

See [Getting Started](../getting-started/index.md#db2-zos-lob-production-modes) for the production mode tradeoffs.

## Source Coverage

This matrix is based on the IBM Db2 for z/OS 12 data type categories: numeric, character, graphic, binary, LOB, datetime, row ID, XML, distinct types, and nullable values. User-defined array types are not exposed as ordinary scalar row values by this driver; use SQL to unnest or serialize arrays before returning them to Node.js.
