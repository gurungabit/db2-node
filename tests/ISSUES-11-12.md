# Issues #11 and #12: behavior and Docker verification

The original uncommitted changes were preserved in commit `1703d77` before issue work started. That snapshot accumulates input descriptors across SQLDARD replies, adds a z/OS input-descriptor parser, binds character/datetime strings through UTF-8 VARCHAR representations, allows an explicit required-native-binary list during publishing, and adds a macOS ARM64 Mach-O check with tests. Those changes alone did not resolve the LUW issues.

Review also found that the new z/OS descriptor parser could discard DECIMAL precision/scale and that a comma-only required-binary override could bypass the publication check. Both are corrected, with regression coverage. DECIMAL(31,2) retains its scale and packs `123.45` correctly; an empty required-binary list fails publication verification.

## Issue #11

[Issue #11](https://github.com/gurungabit/db2-node/issues/11) is resolved by explicit validation after detecting the server. LUW supports an omitted `typeDefinitionName` or `QTDSQLX86`. Other explicit values, including `none`, raise an error instead of being silently ignored. z/OS retains its existing configurable representations. Supporting alternative LUW representations would require corresponding decoders.

## Issue #12

| Reported item | Resulting behavior |
| --- | --- |
| 1. UTF-8 literal and parameter corruption | CCSID 1208 decodes as UTF-8; LUW character descriptors use the negotiated UTF-8 representation. Non-ASCII text round trips. |
| 2. INTEGER beside DECFLOAT/BOOLEAN is byte-swapped | LUW descriptors keep little-endian numeric values even when extended type metadata is present. |
| 3. BOOLEAN phantom rows | Boolean metadata recognizes SQLTYPE 2436/2437, and nullable Boolean values consume the proper null indicator and two-byte value. Three rows stay three rows. |
| 4. SELECT * loses metadata/rows | Boolean, timestamp precision and LOB descriptors preserve all columns and rows in the mixed seven-column table. |
| 5. XML NULL rows disappear | LUW continues fetching after materializing LOB values instead of using z/OS-specific early-close behavior. NULL rows survive. |
| 6. BIGINT rounding and string-binding refusal | Values within JavaScript's safe integer range remain numbers; larger values become exact decimal strings. BIGINT input descriptors accept decimal strings and JavaScript bigint. |
| 7. LOB fetch errors | LUW open/continue query requests include rowset size and legal extended-block settings. CLOB, DBCLOB, BLOB, XMLSERIALIZE and catalog LOB columns fetch correctly, including three large UTF-8 CLOBs spanning continuation blocks. |
| 8. TIMESTAMP(0)/(12) decoding | Wire descriptor lengths determine timestamp width instead of always assuming 26 bytes. |
| 9. CALL fails / loses results | CALL uses EXCSQLSTT rather than a SELECT cursor-open request. Procedure result-set packages, output descriptors and embedded end-of-query SQLCA are decoded. Direct, prepared, transaction and compatibility APIs expose three tested result sets, LOB/XML values and an OUT BIGINT. |
| 10. JavaScript bigint aborts Node | A native parameter converter handles bigint before napi's unsupported JSON conversion. Invalid object parameters raise catchable errors. |
| 11. BOOLEAN/LOB descriptors, Date and GRAPHIC | Complete descriptors support mixed Boolean/CLOB/BLOB parameters; Date becomes UTC timestamp text; GRAPHIC pads with U+0020. Native and wrapper APIs accept binary buffers, Uint8Array and ArrayBuffer. |
| 12. Authentication downgrade / unreliable encryption | Non-TLS encrypted requests reject plaintext fallback before SECCHK sends credentials. LUW retries preserve the requested algorithm; AES negotiation omits the invalid key-length parameter; reply chains are fully parsed; LUW SECMEC 7 uses the negotiated encoding. Two incorrect DES S-box entries are corrected. DH private keys use OS cryptographic randomness. Repeated AES/DES sessions work with SERVER_ENCRYPT and AES works with AES_ONLY. Rejected DES reports the server authentication error. |
| 13. currentSchema ignored | Connect executes SET CURRENT SCHEMA with a quoted, escaped schema value. |
| 14. serverInfo instance name | productName comes from the server class, such as DB2/LINUXX8664. |
| 15. Timed-out work continues | queryTimeout opens a separate control session and invokes WLM_CANCEL_ACTIVITY before resetting the timed-out connection. Client.cancel() can interrupt active work without taking its query lock; the connection remains usable. Tests verify that MON_GET_ACTIVITY no longer contains the cancelled work. |
| 16. Leading comments cause SQLCODE -84 | Direct and prepared statement classification skips leading line/block comments, including multiple and nested block comments, while sending the original SQL to Db2. Comment-prefixed SELECT, VALUES and CALL execute correctly. |
| 17. Zero-row UPDATE reports a negative count | SQLCAXGRP decodes all six SQLERRD integers. SQLERRD3 supplies the affected count; SQLERRD1/2 diagnostics cannot replace a zero count. Direct/prepared unmatched UPDATE returns zero; a matching UPDATE returns one. |
| 18. Truncation warning breaks fetch | QRYDTA consumes the full warning SQLCA, null diagnostic group and SQLDTAGRP envelope before decoding the value. Direct/prepared truncation returns ten `x` characters and keeps the connection usable. A protocol test splits the warning row at every byte boundary. |
| 19. Two VARCHAR(32672) columns break DSS framing | A continued DSS beginning with `0xFFFF` remains a separate logical frame when it arrives with earlier metadata frames in one TCP read. Direct/prepared queries preserve both 32,672-character values, including two wide table rows followed by a NULL row. Protocol tests cover a full combined reply and a partial 64 KiB read; a live proxy combines replies to reproduce the CI failure deterministically. |
| 20. Overflowing/malformed bound DECIMAL silently changes | Packed-decimal encoding validates decimal digits and the integer capacity `precision - scale`. `12345.67`, `99999` and malformed text fail before execution for DECIMAL(5,2); existing stored values remain unchanged. Excess fractional places retain Db2 assignment truncation. |
| 21. Nested bigint aborts / Date and Map become `{}` | Native and wrapper APIs reject nested bigint/objects/Map with catchable errors; Date produces UTC timestamp text. Binary arrays are additionally restricted to integer bytes 0–255, preventing nested arrays or invalid values from silently becoming empty/corrupted binary data. |

The [follow-up comment](https://github.com/gurungabit/db2-node/issues/12#issuecomment-5965620029) added cases 16–21 after the initial PR. Cases 16, 17, 18 and 20 reproduced on the initial PR revision and are fixed by the follow-up change. Case 19 initially passed the local Docker runs, but Linux CI exposed a failure when TCP combined metadata and a continued row frame. That boundary is now corrected and covered by protocol and live coalescing-proxy tests. The originally reported inputs in 21 already passed; additional byte-array validation closes related silent-conversion cases.

## API changes and limits

- Query results add `resultSets` and `outputParameters`. For CALL, the usual `rows`, `columns` and `rowCount` describe the first result set. OUT/INOUT values are returned in parameter order; IN-only parameters are omitted.
- Compatibility `queryResult()` and prepared-result objects expose those fields; callback forms receive OUT parameters as their third argument.
- Unsafe BIGINT results intentionally change from rounded numbers to decimal strings. Date parameters use UTC, not local time.
- A stock `AUTHENTICATION=SERVER` database requires an explicit `securityMechanism: 'userPassword'`, TLS, or server configuration enabling encrypted authentication. The default encrypted setting now rejects plaintext fallback.
- Cancellation is implemented for LUW and requires activity-monitoring privileges and permission to execute `SYSPROC.WLM_CANCEL_ACTIVITY`. The control operation is bounded to five seconds. Timeout errors report cancellation success/failure and then close the original connection. This test environment uses the instance owner; it does not establish cancellation permissions for arbitrary application users.
- z/OS parser regression tests run locally; no live z/OS server was tested.

## Verified results

All required runs passed with the release native binding built from this branch:

| Run | Result |
| --- | --- |
| Rust workspace, including live 12.1 integration and TLS tests | 281 passed, 0 failed |
| Full Node suite on 12.1 with TLS enabled | 83 passed, 0 failed; 2 authentication-mode tests skipped here and exercised separately |
| Node issue/authentication regressions on stock 11.5 | 29 passed, 0 failed; 2 authentication-mode tests exercised separately |
| Native package compatibility/artifact tests | 15 passed, 0 failed |
| Bun 1.3.11, original and follow-up LUW assertions on 11.5 | 26 checks passed |
| SERVER_ENCRYPT on each server version | 40 fresh sessions per version: ten each for SECMEC 9/7 × AES/DES |
| AES_ONLY on each server version | 20 fresh AES sessions per version; both DES mechanisms rejected with an authentication error |
| Release build, ARM64 native-header verification, strict workspace Clippy, docs build | Passed |

The installed Bun version cannot execute nested `node:test` subtests. Its 20 original and six follow-up issue checks were run with the same SQL/assertion bodies in standalone sequential harnesses. Encrypted-authentication repetition and the full suite were run under Node. Authentication-mode runs were completed on the initial issue-fix revision; the follow-up change does not alter authentication.

Both test containers were restored to `AUTHENTICATION=SERVER`, `ALTERNATE_AUTH_ENC=NOT_SPECIFIED` after authentication checks. The 12.1 TLS listener remains enabled.

## Reproduction

The runs use Node 24.14.0 and Bun 1.3.11 on macOS ARM64 with Docker Desktop emulating these AMD64 images:

- `icr.io/db2_community/db2:12.1.0.0`: `db2-wire-test`, TCP 50000 and TLS 50001.
- `icr.io/db2_community/db2:11.5.9.0`: `db2-wire-test-115`, TCP 50002.

Both databases are UTF-8 `testdb`, with the repository's Docker test credentials. The regression suite creates and drops its own table and procedure. The broader Rust/Node suites use the seed data.

For a fresh 12.1 container, from the repository root:

```sh
docker run -d --name db2-wire-test --platform linux/amd64 --privileged \
  --memory 4g -p 127.0.0.1:50000:50000 -p 127.0.0.1:50001:50001 \
  -e LICENSE=accept -e DB2INST1_PASSWORD=db2wire_test_pw -e DBNAME=testdb \
  -e ARCHIVE_LOGS=false -e AUTOCONFIG=false \
  -v db2-node-issues-12-data:/database -v "$PWD/docker/seed:/seed:ro" \
  icr.io/db2_community/db2:12.1.0.0
# Wait for database setup to finish before seeding or running tests.
bash tools/db2.sh seed
bash docker/tls/generate-certs.sh
bash docker/tls/setup-db2-ssl.sh
npm --prefix crates/db2-napi run build
DB2_TEST_SSL_PORT=50001 npm --prefix tests/node test
DB2_TEST_SSL_PORT=50001 cargo test --workspace
npm --prefix crates/db2-napi test
```

For the 11.5 regression run, start the second image with the same environment, a separate volume and `-p 127.0.0.1:50002:50000`, then run:

```sh
cd tests/node
DB2_TEST_PORT=50002 node --import tsx --test ./luw-regressions.test.ts ./luw-followup-regressions.test.ts ./auth-regressions.test.ts
```

For encrypted-authentication checks, update and restart the chosen test instance:

```sh
docker exec db2-wire-test bash -c 'su - db2inst1 -c "db2 update dbm cfg using AUTHENTICATION SERVER_ENCRYPT ALTERNATE_AUTH_ENC NOT_SPECIFIED"; su - db2inst1 -c "db2stop force"; su - db2inst1 -c "db2start"'
cd tests/node
DB2_TEST_ENCRYPTED_AUTH=1 node --import tsx --test ./auth-regressions.test.ts
# For AES_ONLY, update ALTERNATE_AUTH_ENC, restart and run:
DB2_TEST_ENCRYPTED_AUTH=1 DB2_TEST_AES_ONLY=1 node --import tsx --test ./auth-regressions.test.ts
```

Use `DB2_TEST_PORT=50002` and container `db2-wire-test-115` for 11.5. Restore `AUTHENTICATION SERVER ALTERNATE_AUTH_ENC NOT_SPECIFIED` and restart before running the stock-server suites again.
