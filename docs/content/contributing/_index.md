---
title: "Contributing"
weight: 50
---

# Contributing

This guide covers the development setup, testing infrastructure, and contribution workflow for db2-node.

## Prerequisites

- **Rust** (stable, via [rustup](https://rustup.rs))
- **Node.js** 18+
- **Docker** and Docker Compose (for DB2 test instance)
- **~4GB RAM** for the DB2 container

## Quick Start

```bash
# Clone the repo
git clone https://github.com/gurungabit/db2driver-node.git
cd db2driver-node

# Build everything
make build

# Start DB2 test container (2-5 min first time)
make db2-start

# Run all tests
make test
```

## DB2 Test Container

Integration tests run against a real DB2 instance in Docker.

### Managing the Container

```bash
./tools/db2.sh start    # Start + wait for ready + seed
./tools/db2.sh stop     # Stop and remove container
./tools/db2.sh status   # Check if running and ready
./tools/db2.sh seed     # Re-run seed SQL scripts
./tools/db2.sh reset    # Full stop -> start -> seed
./tools/db2.sh sql      # Open interactive db2 shell
./tools/db2.sh exec "SELECT ..."  # Run a single SQL statement
./tools/db2.sh logs     # Tail container logs
```

### Container Details

| Setting | Value |
|---------|-------|
| Image | `icr.io/db2_community/db2:11.5.9.0` |
| Port | `50000` |
| Database | `testdb` |
| User | `db2inst1` |
| Password | `db2wire_test_pw` |
| Memory | 4GB limit |

The container requires `privileged: true` for DB2's shared memory. First startup takes 2-5 minutes for initialization.

On Apple Silicon (M1/M2/M3), DB2 runs under x86 emulation via Rosetta. It works but is slower.

## Test Structure

### Unit Tests (no DB2 needed)

```bash
make test-unit
# or: cargo test --workspace --lib && cargo test -p db2-proto
```

Unit tests validate protocol serialization/parsing using captured byte fixtures. They run offline with no database connection.

Located in:
- `tests/protocol/dss_test.rs` — DSS frame parsing
- `tests/protocol/ddm_test.rs` — DDM object building
- `tests/protocol/codepage_test.rs` — EBCDIC conversion

### Integration Tests (requires DB2)

```bash
make test-integration
```

Integration tests connect to the DB2 Docker container. The Makefile auto-starts DB2 if needed.

Test files:
- `connection_test.rs` — Connect/disconnect, auth errors
- `query_test.rs` — SELECT, INSERT, UPDATE, DELETE
- `prepared_stmt_test.rs` — Parameterized queries
- `transaction_test.rs` — Commit/rollback
- `types_test.rs` — Data type round-trips
- `pool_test.rs` — Connection pool
- `edge_cases_test.rs` — Long SQL, special chars, etc.

### Node.js Tests

```bash
make test-node
```

Tests the napi-rs bindings from JavaScript.

Located in: `tests/node/*.test.ts`

## Capturing Protocol Fixtures

To create new test fixtures from real DB2 traffic:

```bash
# Requires tshark (Wireshark CLI) and a running DB2 container
sudo ./tools/capture-fixtures.sh
```

This captures DRDA byte sequences for known operations and saves them in `tests/protocol/fixtures/` for use in unit tests.

## Documentation Site

You can run the Hugo docs site locally without installing Hugo on your machine:

```bash
make docs-serve
```

This serves the site at `http://localhost:1313/db2driver-node/`.

To build the static site into `.tmp-docs-public/`:

```bash
make docs-build
```

## Makefile Targets

| Target | Description |
|--------|-------------|
| `make build` | Build all crates |
| `make build-release` | Release build |
| `make test` | Run unit + integration tests |
| `make test-unit` | Unit tests only (no DB2) |
| `make test-integration` | Integration tests (starts DB2 if needed) |
| `make test-node` | Node.js tests |
| `make docs-build` | Build the Hugo docs site into `.tmp-docs-public/` |
| `make docs-serve` | Serve the Hugo docs site locally on port 1313 |
| `make db2-start` | Start DB2 container |
| `make db2-stop` | Stop DB2 container |
| `make db2-status` | Check DB2 status |
| `make capture` | Capture Wireshark fixtures |
| `make clean` | Clean build + stop DB2 |

## Code Organization

```
db2driver-node/
+-- crates/
|   +-- db2-proto/      # Pure protocol (zero deps, no I/O)
|   +-- db2-client/     # Async client (tokio)
|   +-- db2-napi/       # Node.js bindings (napi-rs)
+-- docker/             # DB2 container + seed SQL
+-- tests/              # All tests
+-- tools/              # Helper scripts
```

### Guidelines

- `db2-proto` must remain zero-dependency — no I/O, no async, no allocator
- All integers on the wire are big-endian — use `from_be_bytes()` / `to_be_bytes()`
- Always negotiate UTF-8 (CCSID 1208) during connection
- Use parameterized queries — never interpolate user input into SQL
- Integration tests should clean up after themselves (use temp tables)

## CI Pipeline

GitHub Actions runs three jobs:

1. **Unit Tests** — Fast, no Docker. Also runs `clippy` and `fmt` checks.
2. **Integration Tests** — Starts DB2 in Docker, seeds schema, runs tests.
3. **Node.js Tests** — Builds napi module, starts DB2, runs TypeScript tests.

See `.github/workflows/ci.yml` for details.

## Submitting Changes

1. Fork the repo and create a feature branch
2. Make your changes
3. Run `make test` locally (or at least `make test-unit`)
4. Ensure `cargo clippy` and `cargo fmt --check` pass
5. Open a pull request with a clear description
