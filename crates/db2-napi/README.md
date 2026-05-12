# db2-node

Pure Rust DB2 driver for Node.js using the DRDA wire protocol directly. No IBM CLI, ODBC, or `libdb2` dependency is required at runtime.

## Status

`1.0.9` is the current production release for DB2 LUW connectivity and Db2 z/OS encrypted authentication with z/OS-compatible security-check framing, parameterized queries, prepared statements, transactions, connection pooling, TLS, `ibm_db` compatibility entry points, and validated z/OS LOB materialization cleanup.

## Install

```bash
npm install db2-node
```

You can also install the npm-packed artifact from a GitHub release:

```bash
npm install https://github.com/db2-node/db2-node/releases/download/v1.0.9/db2-node-1.0.9.tgz
```

Replace `v1.0.9` and `1.0.9` with the release version you want.

Prebuilt native binaries ship for supported platforms — no Rust toolchain needed:

- macOS: `x64`, `arm64`
- Linux: `x64` glibc, `x64` musl, `arm64` glibc, `arm64` musl
- Windows: `x64`, `arm64`

## Quick Start

```ts
import { Client } from 'db2-node'

const client = new Client({
  host: 'localhost',
  port: 50000,
  database: 'testdb',
  user: 'db2inst1',
  password: 'secret',
})

await client.connect()

const result = await client.query(
  'SELECT id, name FROM employees WHERE dept_id = ?',
  [1],
)
console.log(result.rows)

await client.close()
```

CommonJS also works:

```js
const { Client } = require('db2-node')
```

## `ibm_db` Compatibility

The CommonJS entry point also supports the common `ibm_db` shapes:

```js
const ibmdb = require('db2-node')

const connStr = 'DATABASE=testdb;HOSTNAME=localhost;PORT=50000;PROTOCOL=TCPIP;UID=db2inst1;PWD=secret'

const conn = await ibmdb.open(connStr)
const rows = await conn.query('SELECT 1 AS V FROM SYSIBM.SYSDUMMY1')
await conn.close()

const db = ibmdb()
await db.open(connStr)
await db.close()

const pool = new ibmdb.Pool()
const pooled = await pool.open(connStr)
await pooled.close()
await pool.close()
```

`new ibmdb.Pool()` does not require connection config; use `pool.open(connStr)`,
`pool.init(size, connStr)`, or `pool.initAsync(size, connStr)` in migration code.
The modern `new Pool({ host, database, user, password })` form remains available,
and the native constructor is exported as `Db2Pool`.

## Data Type Support

The Rust driver covers Db2 for z/OS built-in scalar families: integer, floating point, `DECIMAL`/`NUMERIC`, `DECFLOAT`, character, graphic, binary, `BLOB`, `CLOB`, `DBCLOB`, date/time/timestamp, `ROWID`, `XML`, Boolean, nullable values, and distinct types through their source representation.

JavaScript result values use these mappings:

| Db2 type family | JavaScript value |
|-----------------|------------------|
| `SMALLINT`, `INTEGER`, `BIGINT`, floating point | `number` |
| `DECIMAL`, `NUMERIC`, `DECFLOAT` | `string` |
| `CHAR`, `VARCHAR`, `GRAPHIC`, `VARGRAPHIC`, `CLOB`, `DBCLOB`, `DATE`, `TIME`, `TIMESTAMP`, `ROWID`, `XML` | `string` |
| `BINARY`, `VARBINARY`, `BLOB` | `Buffer` |
| `BOOLEAN` | `boolean` |
| nullable columns | `null` |

For exact parameter typing, cast placeholders in SQL:

```ts
await client.query('VALUES CAST(? AS DECFLOAT(34))', ['123456789.00001'])
await client.query('INSERT INTO files (payload) VALUES (CAST(? AS BLOB(1M)))', [
  Buffer.from([0xde, 0xad, 0xbe, 0xef]),
])
```

## Connection Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `host` | `string` | — | DB2 server hostname |
| `port` | `number` | `50000` | DB2 server port |
| `database` | `string` | — | Database name |
| `user` | `string` | — | Username |
| `password` | `string` | — | Password |
| `securityMechanism` | `string` | `'encrypted'` | DRDA authentication mechanism: `'encrypted'` (SECMEC 9), `'encryptedPassword'` (SECMEC 7), `'userPassword'` (SECMEC 3), or `'userOnly'` (SECMEC 4) |
| `encryptionAlgorithm` | `string` | `'aes'` | DRDA encrypted credential algorithm: `'aes'` or `'des'` |
| `credentialEncoding` | `string` | `'auto'` | Credential string encoding for `SECCHK`: `'auto'`, `'utf8'`, or `'ebcdic'` |
| `encryptedPasswordEncoding` | `string` | `'same'` | SECMEC 7 DES encrypted password plaintext encoding: `'same'`, `'utf8'`, or `'ebcdic'`; AES uses UTF-8/source CCSID |
| `encryptedPasswordTokenEncoding` | `string` | `'same'` | SECMEC 7 DES password IV/token encoding, based on the user ID: `'same'`, `'utf8'`, or `'ebcdic'`; AES uses the server security token |
| `ssl` | `boolean` | `false` | Enable TLS/SSL |
| `rejectUnauthorized` | `boolean` | `true` | Verify server certificate (requires `ssl: true`) |
| `sslClientHostnameValidation` | `string` | `'Basic'` | IBM-compatible hostname validation mode: `'Basic'` or `'OFF'` |
| `caCert` | `string` | — | Path to CA certificate PEM file |
| `connectTimeout` | `number` | `30000` | Connection timeout in ms (covers TCP + TLS handshake) |
| `queryTimeout` | `number` | `0` | Query execution timeout in ms (0 = no timeout) |
| `frameDrainTimeout` | `number` | `25` | Time in ms to wait for follow-up DRDA reply frames |
| `currentSchema` | `string` | — | Default schema for unqualified table names |
| `typeDefinitionName` | `string` | `'QTDSQLASC'` | Optional DRDA data representation for ACCRDB: `'QTDSQLASC'`, `'QTDSQL370'`, `'QTDSQLX86'`, `'QTDSQL400'`, or `'none'` to omit |
| `fetchSize` | `number` | `100` | Rows fetched per network round-trip |

## Pool

```ts
import { Pool } from 'db2-node'

const pool = new Pool({
  host: 'localhost',
  port: 50000,
  database: 'testdb',
  user: 'db2inst1',
  password: 'secret',
  maxConnections: 20,
})

// Warm the first TCP/TLS/DRDA session before timed or user-facing queries.
await pool.connect()

// Simple: pool manages the connection lifecycle
const result = await pool.query('SELECT COUNT(*) AS cnt FROM employees')

// Manual: acquire, use, release
const client = await pool.acquire()
try {
  await client.query('VALUES 1')
} finally {
  await pool.release(client)
}

// Monitor pool state
console.log(await pool.idleCount())    // idle connections
console.log(await pool.activeCount())  // checked-out connections
console.log(pool.maxConnections())     // configured max

await pool.close()
```

### Pool Options

All connection options above, plus:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `minConnections` | `number` | `0` | Minimum idle connections |
| `maxConnections` | `number` | `10` | Maximum total connections |
| `idleTimeout` | `number` | `600` | Close idle connections after this many seconds |
| `maxLifetime` | `number` | `3600` | Recycle connections after this many seconds |
| `healthCheckInterval` | `number` | `30` | Reuse an idle connection without a health-check round trip for this many seconds |

`new Pool(config)` only builds local pool state and does not block on the network.
Call `await pool.connect()` or `await pool.warmup()` during application startup to
open the initial connection before measuring query latency. Without warmup, the
first `pool.query()` includes TCP connect, TLS handshake, and DRDA authentication
inside pool acquire time; subsequent queries reuse the idle connection.

## Prepared Statements

```ts
const stmt = await client.prepare('INSERT INTO logs (msg) VALUES (?)')
await stmt.execute(['first message'])
await stmt.execute(['second message'])

// Batch insert (single round-trip for many rows)
await stmt.executeBatch([['row 1'], ['row 2'], ['row 3']])

await stmt.close()  // always close when done
```

## Transactions

```ts
const tx = await client.beginTransaction()
try {
  await tx.query('UPDATE accounts SET balance = balance - 100 WHERE id = ?', [1])
  await tx.query('UPDATE accounts SET balance = balance + 100 WHERE id = ?', [2])
  await tx.commit()
} catch (e) {
  await tx.rollback()
  throw e
}
```

Transactions also support `tx.prepare()` for prepared statements within a transaction.

## TLS / SSL

```ts
// Trust any certificate (development)
const client = new Client({
  host: 'localhost',
  port: 50001,
  database: 'testdb',
  user: 'db2inst1',
  password: 'secret',
  ssl: true,
  rejectUnauthorized: false,
})

// Verify with custom CA (production)
const client = new Client({
  host: 'db2.example.com',
  port: 50001,
  database: 'proddb',
  user: 'app_user',
  password: 'secret',
  ssl: true,
  rejectUnauthorized: true,
  caCert: '/path/to/ca-cert.pem',
})

// IBM CLI-compatible certificate pinning without hostname validation
const client = new Client({
  host: 'db2-vip.example.com',
  port: 50001,
  database: 'proddb',
  user: 'app_user',
  password: 'secret',
  ssl: true,
  rejectUnauthorized: true,
  caCert: '/path/to/server-or-ca-cert.pem',
  sslClientHostnameValidation: 'OFF',
})
```

TLS uses `rustls` (pure Rust, no OpenSSL). System trust store certificates are loaded automatically when `rejectUnauthorized` is `true`. Custom CA/server certificate files can be supplied with `caCert`, or with the IBM CLI connection string keyword `SSLServerCertificate`.

By default hostname validation is enabled (`sslClientHostnameValidation: 'Basic'`). For IBM `ibm_db` connection string compatibility, `SSLServerCertificate=/path/to/cert.pem` defaults hostname validation to `OFF` unless `SSLClientHostnameValidation=Basic` is supplied explicitly. Object-style `caCert` config remains strict by default; set `sslClientHostnameValidation: 'OFF'` only when the DB2 server certificate is trusted but does not contain a matching DNS/IP subjectAltName. This keeps certificate-chain verification enabled while skipping only the hostname/SAN check. The `connectTimeout` covers the full TCP + TLS handshake.

## DB2 z/OS Authentication

The default `securityMechanism` is `'encrypted'`, which sends DRDA encrypted user ID and password credentials (`SECMEC 9`). Some DB2 z/OS DDF environments require user ID and password authentication (`SECMEC 3`) over an already encrypted TLS connection:

```ts
const client = new Client({
  host: 'zos.example.com',
  port: 448,
  database: 'DSNLOC',
  user: 'APPUSER',
  password: 'secret',
  ssl: true,
  rejectUnauthorized: true,
  securityMechanism: 'userPassword',
})
```

Use `securityMechanism: 'userPassword'` only with TLS in production, because DRDA itself will not encrypt the credentials for that mechanism. RACF user IDs are commonly uppercase and limited to 8 characters; pass the exact user ID form accepted by DDF. A `SECCHKCD 0x13` failure means DB2 rejected the user ID or password after the security check.

If your JDBC tool is configured with `securityMechanism=7` / `ENCRYPTED_PASSWORD_SECURITY`, use `securityMechanism: 'encryptedPassword'`. That sends the user ID in clear text and encrypts only the password, matching the IBM JCC `SECMEC 7` flow. Credential bytes default to `credentialEncoding: 'auto'`, which follows the server's negotiated Unicode manager; set `credentialEncoding: 'utf8'` or `'ebcdic'` to override it while diagnosing z/OS authentication.

Some z/OS configurations accept the clear user ID in one encoding while expecting the encrypted password bytes in another. For SECMEC 7 DES diagnostics, `encryptedPasswordEncoding` controls the password plaintext bytes before encryption and `encryptedPasswordTokenEncoding` controls the user-ID-derived IV/token bytes. AES follows IBM JCC: the password plaintext uses UTF-8/source CCSID and the IV uses the server security token.

If the server is configured for AES encrypted authentication, set `encryptionAlgorithm: 'aes'`. This advertises DRDA Security Manager level 9 and sends `ENCALG=AES` / `ENCKEYLEN=256` during `ACCSEC`:

```ts
const client = new Client({
  host: 'zos.example.com',
  port: 446,
  database: 'DSNLOC',
  user: 'APPUSER',
  password: 'secret',
  securityMechanism: 'encrypted',
  encryptionAlgorithm: 'aes',
  credentialEncoding: 'utf8',
})
```

For Db2 for z/OS, IBM's current JDBC driver defaults to encrypted user ID and encrypted password security (`SECMEC 9`) with AES when ICSF is enabled on the subsystem. Use `securityMechanism: 'encryptedPassword'` only when your working JDBC configuration explicitly sets `securityMechanism=7`.

## Db2 z/OS LOB Production Modes

Db2 for z/OS can continue sending external LOB data (`EXTDTA`) after the rows requested by an application have already been materialized. The driver treats that tail data conservatively so a pooled connection is never returned while stale LOB frames can corrupt the next query.

### Recommended default

Run normally:

```bash
node app.js
```

Do not set z/OS LOB cleanup environment variables for the default production path. If a z/OS LOB query cannot be proven clean after materialization, the driver disconnects that socket and warm-replaces it in the pool. This is the fastest mode for large CLOB result sets where preserving the exact same server session is not required.

For aggregate or scalar results that filter CLOB-like text with `LIKE` or `NOT LIKE`, the driver uses a large statement package `EXCSQLSTT` path by default on Db2 for z/OS. This keeps CLOB predicate scans away from the one-shot cursor package path that can hit package-specific resource limits. Set `DB2_ZOS_LIKE_PREDICATE_EXCSQLSTT=0` only when diagnosing package behavior.

Expected default diagnostics, when `DB2_QUERY_DIAGNOSTICS=1` is enabled for troubleshooting:

```text
zos_lob_disconnect_after_materialization=true
```

### Active close mode

Enable active close when connection/session preservation is more important than individual large-LOB query latency:

```bash
DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE=1 node app.js
```

In this mode the driver sends `CLSQRY` with the learned z/OS query instance identifier, drains remaining `EXTDTA`, waits for DB2's close acknowledgement, and only then returns the connection to the pool.

Expected active-close diagnostics:

```text
cursor_lob_materialized_tail skipped=active_close
cursor_lob_materialized_close ... verified=true ... close_reply_seen=true
zos_lob_cleanup_verified=true close_after_materialize=true
```

Active close keeps the same connection reusable, but it can add latency to large LOB queries because the driver reads and discards the remaining LOB tail inline.

### Passive quiet trust

Keep this off in production:

```bash
DB2_ZOS_LOB_TRUST_PASSIVE_TAIL_QUIET=0
```

The passive quiet path is fail-closed, but it is not a useful optimization for large z/OS CLOB workloads because any observed tail `EXTDTA` rejects reuse and falls back to safe cleanup.

### Production validation

The production soak for the current `1.0.x` release line passed:

| Mode | Cycles | Result |
|------|--------|--------|
| Default disconnect + warm replacement | 100/100 | Passed |
| Active close | 50/50 | Passed |

Across both soaks there were no wrong row counts, zero-row corruption, stale `EXTDTA`, or unhandled driver errors.

## Server Info

```ts
await client.connect()
const info = await client.serverInfo()
console.log(info.productName)    // e.g. "DB2/LINUXX8664"
console.log(info.serverRelease)  // e.g. "11.05.0900"
```

## Error Handling

```ts
try {
  await client.query('INVALID SQL')
} catch (err) {
  // err.message includes SQLSTATE and SQLCODE
  // e.g. "SQL Error [SQLSTATE=42601, SQLCODE=-104]: ..."
}
```

## Releases

Tag pushes matching `v*` trigger the release workflow in `.github/workflows/release.yml`, which builds native binaries for all targets, publishes to npm, and attaches the npm-packed `.tgz` to the GitHub release for direct GitHub installs.
