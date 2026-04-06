# db2-node

`@gurungabit/db2-node` is a zero-dependency DB2 driver for Node.js, built in pure Rust. It speaks the IBM DRDA wire protocol directly, so there is no IBM CLI, ODBC, `libdb2`, or OpenSSL runtime dependency to install.

[Get started](getting-started/index.md){ .md-button .md-button--primary }
[Read the API](api-reference/index.md){ .md-button }

## Why db2-node?

- Zero native dependencies
- Pure Rust DRDA implementation
- First-class Node.js bindings via `napi-rs`
- Built-in pooling, transactions, prepared statements, and TLS
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

<div class="grid cards" markdown>

- :material-rocket-launch-outline: __Getting Started__
  ---
  Install the package, connect to DB2, and run your first query.
  [Open the guide](getting-started/index.md)

- :material-book-open-page-variant: __API Reference__
  ---
  See the TypeScript surface for connections, pooling, prepared statements, and transactions.
  [Browse the API](api-reference/index.md)

- :material-source-branch: __Architecture__
  ---
  Learn how the Rust crates, Node bindings, and transport layer fit together.
  [Read the architecture](architecture/index.md)

- :material-lan-connect: __Protocol__
  ---
  Drill into DRDA framing, DDM objects, query flow, and wire-level details.
  [Explore the protocol](protocol/index.md)

</div>
