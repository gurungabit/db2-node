# db2-node

Pure Rust DB2 driver for Node.js using the DRDA wire protocol directly. No IBM CLI, ODBC, or `libdb2` dependency is required at runtime.

## Status

`0.1.0` is an early release focused on DB2 LUW connectivity, parameterized queries, prepared statements, transactions, connection pooling, and TLS.

## Install

```bash
npm install db2-node
```

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

## Connection Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `host` | `string` | — | DB2 server hostname |
| `port` | `number` | `50000` | DB2 server port |
| `database` | `string` | — | Database name |
| `user` | `string` | — | Username |
| `password` | `string` | — | Password |
| `ssl` | `boolean` | `false` | Enable TLS/SSL |
| `rejectUnauthorized` | `boolean` | `true` | Verify server certificate (requires `ssl: true`) |
| `caCert` | `string` | — | Path to CA certificate PEM file |
| `connectTimeout` | `number` | `30000` | Connection timeout in ms (covers TCP + TLS handshake) |
| `queryTimeout` | `number` | `0` | Query execution timeout in ms (0 = no timeout) |
| `frameDrainTimeout` | `number` | `500` | Time in ms to wait for follow-up DRDA reply frames |
| `currentSchema` | `string` | — | Default schema for unqualified table names |
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
```

TLS uses `rustls` (pure Rust, no OpenSSL). System trust store certificates are loaded automatically when `rejectUnauthorized` is `true`. The `connectTimeout` covers the full TCP + TLS handshake.

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

Tag pushes matching `v*` trigger the release workflow in `.github/workflows/release.yml`, which builds native binaries for all targets and publishes to npm.
