# db2-node

`db2-node` is a pure Rust DB2 driver for Node.js. It speaks the DRDA wire protocol directly, so there is no IBM CLI, ODBC, or `libdb2` runtime dependency.

This repository contains the driver, the protocol implementation, the Node.js bindings, the docs site, and the integration test harness used to ship the npm package.

## Repository Layout

- `crates/db2-proto` — low-level DRDA protocol encoding/decoding
- `crates/db2-client` — async Rust client, pooling, transactions, TLS, prepared statements
- `crates/db2-napi` — `napi-rs` bindings published as the `db2-node` npm package
- `tests/integration` — Rust integration tests against a real DB2 instance
- `tests/node` — Node.js integration tests against the public JS API
- `docs` — VitePress docs site
- `examples/demo.ts` / `examples/demo-million.ts` — repo-local examples for quick validation and benchmarking
- `examples/todo-app` — full-stack example app using the published Node bindings

## Package Quick Start

```bash
npm install db2-node
```

Install from a GitHub release tarball:

```bash
npm install https://github.com/gurungabit/db2-node/releases/download/v1.0.16/db2-node-1.0.16.tgz
```

Replace `v1.0.16` and `1.0.16` with the release version you want.

```ts
import { Client } from "db2-node";

const client = new Client({
  host: "localhost",
  port: 50000,
  database: "testdb",
  user: "db2inst1",
  password: "secret",
});

await client.connect();

const result = await client.query(
  "SELECT id, name FROM employees WHERE dept_id = ?",
  [1],
);

console.log(result.rows);

await client.close();
```

Package-level usage and API details live in `crates/db2-napi/README.md`.

## Data Type Coverage

The Rust protocol layer recognizes Db2 for z/OS built-in data families: numeric, decimal floating point, character, graphic, binary, `BLOB`, `CLOB`, `DBCLOB`, datetime, `ROWID`, `XML`, Boolean, nullable values, and distinct types through their source representation. Binary and BLOB results are returned as Node `Buffer`s; exact decimal values are returned as strings.

See `docs/data-types/index.md` for the full JavaScript mapping table.

## TLS Compatibility

The Node package supports IBM CLI-style TLS connection strings:

```txt
SECURITY=SSL;SSLServerCertificate=/path/to/cert.pem;SSLClientHostnameValidation=OFF;
```

`SSLServerCertificate` maps to `caCert`. For IBM `ibm_db` connection string compatibility, `SSLServerCertificate` also defaults hostname validation to `OFF` unless `SSLClientHostnameValidation=Basic` is supplied explicitly. Object-style `caCert` config remains strict by default.

## Db2 for z/OS Production Notes

The z/OS LOB cleanup behavior has two supported production modes:

- **Default mode:** run normally with no z/OS LOB environment variables. This is the recommended default for general production. When a CLOB/LOB result cannot be proven clean after materialization, the driver disconnects that socket and warm-replaces it in the pool so stale `EXTDTA` cannot affect the next query.
- **Active close mode:** set `DB2_ZOS_LOB_CLOSE_AFTER_MATERIALIZE=1` when preserving the same DB2 connection is more important than individual large-LOB query latency. The driver sends an active `CLSQRY`, drains until DB2 acknowledges the close, and reuses the connection only after cleanup is verified.

Keep `DB2_ZOS_LOB_TRUST_PASSIVE_TAIL_QUIET` off in production. It is fail-closed, but it is not a useful performance path for large z/OS CLOB workloads.

CLOB-like `LIKE` and `NOT LIKE` predicate scans that return aggregate or scalar results use a large statement package `EXCSQLSTT` path by default on Db2 for z/OS, which avoids the one-shot cursor package path that can hit package-specific resource limits. Set `DB2_ZOS_LIKE_PREDICATE_EXCSQLSTT=0` only for package diagnostics.

SQL errors surfaced through the JavaScript wrappers include `sqlstate`, `sqlcode`, and `retryable` fields when those values can be inferred. Stale z/OS cursor or statement section errors such as `SQLCODE=-502`, `SQLCODE=-514`, and `SQLCODE=-518` are marked retryable for read-query recovery.

Prepared statements and transactions are session-bound by design. If a reconnect happens after a stale cursor, timeout, or connection failure, create a new prepared statement or start a new transaction and retry the whole application unit of work. The driver does not replay transaction bodies or write statements automatically.

The production soak passed 100/100 default-mode cycles and 50/50 active-close cycles with no wrong row counts, zero-row corruption, stale `EXTDTA`, or unhandled driver errors.

## Local Development

### Requirements

- Rust toolchain
- Node.js 18+
- Docker

### Start the local DB2 test instance

```bash
make db2-start
```

To stop it again:

```bash
make db2-stop
```

## Build

Build the Rust workspace:

```bash
cargo build --workspace
```

Build the Node native addon:

```bash
cd crates/db2-napi
npm install
npm run build
```

## Test

Rust unit / protocol tests:

```bash
cargo test -p db2-proto
```

Rust integration tests:

```bash
cargo test -p db2-integration-tests -- --nocapture
```

Rust TLS integration tests:

```bash
DB2_TEST_SSL_PORT=50001 cargo test -p db2-integration-tests --test tls_test -- --nocapture
```

Node integration tests:

```bash
cd tests/node
npm ci
npm test
```

Node TLS tests:

```bash
cd tests/node
DB2_TEST_SSL_PORT=50001 npm test
```

## Run the Demos

The demos use the same `DB2_TEST_*` environment variables as the test suite.

Quick end-to-end demo:

```bash
npx --yes tsx examples/demo.ts
```

Bulk insert/read benchmark:

```bash
DEMO_TOTAL_ROWS=100000 npx --yes tsx examples/demo-million.ts
```

## Docs Site

Build the VitePress docs site:

```bash
make docs-build
```

Serve it locally with live reload:

```bash
make docs-serve
```

Then open:

- `http://localhost:5173/db2-node/`

The deployed docs site lives at `https://db2-node.github.io/`.

## Release Flow

- The npm package is `db2-node`
- `.github/workflows/release-please.yml` opens and updates a release PR from `main`
- Merging that release PR creates the next `v*` tag
- Tag pushes matching `v*` trigger `.github/workflows/release.yml`
- The release workflow builds prebuilt binaries, smoke-tests module loading, publishes npm artifacts, and attaches the npm-packed `.tgz` to the GitHub release

### Release Automation Notes

- Release Please follows Conventional Commits; `feat`, `fix`, and `deps` commits are releasable by default
- For the tag created by Release Please to trigger the publish workflow, set a `RELEASE_PLEASE_TOKEN` secret to a PAT or GitHub App token with repository write access

## Status

The `1.0.16` release line is production-ready for the validated DB2 LUW and Db2 for z/OS paths:

- Rust and Node integration suites are green
- TLS behavior is covered in both Rust and Node tests
- Prepared statements, pooling, reconnect behavior, and timeout handling have all been hardened
- Db2 for z/OS LOB materialization has passed default-mode and active-close soak testing with no stale `EXTDTA` corruption

## License

MIT. See `LICENSE`.
