# API Reference

Complete TypeScript API reference for `db2-node`.

## Types

### ConnectionConfig

```typescript
interface ConnectionConfig {
  host: string;
  port?: number;                  // default: 50000
  database: string;
  user: string;
  password: string;
  securityMechanism?: 'encrypted' | 'encryptedPassword' | 'userPassword' | 'userOnly';
  encryptionAlgorithm?: 'aes' | 'des';
  credentialEncoding?: 'auto' | 'utf8' | 'ebcdic500' | 'ebcdic037' | 'ebcdic';
  encryptedPasswordEncoding?: 'same' | 'utf8' | 'ebcdic500' | 'ebcdic037' | 'ebcdic';
  encryptedPasswordTokenEncoding?: 'same' | 'utf8' | 'ebcdic500' | 'ebcdic037' | 'ebcdic';
  ssl?: boolean;                  // default: false
  rejectUnauthorized?: boolean;   // default: true (verify server cert)
  sslClientHostnameValidation?: 'Basic' | 'OFF'; // default: 'Basic'
  caCert?: string;                // path to CA certificate PEM file
  connectTimeout?: number;        // ms, default: 30000 (covers TCP + TLS)
  queryTimeout?: number;          // ms, default: 0 (no timeout)
  frameDrainTimeout?: number;     // ms, default: 25
  currentSchema?: string;
  typeDefinitionName?: 'QTDSQLASC' | 'QTDSQL370' | 'QTDSQLX86' | 'QTDSQL400' | 'none';
  fetchSize?: number;             // rows per fetch, default: 100
}
```

LUW uses `QTDSQLX86`; supplying another explicit `typeDefinitionName` raises an error during connect. Omit the option or use `QTDSQLX86`. z/OS keeps its existing configurable type definitions.

The default encrypted mechanism refuses plaintext fallback without TLS. For a stock Docker server configured with `AUTHENTICATION=SERVER`, explicitly choose `securityMechanism: 'userPassword'`, enable TLS, or configure encrypted authentication on the server.

`credentialEncoding: 'auto'` uses UTF-8 when `UNICODEMGR=1208` is negotiated, otherwise CCSID 500. For identified Db2 LUW servers, CCSID 500 uses the IBM CLI-compatible ASCII conversion (`|` at `0x6A`); z/OS and unknown peers use standard IBM-500 (`|` at `0xBB`). The explicit `'ebcdic500'` option uses the same server conversion profile. `'ebcdic'` remains an alias for `'ebcdic037'`. SQL/database data CCSIDs do not determine authentication encoding, and an explicit override does not change server negotiation.

### PoolConfig

Extends all `ConnectionConfig` options, plus:

```typescript
interface PoolConfig extends ConnectionConfig {
  minConnections?: number;    // default: 0
  maxConnections?: number;    // default: 10
  idleTimeout?: number;       // seconds, default: 600 (10 min)
  maxLifetime?: number;       // seconds, default: 3600 (1 hour)
  healthCheckInterval?: number; // seconds, default: 30
}
```

### QueryResult

```typescript
type RowMode = 'object' | 'array';
interface QueryOptions<M extends RowMode = RowMode> { rowMode?: M; }
interface QueryResult<M extends RowMode = 'object'> {
  rows: (M extends 'array' ? any[] : Record<string, any>)[];
  rowCount: number;
  columns: ColumnInfo[];
  diagnostics: string[];
  resultSets: QueryResult<M>[];       // CALL result sets; rows/columns describe the first
  outputParameters: any[];        // OUT/INOUT only, in parameter order
}
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
rows, where the last value wins for duplicate names.

`Pool`/`CompatPool` callbacks also accept `query(sql, params, options, callback)`.
For the `ibm_db`-style `Database.query`, `queryResult`, and `queryStream`, use the SQL
object form `{ sql, params, rowMode: 'array' }`.

### ColumnInfo

```typescript
interface ColumnInfo {
  name: string;
  typeName: string;
  db2TypeName?: string;
  nullable: boolean;
  precision?: number;
  scale?: number;
}
```

`typeName` is the JavaScript-facing type name. `db2TypeName` is present when the underlying Db2 type needs to be preserved separately, for example `GRAPHIC(10)` exposed as public `CHAR(10)`.

### ServerInfo

```typescript
interface ServerInfo {
  productName: string;      // e.g. "DB2/LINUXX8664"
  serverRelease: string;    // e.g. "11.05.0900"
}
```

---

## Client

The `Client` class manages a single connection to a DB2 database.

### Constructor

```typescript
new Client(config: ConnectionConfig)
```

Creates a new client instance. Does not connect immediately — call `connect()` to establish the connection.

### client.connect()

```typescript
connect(): Promise<void>
```

Establishes a TCP connection (with optional TLS upgrade) and performs the DRDA authentication handshake (EXCSAT, ACCSEC, SECCHK, ACCRDB).

The `connectTimeout` covers the entire process: TCP connect + TLS handshake.

**Throws**: `Error` if connection fails (timeout, network error, authentication failure, database not found, TLS handshake failure).

### client.cancel()

```typescript
cancel(): Promise<boolean>
```

Cancels running LUW activity using an independent control session, so it can run while `query()` is awaiting a result. Returns `true` when an activity was cancelled and `false` when no activity was found. The interrupted query rejects with the server cancellation error; the connection remains usable.

The account needs activity-monitoring privileges and permission to execute `SYSPROC.WLM_CANCEL_ACTIVITY`. Control work is bounded to five seconds. z/OS cancellation is not implemented. `queryTimeout` attempts this cancellation, then closes the timed-out connection; its error states whether cancellation succeeded or failed.

### TLS Hostname Validation

`sslClientHostnameValidation` follows the IBM Db2 CLI keyword values:

- `'Basic'` (default) verifies the certificate chain and checks that the connected host matches the certificate DNS/IP subjectAltName.
- `'OFF'` verifies the certificate chain but skips the hostname/SAN check. This matches IBM CLI connection strings that use `SSLServerCertificate=...;SSLClientHostnameValidation=OFF`.

For IBM `ibm_db` connection string compatibility, `SSLServerCertificate=/path/to/cert.pem` defaults hostname validation to `OFF` unless `SSLClientHostnameValidation=Basic` is supplied explicitly. Object-style `caCert` config remains strict by default.

Use `'OFF'` only when the DB2 certificate is trusted but was issued without a hostname that matches the DB2 VIP or load balancer name.

### client.query()

```typescript
query<M extends RowMode = 'object'>(sql: string, params?: any[] | null, options?: QueryOptions<M> | null): Promise<QueryResult<M>>
```

Executes a SQL statement and returns the result.

- For `SELECT` statements: returns rows in `result.rows`
- For `INSERT`/`UPDATE`/`DELETE`: returns affected row count in `result.rowCount`
- For DDL (`CREATE`/`DROP`/`ALTER`): returns empty result

**Parameters**:
- `sql` — SQL statement. Use `?` for parameter placeholders.
- `params` — Optional array of parameter values.

Parameter values can be `string`, `number`, `boolean`, `null`, `Buffer`, `Uint8Array`, number arrays, or values that the server can cast from those representations. For exact `DECIMAL`, `DECFLOAT`, date/time, XML, and LOB typing, cast parameter markers in SQL.

**Example**:
```typescript
// Simple query
const result = await client.query('SELECT * FROM employees');

// Parameterized query
const result = await client.query(
  'SELECT * FROM employees WHERE dept_id = ? AND salary > ?',
  [1, 100000]
);

// Binary / BLOB parameter
await client.query(
  'INSERT INTO files (payload) VALUES (CAST(? AS BLOB(1M)))',
  [Buffer.from([0xde, 0xad, 0xbe, 0xef])]
);
```

See [Data Type Support](../data-types/index.md) for the full Db2 for z/OS type mapping.

### client.prepare()

```typescript
prepare(sql: string): Promise<PreparedStatement>
```

Prepares a SQL statement for repeated execution. Each prepared statement gets a dedicated server-side section, allowing up to 385 concurrent prepared statements per connection.

**Example**:
```typescript
const stmt = await client.prepare('INSERT INTO logs (msg) VALUES (?)');
await stmt.execute(['first message']);
await stmt.execute(['second message']);
await stmt.close(); // always close to release server resources
```

### client.beginTransaction()

```typescript
beginTransaction(): Promise<Transaction>
```

Starts a new transaction. The connection enters manual commit mode. Returns a `Transaction` handle for executing queries, preparing statements, committing, or rolling back.

### client.serverInfo()

```typescript
serverInfo(): Promise<ServerInfo>
```

Returns information about the connected DB2 server (product name and release level), populated during the initial connection handshake.

### client.close()

```typescript
close(): Promise<void>
```

Closes the current connection to the DB2 server. The same `Client` instance can be connected again later by calling `connect()` explicitly.

---

## Pool

The `Pool` class manages reusable DB2 connections for concurrent access. It
supports both the modern config-object constructor and the CommonJS `ibm_db`
compatibility shape.

### Constructor

```typescript
new Pool(config?: PoolConfig | IbmDbPoolOptions)
```

When `config` contains connection settings such as `host`, `database`, `user`,
and `password`, `Pool` creates a native `db2-node` pool. Connections are created
lazily on first use, and the pool uses a semaphore to enforce `maxConnections`.

When called as `new Pool()` or with pool-only options such as `maxPoolSize`,
it behaves like `ibm_db.Pool`: call `pool.open(connectionString)`,
`pool.init(size, connectionString)`, or `pool.initAsync(size, connectionString)`
to initialize or acquire `Database` handles.

The explicit native-style constructor is also exported as `Db2Pool`.

### pool.query()

```typescript
query<M extends RowMode = 'object'>(sql: string, params?: any[] | null, options?: QueryOptions<M> | null): Promise<QueryResult<M>>
```

Acquires a connection, executes the query, and releases the connection back — all in one call. This is the simplest way to run queries with pooling.

### pool.acquire()

```typescript
acquire(): Promise<Client>
```

Acquires a connection from the pool. If all connections are in use and `maxConnections` is reached, this call waits until one is returned.

The caller **must** release the connection with `pool.release()` when done.

### pool.release()

```typescript
release(client: Client): Promise<void>
```

Returns a connection to the pool. The pool checks the connection's health and lifetime before making it available for reuse.

### pool.close()

```typescript
close(): Promise<void>
```

Closes the pool. Waits up to 5 seconds for in-flight connections to be returned, then closes all idle connections.

### pool.idleCount()

```typescript
idleCount(): Promise<number>
```

Returns the number of idle connections currently sitting in the pool.

### pool.activeCount()

```typescript
activeCount(): Promise<number>
```

Returns the number of connections currently checked out (in use).

### pool.totalCount()

```typescript
totalCount(): Promise<number>
```

Returns the total number of connections (idle + active).

### pool.maxConnections()

```typescript
maxConnections(): number
```

Returns the configured maximum number of connections.

### pool.open()

```typescript
open(connectionString: string | Record<string, any>): Promise<Database>
```

`ibm_db` compatibility API. Opens or reuses a pooled `Database` handle for the
given IBM-style connection string or connection object. Callback style is also
supported.

### pool.init() / pool.initAsync()

```typescript
init(size: number, connectionString: string | Record<string, any>): boolean
initAsync(size: number, connectionString: string | Record<string, any>): Promise<void>
```

Initializes a compatibility pool with a fixed max size for a connection string.

---

## PreparedStatement

A prepared SQL statement that can be executed multiple times with different parameters. Each prepared statement holds a dedicated server-side section; always close when done to free it.

### stmt.execute()

```typescript
execute<M extends RowMode = 'object'>(params?: any[] | null, options?: QueryOptions<M> | null): Promise<QueryResult<M>>
```

Executes the prepared statement with the given parameters.

### stmt.executeBatch()

```typescript
executeBatch<M extends RowMode = 'object'>(paramRows: any[][], options?: QueryOptions<M> | null): Promise<QueryResult<M>>
```

Executes the prepared statement as a batch with multiple rows of parameters. Each element of `paramRows` is an array of parameter values for one row. Uses a single network round-trip for efficiency.

**Example**:
```typescript
const stmt = await client.prepare('INSERT INTO items (name, qty) VALUES (?, ?)');
await stmt.executeBatch([
  ['Widget', 10],
  ['Gadget', 25],
  ['Sprocket', 5],
]);
await stmt.close();
```

### stmt.close()

```typescript
close(): Promise<void>
```

Closes the prepared statement and releases the server-side section back to the connection's section pool. Always close prepared statements when done.

---

## Transaction

A database transaction with manual commit/rollback control. If a transaction is dropped without committing or rolling back, it is automatically rolled back.

### tx.query()

```typescript
query<M extends RowMode = 'object'>(sql: string, params?: any[] | null, options?: QueryOptions<M> | null): Promise<QueryResult<M>>
```

Executes a SQL statement within the transaction.

### tx.prepare()

```typescript
prepare(sql: string): Promise<PreparedStatement>
```

Prepares a SQL statement within this transaction. The prepared statement executes in the transaction's context (manual commit mode).

### tx.commit()

```typescript
commit(): Promise<void>
```

Commits all changes made within the transaction.

### tx.rollback()

```typescript
rollback(): Promise<void>
```

Rolls back all changes made within the transaction.

---

## Type Mapping

### DB2 to JavaScript

| DB2 Type | JavaScript Type | Notes |
|----------|----------------|-------|
| `SMALLINT` | `number` | 16-bit integer |
| `INTEGER` | `number` | 32-bit integer |
| `BIGINT` | `number` or `string` | Safe integers remain numbers; larger magnitudes are exact decimal strings |
| `REAL` | `number` | 32-bit float |
| `DOUBLE` | `number` | 64-bit float |
| `DECIMAL` | `string` | Preserves exact precision |
| `NUMERIC` | `string` | Preserves exact precision |
| `DECFLOAT` | `string` | Returned as string |
| `CHAR` | `string` | Fixed-length, right-trimmed |
| `VARCHAR` | `string` | Variable-length |
| `CLOB` | `string` | Character LOB |
| `DATE` | `string` | `YYYY-MM-DD` format |
| `TIME` | `string` | `HH:MM:SS` format |
| `TIMESTAMP` | `string` | ISO 8601 format |
| `BOOLEAN` | `boolean` | Native JavaScript boolean |

### Parameter Type Inference

When passing parameters, JavaScript types are automatically mapped:

| JavaScript Type | DB2 Type |
|----------------|----------|
| `number` (fits in 32 bits) | `INTEGER` |
| `bigint` | Exact decimal text; BIGINT when described by the server |
| `Date` | UTC timestamp text |
| `number` (large or float) | `DOUBLE` |
| `string` | `VARCHAR` |
| `boolean` | `BOOLEAN` |
| `null` | `NULL` |
| `Array<number>` | `BINARY` (byte array) |

---


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

---

## Error Handling

All async methods throw on failure. SQL errors include SQLSTATE and SQLCODE in the message:

```typescript
try {
  await client.query('INVALID SQL');
} catch (err) {
  console.error(err.message);
  // "SQL Error [SQLSTATE=42601, SQLCODE=-104]: An unexpected token..."
}
```


Client-side binding and protocol errors expose a stable `driverCode` on the Error
object, including through `Js*`/`Native*` APIs and callbacks:

| `driverCode` | Meaning |
|--------------|---------|
| `DB2_PARAMETER_COUNT` | Parameter count differs from the input descriptor count. |
| `DB2_PARAMETER_TYPE` | A parameter cannot be converted or encoded for its target type, including invalid numeric text/range, unsupported JS values, or invalid byte arrays. |
| `DB2_PROTOCOL` | A malformed or unexpected DRDA reply or protocol decoding failure. |
| `DB2_INVALID_OPTION` | An unsupported `rowMode`. |

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
`'GenericFailure'`), and messages retain their existing content. Client-side
errors do not invent `sqlstate` or `sqlcode` values. Server SQL errors retain their
SQLSTATE, SQLCODE, and retry behavior and have no `driverCode`, even if the server
reports a parameter mistake. Wrapper APIs expose server `sqlstate`, `sqlcode`,
and `retryable` properties as before; raw native APIs retain the server diagnostic
message. `Db2Error` and `DriverErrorCode` provide the public TypeScript types.

### Common Error Patterns

| Error Type | Cause |
|-----------|-------|
| Connection timeout | `connectTimeout` exceeded (TCP or TLS handshake) |
| Query timeout | `queryTimeout` exceeded during execution |
| Authentication failure | Wrong username/password (SQLSTATE 28000) |
| SQL syntax error | Invalid SQL (SQLSTATE 42601) |
| Object not found | Table/view doesn't exist (SQLSTATE 42704) |
| Unique violation | Duplicate key (SQLSTATE 23505) |
| Pool exhaustion | All connections in use; `acquire()` waits until one is freed |
| TLS handshake failure | Certificate verification failed or wrong port |
