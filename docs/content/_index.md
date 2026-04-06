---
title: "db2-node"
type: "home"
---

# db2-node

**A zero-dependency DB2 driver for Node.js, built with pure Rust.**

db2-node implements the IBM DRDA (Distributed Relational Database Architecture) wire protocol from scratch in Rust, exposed to Node.js via [napi-rs](https://napi.rs). No wrapping of `libpq`, `libdb2`, or any IBM CLI/ODBC library — we speak raw TCP to DB2.

## Why db2-node?

- **Zero native dependencies** — No `libdb2`, `unixODBC`, or OpenSSL required
- **Pure Rust performance** — Direct DRDA wire protocol implementation
- **First-class Node.js support** — Native addon via napi-rs with full TypeScript types
- **Connection pooling** — Built-in async pool with health checks and monitoring
- **TLS support** — Via rustls (pure Rust, no OpenSSL)
- **Prepared statements** — Dedicated server-side sections with automatic recycling

## Quick Example

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

const result = await client.query(
  'SELECT * FROM employees WHERE dept = ?',
  ['SALES']
);
console.log(result.rows);
// [{ ID: 1, NAME: 'Alice', DEPT: 'SALES' }, ...]

await client.close();
```

## Get Started

Head to the [Getting Started]({{< relref "getting-started/_index.md" >}}) guide to install db2-node and run your first query.
