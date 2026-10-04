# LOB decoding regressions (#19 and #20)

Both reports were reproduced against the existing Db2 LUW 12.1 server before
the fix. `CLOB(1M)` with `GRAPHIC(3)` returned no rows; with `DOUBLE` it returned
one row containing `Real(0.0)` instead of three rows containing 1.5, 2.5 and 3.5.
A CLOB followed by a four-byte BLOB returned CLOB bytes in the BLOB and also
shifted payloads between rows. `CLOB(1K)` fixed-column controls decoded correctly.

The causes were:

- EXTDTA was applied in the cursor, again while collecting fetches, and again
  when finishing the query. Materialized four-byte BLOBs still matched the
  reference heuristic, so they consumed other columns' payloads.
- Compact SQLDARD discovery rejected declared lengths above 64 KiB, losing
  CLOB(1M) metadata and sometimes following columns. The QRYDSC fallback then
  confused default FD:OCA LIDs with standalone scalar codes, decoding DOUBLE as
  REAL and GRAPHIC as VARCHAR.
- LUW fetches could send another CNTQRY before a chained EXTDTA reply finished.
  The five-row, 25-column case with `fetchSize: 1` reproduced a server disconnect
  and late QRYNOPRM replies before chain completion was fixed.

LUW rows now retain the columns that explicitly reference external data on the
wire. Payloads are applied once in row/column order; genuine inline four-byte
BLOBs and inline text beginning `LOB locator 0x` cannot consume them. Empty LOB
references consume no EXTDTA. Fetches read the advertised DSS chain to completion.
LOB metadata retains large declarations, and compact descriptors distinguish
float widths, DBCS strings, and mixed-character CLOBs.

Protocol coverage includes captured compact descriptor shapes, partial rows split
inside fixed values and LOB references, inline/external LOB mixtures, and repeated
materialization. Existing z/OS MDD/SDA overrides, EBCDIC rows, descriptor preference,
and cursor/materialization tests continue to pass. The z/OS early-materialization
and cleanup strategy remains in place; no live z/OS server was available.

Live coverage uses uniquely named tables and drops them after each case. The Rust
tests cover the exact issue projections and 1K/1M controls. The Node tests cover
direct and prepared execution with `fetchSize: 1`, NULL/empty LOBs, multiple
CLOB/BLOB rows, a 70,000-byte CLOB, DBCLOB, XML, and all 25 columns.

Verification commands (both local servers use the defaults in the test helpers):

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked -p db2-proto -p db2-client -p db2-proto-tests
cargo test --locked -p db2-integration-tests --test lob_regressions_test --test types_test --test query_test --test prepared_stmt_test
DB2_TEST_PORT=50002 cargo test --locked -p db2-integration-tests --test lob_regressions_test --test types_test --test query_test --test prepared_stmt_test
# Build the native addon before running Node tests.
cd tests/node
node --import tsx --test ./lob-regressions.test.ts
DB2_TEST_PORT=50002 node --import tsx --test ./lob-regressions.test.ts
node --import tsx --test ./data-types.test.ts ./query.test.ts ./luw-regressions.test.ts ./luw-followup-regressions.test.ts
```

The targeted Rust unit/protocol run passes 218 tests. The four live Rust suites
pass 31 tests on each Db2 version; the new Node regressions pass on both versions.
The existing Node type/query and LUW regression suites pass all 48 tests on 12.1.
Cargo.lock and dependency versions are unchanged. TLS and authentication changes
are outside this fix.
