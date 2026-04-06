# Getting Started

This guide walks you through installing `@gurungabit/db2-node`, connecting to a DB2 database, and running your first queries.

## Prerequisites

- **Node.js** 18 or later
- **DB2** instance (local or remote) — see [Development Setup](#development-setup) for Docker instructions

## Installation

```bash
npm install @gurungabit/db2-node
```

`@gurungabit/db2-node` ships prebuilt native binaries for:
- Linux: `x64` glibc, `x64` musl, `arm64` glibc, `arm64` musl
- macOS: `x64`, `arm64` (Apple Silicon)
- Windows: `x64`, `arm64`

No compiler toolchain needed for supported platforms.

## Connecting to DB2

```typescript
import { Client } from '@gurungabit/db2-node';

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
| `caCert` | `string` | — | Path to CA certificate PEM file |
| `connectTimeout` | `number` | `30000` | Connection timeout in ms (TCP + TLS) |
| `queryTimeout` | `number` | `0` | Query timeout in ms (0 = no timeout) |
| `frameDrainTimeout` | `number` | `500` | DRDA reply frame drain timeout in ms |
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

For applications with concurrent database access:

```typescript
import { Pool } from '@gurungabit/db2-node';

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

### Pool Options

All connection options above, plus:

| Option | Type | Default | Description |
|--------|------|---------|-------------|
| `minConnections` | `number` | `0` | Minimum idle connections to maintain |
| `maxConnections` | `number` | `10` | Maximum total connections |
| `idleTimeout` | `number` | `600` | Close idle connections after this many seconds (10 min) |
| `maxLifetime` | `number` | `3600` | Recycle connections after this many seconds (1 hour) |

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
```

When `rejectUnauthorized` is `true` (the default), system trust store certificates are loaded automatically. Custom CA certificates from `caCert` are added on top.

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
