# db2-node

`@gurungabit/db2-node` is a zero-dependency DB2 driver for Node.js, built in pure Rust. It speaks the IBM DRDA wire protocol directly, so there is no IBM CLI, ODBC, `libdb2`, or OpenSSL runtime dependency to install.

## Start Here

- [Getting Started](getting-started/index.md)
- [API Reference](api-reference/index.md)
- [Data Type Support](data-types/index.md)

## Why db2-node?

- Zero native dependencies
- Pure Rust DRDA implementation
- First-class Node.js bindings via `napi-rs`
- Built-in pooling, transactions, prepared statements, and TLS
- Db2 LUW and Db2 for z/OS type coverage for numeric, string, binary, LOB, datetime, XML, ROWID, and Boolean values
- Searchable docs with section navigation and page outlines

## Quick Example

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

const result = await client.query(
  'SELECT * FROM employees WHERE dept = ?',
  ['SALES']
);
console.log(result.rows);
// [{ ID: 1, NAME: 'Alice', DEPT: 'SALES' }, ...]

await client.close();
```

## Explore the docs

| Page | What it covers |
|------|----------------|
| [Getting Started](getting-started/index.md) | Install the package, connect to DB2, and run your first query. |
| [API Reference](api-reference/index.md) | TypeScript API for connections, pools, prepared statements, and transactions. |
| [Data Type Support](data-types/index.md) | Db2 for z/OS built-in data type coverage and JavaScript mappings. |
| [Architecture](architecture/index.md) | How the Rust crates, Node bindings, and transport layer fit together. |
| [Protocol](protocol/index.md) | DRDA framing, DDM objects, query flow, and wire-level details. |
