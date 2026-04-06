# Architecture

db2-node is a pure Rust implementation of the IBM DRDA wire protocol, exposed to Node.js through napi-rs. This page explains the high-level architecture and how the pieces fit together.

## Overview

```
Node.js (JavaScript/TypeScript)
        |
        v
   napi-rs FFI layer (Rust -> Node.js native addon)
        |
        v
   db2-node (pure Rust)
        +-- connection pool (async, tokio-based)
        +-- DRDA protocol engine
        |     +-- DSS framing (Data Stream Structure)
        |     +-- DDM command/reply objects
        |     +-- FD:OCA data format decoder
        |     +-- EBCDIC <-> UTF-8 codepage conversion
        +-- TCP + TLS transport (tokio + rustls)
               |
               v
          IBM DB2 Server (port 50000)
```

## Crate Structure

The project is organized as a Cargo workspace with three crates:

### db2-proto — Protocol Library

**Zero dependencies.** Pure protocol serialization and deserialization with no I/O.

- `codepoints.rs` — All DDM/DRDA code point constants
- `dss.rs` — DSS framing (serialize/deserialize)
- `ddm.rs` — DDM object builder/parser
- `commands/` — One file per DRDA command (EXCSAT, ACCSEC, SECCHK, etc.)
- `replies/` — One file per DRDA reply (EXSATRD, ACCRDBRM, SQLCARD, etc.)
- `types.rs` — DB2 SQL type definitions and conversions
- `fdoca.rs` — FD:OCA decoder for result set format
- `codepage.rs` — EBCDIC 037 <-> UTF-8 conversion tables

### db2-client — Async Client

Async TCP/TLS client built on tokio.

- `connection.rs` — Single connection management
- `pool.rs` — Connection pool with health checks
- `transport.rs` — TCP + TLS transport layer
- `auth.rs` — Authentication flows
- `statement.rs` — Prepared statement handles
- `cursor.rs` — Result set cursor / row iterator
- `transaction.rs` — Transaction management
- `row.rs` — Row type with column access
- `error.rs` — Error types with SQLSTATE mapping

### db2-napi — Node.js Bindings

napi-rs bindings exposing the Rust client to JavaScript.

- `js_connection.rs` — JavaScript Client class
- `js_pool.rs` — JavaScript Pool class
- `js_statement.rs` — JavaScript PreparedStatement class
- `js_transaction.rs` — JavaScript Transaction class
- `js_types.rs` — JS <-> Rust type conversions

## Key Design Decisions

### Zero Native Dependencies

db2-node has **no dependency** on `libdb2`, `unixODBC`, `OpenSSL`, or any C library. Everything is implemented in pure Rust:

- **Protocol**: Custom DRDA implementation
- **TLS**: rustls (pure Rust)
- **Async I/O**: tokio
- **Node.js FFI**: napi-rs

This means no `apt-get install`, no IBM client packages, and no platform-specific shared libraries.

### Protocol-first Design

The `db2-proto` crate is a pure protocol library with no I/O dependencies. This enables:

- **Unit testing** without a running DB2 instance
- **Fixture-based testing** with captured byte sequences
- **Reuse** in other contexts (e.g., a standalone Rust DB2 client)

### UTF-8 by Default

During connection setup (ACCRDB), we negotiate UTF-8 encoding (CCSID 1208) via TYPDEFOVR. This avoids EBCDIC encoding for all data exchange. A minimal EBCDIC 037 conversion table is included as a fallback for servers that require it (e.g., DB2 on z/OS).

### Connection Pooling

The built-in pool uses tokio's `Semaphore` for concurrency control and provides:

- Configurable min/max connections
- Idle timeout and max lifetime
- Periodic health checks
- Transparent reconnection on broken connections

### Big-Endian Wire Format

All integers in the DRDA protocol are big-endian. The codebase uses `u16::from_be_bytes()` / `u16::to_be_bytes()` consistently for wire format serialization.

## Data Flow

### Query Lifecycle

1. **Node.js** calls `client.query(sql, params)` (JavaScript)
2. **napi-rs** bridges the call to Rust, creates a tokio task
3. **db2-client** acquires a connection, encodes the DRDA command
4. **db2-proto** serializes DSS + DDM frames
5. **transport** sends bytes over TCP/TLS to DB2
6. **DB2 server** processes the query, returns DRDA reply frames
7. **db2-proto** parses the reply (QRYDTA, SQLCARD, etc.)
8. **db2-client** converts to Row objects
9. **napi-rs** converts Rust types to JavaScript objects
10. **Node.js** receives the `QueryResult` promise resolution

### Connection Handshake

The DRDA connection handshake is optimized to just 2 TCP round trips using DSS chaining:

```
Round trip 1:
  Client -> Server: EXCSAT (chained) + ACCSEC
  Server -> Client: EXSATRD + ACCSECRD

Round trip 2:
  Client -> Server: SECCHK (chained) + ACCRDB
  Server -> Client: SECCHKRM + ACCRDBRM
```

See the [Protocol](../protocol/index.md) section for details.

## Dependencies

### db2-proto
None. Zero dependencies.

### db2-client
| Crate | Purpose |
|-------|---------|
| `tokio` | Async runtime, TCP, timers |
| `tokio-rustls` | TLS support |
| `rustls` | Pure Rust TLS |
| `bytes` | Efficient byte buffers |
| `thiserror` | Error type derivation |
| `tracing` | Structured logging |

### db2-napi
| Crate | Purpose |
|-------|---------|
| `napi` | Node.js native addon API |
| `napi-derive` | Proc macros for napi |
| `tokio` | Async runtime bridge |
