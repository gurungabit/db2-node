# db2-node Performance & Edge Case Suite

Comprehensive benchmarks for `@gurungabit/db2-node` covering CRUD, batch operations, data pipelines, BLOB handling, edge cases, and connection pool stress tests.

## Quick Start

```bash
cd examples/performance
npm install
npm run all        # Run everything
```

## Individual Suites

| Command          | Description                                       |
|------------------|---------------------------------------------------|
| `npm run crud`   | INSERT / SELECT / UPDATE / DELETE at scale         |
| `npm run batch`  | Large batch inserts, deletes, rollbacks, churn     |
| `npm run pipeline` | Rapid fetches, concurrent queries, ETL, pagination |
| `npm run blob`   | BLOBs, CLOBs, wide rows, mixed types, NULL-heavy  |
| `npm run edge`   | Unicode, boundary values, error handling, metadata |
| `npm run pool`   | Pool saturation, acquire/release, multi-pool       |

## Environment Variables

### Connection (same as integration tests)

```
DB2_TEST_HOST=localhost
DB2_TEST_PORT=50000
DB2_TEST_DATABASE=testdb
DB2_TEST_USER=db2inst1
DB2_TEST_PASSWORD=db2wire_test_pw
```

### Tuning

```
CRUD_ROWS=10000          # Rows for CRUD bench (default: 10,000)
CRUD_BATCH_SIZE=2000     # Batch size for CRUD (default: 2,000)
BATCH_TOTAL=100000       # Rows for batch bench (default: 100,000)
PIPELINE_ROWS=50000      # Rows for pipeline bench (default: 50,000)
```

## What's Tested

### CRUD (`bench-crud.ts`)
- Batch inserts with prepared statements
- Full table scan, point lookups, range scans
- Aggregations (COUNT/SUM/AVG/MIN/MAX), GROUP BY
- Bulk update, conditional update, single-row updates
- Partial delete, full purge

### Batch Operations (`bench-batch.ts`)
- Insert throughput at batch sizes: 100, 500, 1K, 5K, 10K
- Single large-commit transaction stress
- Delete by bucket, range, full purge
- Insert→delete churn cycles
- Large transaction rollback verification

### Data Pipeline (`bench-pipeline.ts`)
- 100 rapid sequential fetches
- 10-way concurrent queries via Pool
- OFFSET/FETCH pagination vs keyset pagination
- ETL: aggregate → transform → write
- 10,000 prepared statement reuse cycles
- 500 concurrent pool point-queries

### BLOB / Large Data (`bench-blob.ts`)
- BLOB insert/read: 1KB, 10KB, 100KB, 500KB
- Batch 500x 4KB BLOBs
- CLOB: 1KB, 10KB, 100KB text
- 50-column wide rows (5,000 rows)
- Mixed data types (all supported DB2 types)
- NULL-heavy sparse data (80% NULLs)

### Edge Cases (`bench-edge-cases.ts`)
- Empty result sets (1,000 queries)
- Unicode: Japanese, Russian, Arabic, Emoji, math symbols
- Boundary numerics: SMALLINT/INT/BIGINT min/max, DECIMAL precision
- Rapid connect/disconnect (50 cycles)
- Empty transaction commit/rollback
- Insert→rollback→reinsert with same PK
- Duplicate key error handling (500 errors)
- 200 UNION ALL long SQL
- 500-parameter IN clause
- Prepared statement open/close churn
- System catalog queries (SYSCAT.TABLES, SYSCAT.COLUMNS)

### Connection Pool (`bench-pool.ts`)
- Pool warmup and stats
- Sequential acquire/release (200 cycles)
- Pool saturation at max connections
- Beyond-max queuing (20 concurrent on max=10)
- Mixed read/write workload (200 concurrent ops)
- Acquire→transaction→commit→release pattern
- Multiple pools (3 pools concurrently)
- Rapid pool create/destroy (20 cycles)
