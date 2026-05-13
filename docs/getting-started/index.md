# Getting Started

This guide walks you through installing `db2-node`, connecting to a DB2 database, and running your first queries.

## Prerequisites

- **Node.js** 18 or later
- **DB2** instance (local or remote) — see [Development Setup](#development-setup) for Docker instructions

## Installation

```bash
npm install db2-node
```

`db2-node` ships prebuilt native binaries for:
- Linux: `x64` glibc, `x64` musl, `arm64` glibc, `arm64` musl
- macOS: `x64`, `arm64` (Apple Silicon)
- Windows: `x64`, `arm64`

No compiler toolchain needed for supported platforms.

## Connecting to DB2

```typescript
import { Client } from 'db2-node';

const client = new Client({
  host: 'localhost',
  port: 50000,
  database: 'MYDB',
  user: 'db2inst1',
  password: 'password',
});

await client.connect();
```

### Connection Options

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `host` | `string` | — | DB2 server hostname |
| `port` | `number` | `50000` | DB2 server port |
| `database` | `string` | — | Database name |
| `user` | `string` | — | Username |
| `password` | `string` | — | Password |
| `ssl` | `boolean` | `false` | Enable TLS/SSL |
| `rejectUnauthorized` | `boolean` | `true` | Verify server certificate |
| `sslClientHostnameValidation` | `string` | `'Basic'` | IBM-compatible hostname validation mode: `'Basic'` or `'OFF'` |
| `caCert` | `string` | — | Path to CA certificate PEM file |
| `connectTimeout` | `number` | `30000` | Connection timeout in ms (TCP + TLS) |
| `queryTimeout` | `number` | `0` | Query timeout in ms (0 = no timeout) |
| `frameDrainTimeout` | `number` | `25` | DRDA reply frame drain timeout in ms |
| `currentSchema` | `string` | — | Default schema |
| `fetchSize` | `number` | `100` | Rows per fetch batch |

## Running Queries

### Simple Queries

```typescript
const result = await client.query('SELECT * FROM employees');
console.log(result.rows);    // Array of row objects
console.log(result.rowCount); // Number of rows
console.log(result.columns);  // Column metadata
```

See [Data Type Support](../data-types/index.md) for the Db2 for z/OS built-in type mapping, including BLOB, CLOB, DBCLOB, XML, ROWID, graphic strings, binary strings, and datetime values.

### Parameterized Queries

Always use parameterized queries to prevent SQL injection:

```typescript
const result = await client.query(
  'SELECT name, salary FROM employees WHERE dept_id = ? AND active = ?',
  [1, true]
);
```

### INSERT / UPDATE / DELETE

```typescript
const result = await client.query(
  'INSERT INTO employees (name, dept_id) VALUES (?, ?)',
  ['Alice', 1]
);
console.log(result.rowCount); // 1
```

## Prepared Statements

For queries executed multiple times with different parameters:

```typescript
const stmt = await client.prepare(
  'INSERT INTO employees (name, dept_id) VALUES (?, ?)'
);

await stmt.execute(['Alice', 1]);
await stmt.execute(['Bob', 2]);
await stmt.execute(['Carol', 1]);

// Batch insert (single round-trip)
await stmt.executeBatch([
  ['Dave', 3],
  ['Eve', 1],
]);

await stmt.close(); // always close when done
```

## Transactions

```typescript
const tx = await client.beginTransaction();

try {
  await tx.query('UPDATE accounts SET balance = balance - 100 WHERE id = ?', [1]);
  await tx.query('UPDATE accounts SET balance = balance + 100 WHERE id = ?', [2]);
  await tx.commit();
} catch (e) {
  await tx.rollback();
  throw e;
}
```

Transactions also support prepared statements via `tx.prepare()`.

## Connection Pool

For applications with concurrent database access, `Pool` supports the modern
config-object API and the common `ibm_db` pool shape:

```typescript
import { Pool } from 'db2-node';

const pool = new Pool({
  host: 'localhost',
  port: 50000,
  database: 'MYDB',
  user: 'db2inst1',
  password: 'password',
  maxConnections: 20,
});

// Pool manages connections automatically
const result = await pool.query('SELECT COUNT(*) AS cnt FROM employees');

// Or acquire a connection for multiple operations
const client = await pool.acquire();
try {
  // ... use client ...
} finally {
  await pool.release(client);
}

await pool.close();
```

CommonJS callers migrating from `ibm_db` can create a pool without connection
config and open databases by connection string:

```javascript
const ibmdb = require('db2-node');

const connStr = 'DATABASE=MYDB;HOSTNAME=localhost;PORT=50000;PROTOCOL=TCPIP;UID=db2inst1;PWD=password';
const pool = new ibmdb.Pool();

const db = await pool.open(connStr);
const rows = await db.query('SELECT COUNT(*) AS cnt FROM employees');

await db.close();
await pool.close();
```

### Pool Options

All connection options above, plus:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `minConnections` | `number` | `0` | Minimum idle connections to maintain |
| `maxConnections` | `number` | `10` | Maximum total connections |
| `idleTimeout` | `number` | `600` | Close idle connections after this many seconds (10 min) |
| `maxLifetime` | `number` | `3600` | Recycle connections after this many seconds (1 hour) |
| `healthCheckInterval` | `number` | `30` | Reuse an idle connection without a health-check round trip for this many seconds |

## TLS / SSL

```typescript
// Skip certificate verification (development/testing)
const client = new Client({
  host: 'localhost',
  port: 50001,
  database: 'testdb',
  user: 'db2inst1',
  password: 'secret',
  ssl: true,
  rejectUnauthorized: false,
});

// Verify with custom CA certificate (production)
const client = new Client({
  host: 'db2.example.com',
  port: 50001,
  database: 'PRODDB',
  user: 'app_user',
  password: 'secret',
  ssl: true,
  caCert: '/path/to/ca-cert.pem',
});

// IBM CLI-compatible SSLServerCertificate behavior for DB2 certs without SANs
const client = new Client({
  host: 'db2-vip.example.com',
  port: 50001,
  database: 'PRODDB',
  user: 'app_user',
  password: 'secret',
  ssl: true,
  rejectUnauthorized: true,
  caCert: '/path/to/server-or-ca-cert.pem',
  sslClientHostnameValidation: 'OFF',
});
```

When `rejectUnauthorized` is `true` (the default), system trust store certificates are loaded automatically. Custom CA certificates from `caCert` are added on top. IBM-style connection strings can also use `SSLServerCertificate=/path/to/cert.pem`, which maps to `caCert`.

Hostname validation is enabled by default (`sslClientHostnameValidation: 'Basic'`). For IBM `ibm_db` connection string compatibility, `SSLServerCertificate=/path/to/cert.pem` defaults hostname validation to `OFF` unless `SSLClientHostnameValidation=Basic` is supplied explicitly. Object-style `caCert` config remains strict by default. Use `sslClientHostnameValidation: 'OFF'` only when the DB2 server certificate is trusted but does not contain a matching DNS/IP subjectAltName. That mode verifies the certificate chain and skips only the hostname/SAN check.

## Db2 z/OS LOB Production Modes

Db2 for z/OS can continue sending external LOB data (`EXTDTA`) after an application has materialized the requested rows. The driver keeps pooled connections safe by verifying cleanup before reuse.

### Default mode

Run normally with no z/OS LOB environment variables:

```bash
node app.js
```

This is the recommended default. If a z/OS LOB query cannot be proven clean after materialization, the driver disconnects that socket and warm-replaces it in the pool. This prevents stale `EXTDTA` from corrupting the next query and is the fastest mode for large CLOB workloads where preserving the exact same server session is not required.

CLOB-like `LIKE` and `NOT LIKE` predicate scans that return aggregate or scalar results use a large statement package `EXCSQLSTT` path by default on Db2 for z/OS. Set `DB2_ZOS_LIKE_PREDICATE_EXCSQLSTT=0` only while diagnosing package-specific behavior.

Errors returned through the JavaScript wrapper APIs include `sqlstate`, `sqlcode`, and `retryable` fields when the driver can infer them from the DB2 error. Stale z/OS cursor or statement state (`SQLCODE=-502`, `SQLCODE=-514`, `SQLCODE=-518`) is marked retryable. For plain read queries the driver can reconnect and retry internally; for prepared statements and transactions, recreate the prepared statement or retry the whole transaction body because those resources are bound to one server session.

### Active close mode

Set `DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE=1` when preserving the same DB2 connection is more important than individual large-LOB query latency:

```bash
DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE=1 node app.js
```

The driver sends `CLSQRY`, drains remaining `EXTDTA`, waits for DB2's close acknowledgement, and only then returns the connection to the pool.

Keep `DB2_ZOS_LOB_TRUST_PASSIVE_TAIL_QUIET` off in production. It is fail-closed, but it is not a useful performance path for large z/OS CLOB workloads.

Production soak coverage for the current `1.0.x` release line:

| Mode | Cycles | Result |
|------|--------|--------|
| Default disconnect + warm replacement | 100/100 | Passed |
| Active close | 50/50 | Passed |

Both soaks completed with no wrong row counts, zero-row corruption, stale `EXTDTA`, or unhandled driver errors.

## Development Setup

To run a local DB2 instance for development and testing:

```bash
# Clone the repository
git clone https://github.com/gurungabit/db2-node.git
cd db2-node

# Start DB2 in Docker (takes 2-5 min on first run)
./tools/db2.sh start

# DB2 is now running at localhost:50000
# Database: testdb
# User: db2inst1
# Password: db2wire_test_pw
```

See the [Contributing](../contributing/index.md) guide for full development instructions.
