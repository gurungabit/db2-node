---
title: "API Reference"
weight: 30
---

# API Reference

Complete TypeScript API reference for db2-node.

## Types

### ConnectionConfig

```typescript
interface ConnectionConfig {
  host: string;
  port?: number;                  // default: 50000
  database: string;
  user: string;
  password: string;
  ssl?: boolean;                  // default: false
  rejectUnauthorized?: boolean;   // default: true (verify server cert)
  caCert?: string;                // path to CA certificate PEM file
  connectTimeout?: number;        // ms, default: 30000 (covers TCP + TLS)
  queryTimeout?: number;          // ms, default: 0 (no timeout)
  frameDrainTimeout?: number;     // ms, default: 500
  currentSchema?: string;
  fetchSize?: number;             // rows per fetch, default: 100
}
```

### PoolConfig

Extends all `ConnectionConfig` options, plus:

```typescript
interface PoolConfig extends ConnectionConfig {
  minConnections?: number;    // default: 0
  maxConnections?: number;    // default: 10
  idleTimeout?: number;       // seconds, default: 600 (10 min)
  maxLifetime?: number;       // seconds, default: 3600 (1 hour)
}
```

### QueryResult

```typescript
interface QueryResult {
  rows: Record<string, any>[];
  rowCount: number;
  columns: ColumnInfo[];
}
```

### ColumnInfo

```typescript
interface ColumnInfo {
  name: string;
  typeName: string;
  nullable: boolean;
  precision?: number;
  scale?: number;
}
```

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

### client.query()

```typescript
query(sql: string, params?: any[]): Promise<QueryResult>
```

Executes a SQL statement and returns the result.

- For `SELECT` statements: returns rows in `result.rows`
- For `INSERT`/`UPDATE`/`DELETE`: returns affected row count in `result.rowCount`
- For DDL (`CREATE`/`DROP`/`ALTER`): returns empty result

**Parameters**:
- `sql` — SQL statement. Use `?` for parameter placeholders.
- `params` — Optional array of parameter values.

**Example**:
```typescript
// Simple query
const result = await client.query('SELECT * FROM employees');

// Parameterized query
const result = await client.query(
  'SELECT * FROM employees WHERE dept_id = ? AND salary > ?',
  [1, 100000]
);
```

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

The `Pool` class manages a pool of reusable connections for concurrent access.

### Constructor

```typescript
new Pool(config: PoolConfig)
```

Creates a new connection pool. Connections are created lazily on first use. The pool uses a semaphore to enforce `maxConnections`.

### pool.query()

```typescript
query(sql: string, params?: any[]): Promise<QueryResult>
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

Returns the configured maximum number of connections (synchronous).

---

## PreparedStatement

A prepared SQL statement that can be executed multiple times with different parameters. Each prepared statement holds a dedicated server-side section; always close when done to free it.

### stmt.execute()

```typescript
execute(params?: any[]): Promise<QueryResult>
```

Executes the prepared statement with the given parameters.

### stmt.executeBatch()

```typescript
executeBatch(paramRows: any[][]): Promise<QueryResult>
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
query(sql: string, params?: any[]): Promise<QueryResult>
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
| `BIGINT` | `string` | Returned as string to avoid precision loss |
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
| `number` (large or float) | `DOUBLE` |
| `string` | `VARCHAR` |
| `boolean` | `BOOLEAN` |
| `null` | `NULL` |
| `Array<number>` | `BINARY` (byte array) |

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
