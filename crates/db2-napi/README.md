# db2-node

Pure Rust DB2 driver for Node.js using the DRDA wire protocol directly. No IBM CLI, ODBC, or `libdb2` dependency is required at runtime.

## Status

`1.0.26` is the current release. It writes LOB and XML parameters larger than 32,767 bytes (#31), passes binary parameters without per-byte conversion, and completes Db2 LUW replies at the end of the DRDA reply chain instead of waiting on fixed timers. Validation covers Docker Db2 12.1 and 11.5, with verified TLS on 12.1. The package supports parameterized queries, prepared statements, transactions, connection pooling, TLS, `ibm_db` compatibility entry points, and the existing Db2 for z/OS paths. No live z/OS server was tested for this release.

## Install

```bash
npm install db2-node
```

You can also install the npm-packed artifact from a GitHub release:

```bash
npm install https://github.com/gurungabit/db2-node/releases/download/v1.0.26/db2-node-1.0.26.tgz
```

Replace `v1.0.26` and `1.0.26` with the release version you want.

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

### Array rows

Use `{ rowMode: 'array' }` per call to retain duplicate column names:

```js
const result = await client.query(
  'SELECT 1 AS A, 2 AS A FROM SYSIBM.SYSDUMMY1',
  [],
  { rowMode: 'array' }
)
// result.rows: [[1, 2]]
// result.columns.map(column => column.name): ['A', 'A']
```

`rows[i][j]` corresponds to `columns[j]` in SELECT order. This also applies to every
CALL `resultSets` entry. Value conversions and column metadata are the same as for
object rows; OUT/INOUT values remain in `outputParameters` in parameter order.

The optional final argument is supported by `Client.query`, `Pool.query`,
`Db2Pool.query`, `Transaction.query`, `PreparedStatement.execute(params, options)`,
and `PreparedStatement.executeBatch(paramRows, options)`, including the `Js*` and
`Native*` classes and clients acquired from a pool. Pass `[]`, `undefined`, or
`null` for omitted parameters when supplying options. TypeScript infers
`QueryResult<'array'>`; `QueryOptions`, `RowMode`, and generic `QueryResult` are
exported. Omitted options and `{ rowMode: 'object' }` return the existing object
rows, where the last value wins for duplicate names. `QueryOptions<'array'>`
requires `rowMode: 'array'`; an empty options object cannot select array rows. A
dynamic or optional `rowMode` returns `QueryResult<RowMode>`, a union of object
and array results, so callers must handle both shapes.

`Pool`/`CompatPool` callbacks also accept `query(sql, params, options, callback)`.
For the `ibm_db`-style `Database.query`, `queryResult`, and `queryStream`, use the SQL
object form `{ sql, params, rowMode: 'array' }`.

`queryStream` returns an object-mode Node.js Readable. Its self-contained
`RowStream` type covers row-aware async iteration, `read()` and `'data'` events,
error/lifecycle listeners, piping, and pause/resume/destroy controls without
requiring `@types/node`. Default rows are objects; an explicit array mode yields
arrays, and dynamic modes expose both shapes. Consumers with Node typings can
pipe to an object-mode `Writable`, use `stream/promises.pipeline`, or use
`Readable.from(rows)` when an API requires the full Node `Readable` declaration.

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
| `SMALLINT`, `INTEGER`, floating point | `number` |
| `BIGINT` | `number` within JavaScript’s safe integer range; decimal `string` outside it |
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
| `credentialEncoding` | `string` | `'auto'` | `SECCHK` encoding: UTF-8 if `UNICODEMGR=1208` is negotiated, otherwise CCSID 500. Overrides: `'utf8'`, `'ebcdic500'`, or `'ebcdic037'` (also `'ebcdic'`) |
| `encryptedPasswordEncoding` | `string` | `'same'` | SECMEC 7 encrypted password plaintext encoding: `'same'`, `'utf8'`, `'ebcdic500'`, or `'ebcdic037'` (also `'ebcdic'`); AES defaults to the negotiated credential encoding on LUW and UTF-8/source CCSID on z/OS |
| `encryptedPasswordTokenEncoding` | `string` | `'same'` | SECMEC 7 DES password IV/token encoding, based on the user ID: `'same'`, `'utf8'`, `'ebcdic500'`, or `'ebcdic037'` (also `'ebcdic'`); AES uses the server security token |
| `ssl` | `boolean` | `false` | Enable TLS/SSL |
| `rejectUnauthorized` | `boolean` | `true` | Verify server certificate (requires `ssl: true`) |
| `sslClientHostnameValidation` | `string` | `'Basic'` | IBM-compatible hostname validation mode: `'Basic'` or `'OFF'` |
| `caCert` | `string` | — | Path to CA certificate PEM file |
| `connectTimeout` | `number` | `30000` | Connection timeout in ms (covers TCP + TLS handshake) |
| `queryTimeout` | `number` | `0` | Query execution timeout in ms (0 = no timeout); attempts LUW server cancellation before closing the connection |
| `frameDrainTimeout` | `number` | `25` | Db2 for z/OS only: time in ms to wait for follow-up DRDA reply frames. Other servers mark the end of each reply, so no wait is needed |
| `currentSchema` | `string` | — | Default schema for unqualified table names |
| `typeDefinitionName` | `string` | Server-dependent | LUW supports omitted or `'QTDSQLX86'`; other explicit values fail. z/OS supports `'QTDSQLASC'` (default), `'QTDSQL370'`, `'QTDSQLX86'`, `'QTDSQL400'`, or `'none'` |
| `fetchSize` | `number` | `100` | Rows fetched per network round-trip |

The default encrypted authentication refuses a non-TLS downgrade to plaintext. A stock Docker server with `AUTHENTICATION=SERVER` requires an explicit `securityMechanism: 'userPassword'`, TLS, or server configuration enabling encrypted authentication. AES and DES are supported with `SERVER_ENCRYPT`; `AES_ONLY` rejects DES explicitly.

## Parameters, Procedures and Cancellation

Parameters accept `bigint`, decimal strings for BIGINT, UTC `Date` values, `Buffer`, `Uint8Array`, `ArrayBuffer`, numbers, strings, booleans and null. A `Date` becomes UTC timestamp text; SQL casts select the desired Db2 type. Unsafe BIGINT results are decimal strings, so `9223372036854775807` stays exact. Byte arrays must contain only integer values from 0 through 255; nested arrays and objects raise catchable errors. DECIMAL parameters reject malformed text and integer overflow before sending data to Db2.

A `CALL` result exposes `resultSets` and `outputParameters`. `rows`, `columns` and `rowCount` describe the first result set. `outputParameters` contains OUT and INOUT values in parameter order; IN-only parameters are omitted. These fields are also available on compatibility `queryResult()` results, and callback forms receive OUT values as their third argument.

```ts
const result = await client.query('CALL MY_PROC(?, ?)', [7, null])
console.log(result.resultSets, result.outputParameters)
```

`await client.cancel()` cancels active work on a LUW connection through a separate control connection and returns whether it cancelled an activity. The query rejects with the server cancellation error while the connection remains usable. `queryTimeout` uses the same mechanism, then closes the timed-out connection. Cancellation has a five-second control-operation bound and requires permission to monitor activity and execute `SYSPROC.WLM_CANCEL_ACTIVITY`; failures are reported in the timeout error. z/OS server cancellation is not implemented.

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

// Batch insert (single round-trip for many rows). Outside a transaction it
// commits once all rows succeed; if any row fails, none are kept.
await stmt.executeBatch([['row 1'], ['row 2'], ['row 3']])

await stmt.close()  // always close when done
```


### BOOLEAN parameters

For a BOOLEAN target, string parameters accept these explicit, case-insensitive
Db2 aliases, with leading and trailing ASCII spaces ignored:

| Value | Accepted strings |
|-------|------------------|
| `true` | `'t'`, `'true'`, `'y'`, `'yes'`, `'on'`, `'1'` |
| `false` | `'f'`, `'false'`, `'n'`, `'no'`, `'off'`, `'0'` |

```js
await client.query('VALUES CAST(? AS BOOLEAN)', [' false '])
// rows: [{ '1': false }]
```

Booleans, integers (zero is false, nonzero is true), and NULL keep their existing
behavior. Strings sent to VARCHAR targets remain strings. Other strings,
including tabs/newlines around a token and numeric text such as `'2'`, are
rejected with `driverCode: 'DB2_PARAMETER_TYPE'`. Db2 also accepts broader numeric
text conversions; use `CAST(CAST(? AS VARCHAR(128)) AS BOOLEAN)` to delegate those
to the server. See IBM's [BOOLEAN reference](https://www.ibm.com/docs/en/db2/12.1.x?topic=functions-boolean).

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

If your JDBC tool is configured with `securityMechanism=7` / `ENCRYPTED_PASSWORD_SECURITY`, use `securityMechanism: 'encryptedPassword'`. That sends the user ID in clear text and encrypts only the password, matching the IBM JCC `SECMEC 7` flow. Credential bytes default to `credentialEncoding: 'auto'`: UTF-8 when `UNICODEMGR=1208` is negotiated, otherwise DRDA's default CCSID 500. Database and SQL data CCSIDs do not select the authentication encoding. Use `'utf8'`, `'ebcdic500'`, or `'ebcdic037'` (also `'ebcdic'`) as diagnostic overrides; they do not renegotiate the server's encoding or trigger an authentication retry.

CCSID 500 credentials follow IBM CLI's ASCII-compatible conversion on identified Db2 LUW servers (`|` at `0x6A`). z/OS and unknown peers use standard IBM-500 (`|` at `0xBB`). The `'ebcdic500'` override follows this same server conversion profile; `'ebcdic'` retains its existing CCSID 037 meaning.

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

Do not set z/OS LOB cleanup environment variables for the default production path. The driver uses Db2 for z/OS native LOB fetches first, sends an active `CLSQRY` after LOB materialization when needed, and reuses the connection only after cleanup is verified. If cleanup still cannot be proven, the driver disconnects that socket rather than returning a potentially stale session to the pool.

Native z/OS LOB continuation fetches send `QRYROWSET` with `RTNEXTDTA=RTNEXTALL` so full CLOB reads stay on the native cursor path. The default rowset follows the configured fetch size for throughput; set `DB2_ZOS_NATIVE_LOB_CNTQRY_ROWSET` only if a specific server needs a smaller continuation window. Extended continuation blocks are enabled by default and can be disabled with `DB2_ZOS_NATIVE_LOB_CNTQRY_EXTRA_BLOCKS=0` for diagnostics.

Set `DB2_ZOS_LOB_STRATEGY=sql` only for diagnostics or for servers where native LOB fetches fail. That mode rebuilds CLOB/LOB values through generated `SUBSTR` queries and keeps the generated cursor path conservative.

For aggregate or scalar results that filter CLOB-like text with `LIKE` or `NOT LIKE`, the driver uses a large statement package `EXCSQLSTT` path by default on Db2 for z/OS. This keeps CLOB predicate scans away from the one-shot cursor package path that can hit package-specific resource limits. Set `DB2_ZOS_LIKE_PREDICATE_EXCSQLSTT=0` only when diagnosing package behavior.

### Retry and recovery metadata

Errors returned by `Client`, `Pool`, `PreparedStatement`, `Transaction`, and `ibm_db`-style wrapper APIs include DB2 metadata when it can be inferred from the server message:

```js
try {
  await client.query(sql)
} catch (err) {
  if (err.retryable) {
    // Retry the whole read operation or recreate session-bound resources.
  }
  console.error(err.sqlstate, err.sqlcode)
}
```


Client-side binding and protocol errors expose a stable `driverCode` on the Error
object, including through `Js*`/`Native*` APIs and callbacks:

| `driverCode` | Meaning |
|--------------|---------|
| `DB2_PARAMETER_COUNT` | Parameter count differs from the input descriptor count. |
| `DB2_PARAMETER_TYPE` | A parameter cannot be converted or encoded for its target type, including invalid numeric text/range, unsupported JS values, or invalid byte arrays. |
| `DB2_PROTOCOL` | A malformed or unexpected DRDA reply or protocol decoding failure. |
| `DB2_INVALID_OPTION` | Malformed query options or an unsupported `rowMode` value/type. |

```js
try {
  await client.query('VALUES CAST(? AS BOOLEAN)', ['invalid'])
} catch (err) {
  if (err.driverCode === 'DB2_PARAMETER_TYPE') {
    // Ask the caller to correct the parameter.
  }
}
```

`driverCode` is additive: `code` retains the existing N-API status (usually
`'GenericFailure'`), and existing parameter/server messages retain their content.
Client-side parameter and option errors do not invent `sqlstate` or `sqlcode`
values, and parameter/option text cannot mark an error retryable. Non-string
`rowMode` values retain `StringExpected` with a deterministic validation message. Rejected `NaN`, `Infinity`,
and `-Infinity` parameters retain their original `InvalidArg` code and conversion
message while gaining `DB2_PARAMETER_TYPE`. Recognized protocol session errors
(such as `QRYNOPRM` or a connection closed by the server) retain wrapper
`retryable: true` metadata even when classified as `DB2_PROTOCOL`.
Server SQL errors retain their SQLSTATE, SQLCODE, and retry behavior and have no `driverCode`, even if the server
reports a parameter mistake. Wrapper APIs expose server `sqlstate`, `sqlcode`,
and `retryable` properties as before; raw native APIs retain the server diagnostic
message. `Db2Error` and `DriverErrorCode` provide the public TypeScript types.

For Db2 for z/OS stale cursor/statement state, `SQLCODE=-502`, `SQLCODE=-514`, and `SQLCODE=-518` are marked `retryable: true`. Plain `client.query()` read operations can reconnect and retry internally. Prepared statements and transactions are bound to a specific server session, so after a reconnect you should create a new prepared statement or rerun the entire transaction body from the beginning. The driver does not replay writes or partial transaction work automatically.

Expected default diagnostics, when `DB2_QUERY_DIAGNOSTICS=1` is enabled for troubleshooting:

```text
cursor_lob_materialized_close ... verified=true ... close_reply_seen=true
zos_lob_cleanup_verified=true close_after_materialize=true
```

### Active close mode

Active close is enabled by default. To diagnose the older reconnect-after-materialization mode:

```bash
DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE=0 node app.js
```

In active close mode the driver sends `CLSQRY` with the learned z/OS query instance identifier, drains remaining `EXTDTA`, waits for DB2's close acknowledgement, and only then returns the connection to the pool.

Expected active-close diagnostics:

```text
cursor_lob_materialized_tail skipped=active_close
cursor_lob_materialized_close ... verified=true ... close_reply_seen=true
zos_lob_cleanup_verified=true close_after_materialize=true
```

Active close keeps the same connection reusable. Disabling it can add latency to repeated LOB workloads because the driver may need to reconnect after materialization to avoid reusing a cursor with unverified tail data.

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
