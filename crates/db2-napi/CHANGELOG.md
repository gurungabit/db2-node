# Changelog

All notable changes to this package will be documented in this file.

## [1.0.27](https://github.com/gurungabit/db2-node/compare/v1.0.26...v1.0.27) (2026-10-05)

db2-node 1.0.27 fixes four driver bugs found while benchmarking 1.0.26.

- Commit `executeBatch` under autocommit. Rows from an autocommit batch previously stayed uncommitted, and invisible to other connections, until something else committed on the connection. The batch now commits once every row succeeds; if any row fails, it rolls back and throws the first error, so no partial rows are kept. Batches inside a transaction are unchanged. (PR #35)
- Allow statements with more than 84 parameters. Parameter descriptors beyond the 84th now continue in FD:OCA CPT triplets, the layout Db2 uses for wide result sets, instead of failing with "too many parameters for SQLDTA descriptor header". (PR #35)
- Bind whole JavaScript numbers up to `Number.MAX_SAFE_INTEGER` as integers, so values such as `606227179000` are accepted for BIGINT parameters. (PR #35)
- Retry the padded RDBNAM when Db2 12.1 closes the socket after rejecting the trimmed form, fixing intermittent "RDB not accessed or database not found" connection failures. (PR #35)

Validation covers Db2 LUW 12.1 and 11.5 and TLS on 12.1. No live z/OS server was available.

## [1.0.26](https://github.com/gurungabit/db2-node/compare/v1.0.25...v1.0.26) (2026-10-05)

db2-node 1.0.26 fixes LOB parameters that were silently not written and removes the fixed reply waits that dominated request latency on Db2 LUW.

- Write parameters bound to CLOB, BLOB and DBCLOB columns declared larger than 32,767 bytes. They are now sent as DRDA LOB placeholders with EXTDTA, instead of oversized VARCHAR descriptors that Db2 LUW rejected while the statement reported `rowCount: 0`. Direct and prepared execution, batches, CALL, NULL and empty values are covered. A rejected descriptor (DSCINVRM) now raises an error instead of reporting success. (#31; PR #32)
- Accept XML parameters larger than 32,767 bytes on Db2 LUW. (PR #32)
- Pass `Buffer`, typed array and `ArrayBuffer` parameters to the native addon without per-byte conversion. A 50 MB BLOB write drops from about 3 s to under 0.5 s and no longer blocks the event loop. (PR #32)
- Complete Db2 LUW replies at the end of the DRDA reply chain. The driver no longer waits `frameDrainTimeout` (25 ms) after each reply, or 250 ms before fetching results with LOB columns. On Db2 12.1, `VALUES 1` drops from about 30 ms to 0.5 ms and a LOB SELECT from about 285 ms to 1 ms. `frameDrainTimeout` now applies only to Db2 for z/OS. (PR #34)

Validation covers Db2 LUW 12.1 and 11.5, TLS on 12.1, and the full Rust and Node suites run through a DRDA protocol audit. No live z/OS server was available; z/OS keeps its existing reply handling.

## [1.0.25](https://github.com/gurungabit/db2-node/compare/v1.0.24...v1.0.25) (2026-10-04)

db2-node 1.0.25 resolves seven reported issues across LOB decoding, JavaScript query APIs, bundled TLS libraries, and credential encoding.

- Preserve mixed CLOB/BLOB values, Unicode/DBCLOB values, NULL rows, and wide result sets across direct and prepared fetches. Descriptor-sized external LOB references remain supported, and chained replies share the configured fetch deadline or the existing 30-second fallback. (#19, #20; PR #27)
- Add opt-in `{ rowMode: 'array' }` to retain duplicate result columns across client, pool, prepared, transaction, CALL, and compatibility APIs. Object rows remain the default; TypeScript accurately distinguishes omitted, array, and dynamic options. (#21; PR #28)
- Accept the documented BOOLEAN text aliases and provide additive `driverCode` classifications while preserving existing error status/messages, server diagnostics, and protocol retryability. Invalid options cannot masquerade as server SQL errors. (#22, #23; PR #28)
- Rebuild the native binaries with patched rustls 0.23.45 and rustls-webpki 0.103.15. (#24; PR #26)
- Negotiate credential encoding for LUW passwords containing punctuation, including `! ^ [ ] |`, with the identified LUW IBM CLI conversion profile. Explicit encoding overrides, z/OS behavior, and authentication protections are preserved. (#25; PR #29)

Independent review findings were corrected and rechecked. Combined validation covers Db2 LUW 12.1 and 11.5, TLS on 12.1, strict TypeScript, Rust/protocol tests, CJS/ESM and compatibility APIs, and disposable-user credential tests. No live z/OS server was available; synthetic z/OS protocol regressions pass.


## [1.0.24](https://github.com/gurungabit/db2-node/compare/v1.0.22...v1.0.24) (2026-10-03)

### Release Fix

- Build Linux ARM64 musl on a native ARM64 runner using the existing multi-platform Alpine image. This avoids the Zig cross-linker incompatibility with Rust 1.99 while retaining all eight native binaries and the publication checks.
- This is the first npm publication of the fixes documented under 1.0.23 below. The v1.0.23 tag was created, but its native-build failure prevented npm publication.

### Included Bug Fixes

- Resolve [#11](https://github.com/gurungabit/db2-node/issues/11) and all 21 cases in [#12](https://github.com/gurungabit/db2-node/issues/12), including UTF-8 and row decoding, LOBs, lossless BIGINT values, CALL results, parameter validation and encrypted authentication.
- Preserve wide-row DSS boundaries when TCP combines replies; apply currentSchema and support bounded LUW cancellation.
- Verify TCP SQL execution after the CI Db2 TLS restart before starting database tests.

### Compatibility and Validation

- Unsafe BIGINT results return decimal strings; LUW accepts only an omitted typeDefinitionName or QTDSQLX86. Stock AUTHENTICATION=SERVER requires explicit userPassword, TLS or encrypted server authentication. Cancellation requires monitoring and WLM_CANCEL_ACTIVITY privileges.
- Validated against Docker Db2 12.1 and 11.5 with Rust, Node and Bun. No live z/OS server was tested. See the [behavior matrix](https://github.com/gurungabit/db2-node/blob/main/tests/ISSUES-11-12.md) and the detailed 1.0.23 notes below.

## [1.0.23](https://github.com/gurungabit/db2-node/compare/v1.0.22...v1.0.23) (2026-10-03)

### Bug Fixes

- Resolve [#11](https://github.com/gurungabit/db2-node/issues/11) and all 21 cases in [#12](https://github.com/gurungabit/db2-node/issues/12), including the follow-up report.
- Correct LUW UTF-8 text, mixed numeric and Boolean decoding, timestamp widths, NULL/LOB row handling, and continued wide-row DSS boundaries when TCP combines replies.
- Preserve BIGINT values outside JavaScript's safe integer range and accept bigint, Date, Buffer, Uint8Array and ArrayBuffer parameters. Reject invalid nested objects, byte arrays and malformed or overflowing DECIMAL input with catchable errors.
- Execute CALL with multiple result sets and OUT/INOUT parameters across direct, prepared, transaction and compatibility APIs. Handle leading SQL comments, zero-row updates and truncation warnings correctly.
- Repair AES/DES encrypted authentication, refuse implicit plaintext downgrade, retain encryption settings on retries, and use OS randomness for DH private keys.
- Apply currentSchema, return the actual server product name, and support bounded LUW cancellation and queryTimeout.
- Preserve z/OS DECIMAL input precision/scale and strengthen native artifact and publication checks.

### Compatibility

- Unsafe BIGINT results now return exact decimal strings. LUW accepts only an omitted typeDefinitionName or QTDSQLX86.
- Stock AUTHENTICATION=SERVER requires an explicit userPassword mechanism, TLS, or encrypted server authentication. Date parameters use UTC.
- LUW cancellation requires activity-monitoring privileges and permission to execute SYSPROC.WLM_CANCEL_ACTIVITY.

### Validation

- Verified against Docker Db2 12.1.0.0 and 11.5.9.0, including all issue regressions, TLS, wide-row reply coalescing, and repeated encrypted-authentication sessions.
- Rust, Node and Bun checks passed. No live z/OS server was tested for this release. See the [behavior matrix and reproduction commands](https://github.com/gurungabit/db2-node/blob/main/tests/ISSUES-11-12.md).

## [1.0.22](https://github.com/gurungabit/db2-node/releases/tag/v1.0.22) (2026-06-18)

### Bug Fixes

- parse Db2 for z/OS FD:OCA metadata-definition triplets and environmental local identifiers according to their declared scalar types and CCSIDs
- resolve compact GDA column references through the descriptor environment instead of inferring character columns from LOB local-identifier byte patterns
- validate the real MDD/SDA/GDA descriptor shape through the full parameterized query-frame decoder

## [1.0.21](https://github.com/gurungabit/db2-node/releases/tag/v1.0.21) (2026-06-18)

### Bug Fixes

- restore Db2 for z/OS late FD:OCA descriptor decoding for fixed and varying EBCDIC text columns
- apply compact GDA per-column length overrides and prefer the complete row layout, preventing parameterized multi-column queries from being decoded as a single `LOBBYTES(4096)` value
- cover the reported 1,482-byte multirow tail through the full QRYDSC/QRYDTA query-frame path

## [1.0.20](https://github.com/gurungabit/db2-node/releases/tag/v1.0.20) (2026-06-18)

### Bug Fixes

- preserve SQLDARD result descriptors for immediate result-set replies instead of decoding ordinary rows through a one-column QRYDSC LOB envelope
- prefer multi-column SQLDARD row layouts when z/OS exposes a single `LOBBYTES` or `LOBCHAR` QRYDSC envelope, preventing complete row blocks from being reported as undecoded LOB tails

## [1.0.19](https://github.com/gurungabit/db2-node/releases/tag/v1.0.19) (2026-05-14)

### Bug Fixes

- send `QRYROWSET` with native Db2 for z/OS LOB `RTNEXTALL` continuation fetches, matching the DRDA shape required for full CLOB cursor reads
- keep native z/OS LOB continuation fetches on the configured fetch-size rowset with extended blocks enabled by default for throughput

## [1.0.18](https://github.com/gurungabit/db2-node/releases/tag/v1.0.18) (2026-05-14)

### Bug Fixes

- keep Db2 for z/OS CLOB continuation fetches on the native LOB path while using conservative `CNTQRY` framing by default, avoiding z/OS `SYNTAXRM` failures during long full-CLOB scans
- add `DB2_ZOS_NATIVE_LOB_CNTQRY_EXTRA_BLOCKS=1` as an opt-in diagnostic/performance override for native LOB continuation fetches

## [1.0.17](https://github.com/gurungabit/db2-node/releases/tag/v1.0.17) (2026-05-14)

### Bug Fixes

- use the native Db2 for z/OS LOB cursor path before SQL `SUBSTR` materialization so full CLOB reads behave closer to `ibm_db`
- actively close native z/OS LOB cursors after materialization by default, avoiding reconnect-per-query overhead in repeated CLOB read loops
- keep SQL chunk materialization available as a fallback and via `DB2_ZOS_LOB_STRATEGY=sql`

## [1.0.16](https://github.com/gurungabit/db2-node/releases/tag/v1.0.16) (2026-05-14)

### Bug Fixes

- fetch z/OS CLOB materialization chunks one source row at a time so large `IN (...)` batches do not request 100 wide CLOB-derived rows in a single cursor fetch
- keep internal z/OS CLOB materialization on conservative 32 KB cursor blocks by default, avoiding `SYNTAXRM` failures during generated CLOB chunk fetches

## [1.0.15](https://github.com/gurungabit/db2-node/releases/tag/v1.0.15) (2026-05-13)

### Bug Fixes

- keep Db2 for z/OS CLOB materialization fetch rows narrow by fetching one 16 KB CLOB chunk per generated row
- avoid z/OS DRDA `SYNTAXRM` failures when full CLOB reads require multiple internal chunks for the same application query

## [1.0.14](https://github.com/gurungabit/db2-node/releases/tag/v1.0.14) (2026-05-13)

### Bug Fixes

- preserve bare and qualified projection column names such as `r.INSP_RPT_ID` when Db2 for z/OS returns generated names like `COL1`
- prevent generated `COL1` result keys from causing application-built `IN (...)` lists to contain empty values during follow-up CLOB scans

## [1.0.13](https://github.com/gurungabit/db2-node/releases/tag/v1.0.13) (2026-05-13)

### Bug Fixes

- route simple Db2 for z/OS CLOB `SELECT` queries through chunked materialization before opening the native LOB cursor, avoiding stale cursor failures such as `SQLCODE=-514`
- retry catalog-driven z/OS CLOB recovery directly after retryable session errors instead of falling back to the same failing native cursor path
- recognize z/OS servers by `server_class` values containing `z/OS` or `MVS` when `server_release` is generic

## [1.0.12](https://github.com/gurungabit/db2-node/releases/tag/v1.0.12) (2026-05-13)

### Bug Fixes

- recover plain Db2 for z/OS CLOB read queries that hit stale cursor or statement section errors by reconnecting and switching to catalog-driven chunked materialization
- retry internal z/OS CLOB materialization queries after stale session errors such as `SQLCODE=-502`, `SQLCODE=-514`, and `SQLCODE=-518`

## [1.0.11](https://github.com/db2-node/db2-node/releases/tag/v1.0.11) (2026-05-12)

### Bug Fixes

- retry unparameterized read queries after stale Db2 for z/OS cursor or statement section errors such as `SQLCODE=-502`, `SQLCODE=-514`, and `SQLCODE=-518`
- reset stale z/OS sessions before they can be returned to a pool when stale cursor errors escape retry handling
- recognize wrapped stale cursor errors from internal z/OS CLOB materialization queries

### Compatibility

- expose `sqlstate`, `sqlcode`, and `retryable` metadata on JavaScript errors when DB2 error details can be inferred

## [1.0.10](https://github.com/db2-node/db2-node/releases/tag/v1.0.10) (2026-05-12)

### Bug Fixes

- apply explicit `SELECT ... AS ...` aliases to final row objects when Db2 for z/OS returns generated expression names such as `COL1`, `COL2`, and `COL3`

## [1.0.9](https://github.com/db2-node/db2-node/releases/tag/v1.0.9) (2026-05-12)

### Bug Fixes

- route Db2 for z/OS non-LOB `SELECT` results with `LIKE` predicates through the large statement package `EXCSQLSTT` path, avoiding the direct cursor/package path that can trip CLOB predicate resource limits
- preserve explicit `SELECT ... AS ...` aliases when Db2 metadata falls back to generated names such as `COL1`
- decode mixed SQLERRMC token separators in z/OS SQL errors, including readable `SQLCODE=-905` / `SQLSTATE=57014` details

## [1.0.8](https://github.com/db2-node/db2-node/releases/tag/v1.0.8) (2026-05-12)

### Compatibility

- make the CommonJS entry point callable like `ibm_db`
- make `new ibmdb.Pool()` work without connection config and support `open`, `init`, `initAsync`, and pool sizing APIs
- keep the async native-style pool path available while exposing the explicit `Db2Pool` constructor
- normalize IBM-style connection strings and connection objects through the compatibility API

### Documentation

- document selected `ibm_db` migration entry points
- replace domain-specific fixture identifiers with neutral randomized test values

## [1.0.7](https://github.com/db2-node/db2-node/releases/tag/v1.0.7) (2026-05-08)

### Release

- restore the validated Rust DRDA driver line as the official `db2-node` package
- publish the unscoped `db2-node` npm package name
- include VitePress documentation and expanded API reference coverage
- support the extended Db2 scalar and LOB data type mapping used by DB2 LUW and Db2 for z/OS

## [1.0.0](https://github.com/db2-node/db2-node/releases/tag/v1.0.0) (2026-05-06)

### Release

- promote the validated DB2 LUW and Db2 for z/OS driver line to the first production release
- publish stable npm installs on the `latest` dist-tag
- document default and active-close z/OS LOB production modes
- remove stale checked-in npm tarballs from future GitHub release artifacts

## [0.1.7-zos.198](https://github.com/db2-node/db2-node/releases/tag/v0.1.7-zos.198) (2026-05-06)

### Documentation

- document production-ready z/OS LOB cleanup modes, soak validation, and recommended environment flags

## [0.1.7-zos.54](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.54) (2026-05-02)

### Bug Fixes

- fetch z/OS SELECT-star CLOB values in smaller stitched chunks to avoid hangs and truncation

## [0.1.7-zos.53](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.53) (2026-05-02)

### Bug Fixes

- preserve z/OS rewritten CLOB column names and decode ROWID as hex text

## [0.1.7-zos.52](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.52) (2026-05-02)

### Bug Fixes

- restore chained z/OS SELECT execution and auto-expand simple SELECT-star CLOB columns

## [0.1.7-zos.51](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.51) (2026-05-02)

### Bug Fixes

- automatically materialize z/OS CLOB result columns as string values

## [0.1.7-zos.50](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.50) (2026-05-02)

### Bug Fixes

- decode z/OS CLOB locators and ROWID columns without hanging fetches

## [0.1.7-zos.49](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.49) (2026-05-02)

### Bug Fixes

- show z/OS GRAPHIC column metadata as CHAR/VARCHAR while preserving raw Db2 type

## [0.1.7-zos.48](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.48) (2026-05-02)

### Bug Fixes

- apply scanned z/OS SQLDARD names when they match QRYDSC descriptors

## [0.1.7-zos.47](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.47) (2026-05-02)

### Bug Fixes

- extract z/OS column names from repeated DB/table/schema/name SQLDARD pattern

## [0.1.7-zos.46](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.46) (2026-05-02)

### Bug Fixes

- include SQLDARD column-name candidate diagnostics without changing metadata

## [0.1.7-zos.45](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.45) (2026-05-02)

### Bug Fixes

- avoid applying partial z/OS name scans as result metadata

## [0.1.7-zos.44](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.44) (2026-05-02)

### Bug Fixes

- scan z/OS SQLDOPTGRP names without relying on SQLTYPE parsing

## [0.1.7-zos.43](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.43) (2026-05-02)

### Bug Fixes

- prefer real z/OS SQLDARD names over generated COL fallbacks

## [0.1.7-zos.42](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.42) (2026-05-02)

### Bug Fixes

- recover z/OS SQLDARD column names from padded descriptor tables

## [0.1.7-zos.41](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.41) (2026-05-02)

### Bug Fixes

- decode compact z/OS decimal descriptors without shifting row offsets

## [0.1.7-zos.40](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.40) (2026-05-02)

### Bug Fixes

- include z/OS row decode diagnostics for pending QRYDTA blocks

## [0.1.7-zos.39](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.39) (2026-05-02)

### Bug Fixes

- prefer QRYDSC physical descriptors when decoding query row data

## [0.1.7-zos.38](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.38) (2026-05-02)

### Bug Fixes

- decode z/OS graphic row lengths as bytes and prefer standard SQLDARD names

## [0.1.7-zos.37](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.37) (2026-05-02)

### Bug Fixes

- decode compact z/OS QRYDSC numeric row values as big-endian

## [0.1.7-zos.36](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.36) (2026-05-02)

### Bug Fixes

- encode z/OS SQLSTT and SQLATTR as SQLAM 7 nullable single-byte strings

## [0.1.7-zos.35](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.35) (2026-05-02)

### Bug Fixes

- omit malformed SQLATTR from z/OS chained direct SELECT and name query reply diagnostics

## [0.1.7-zos.34](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.34) (2026-05-02)

### Bug Fixes

- include query reply diagnostics in result objects

## [0.1.7-zos.33](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.33) (2026-05-02)

### Bug Fixes

- request a non-empty z/OS query block on direct SELECT open

## [0.1.7-zos.32](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.32) (2026-05-02)

### Bug Fixes

- fetch z/OS query data after sparse open-query replies

## [0.1.7-zos.31](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.31) (2026-05-02)

### Bug Fixes

- decode standard z/OS SQLDARD descriptors and big-endian row values

## [0.1.7-zos.30](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.30) (2026-05-01)

### Bug Fixes

- match JCC z/OS open-query block size for direct SELECT

## [0.1.7-zos.29](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.29) (2026-05-01)

### Bug Fixes

- match JCC-style chained z/OS prepare and open query flow

## [0.1.7-zos.28](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.28) (2026-05-01)

### Bug Fixes

- use z/OS SYSLVL02 package token for dynamic SQL package references

## [0.1.7-zos.27](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.27) (2026-05-01)

### Bug Fixes

- use low z/OS dynamic package section for direct SELECT queries

## [0.1.7-zos.26](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.26) (2026-05-01)

### Bug Fixes

- send z/OS SQLSTT text length without terminator bytes

## [0.1.7-zos.25](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.25) (2026-05-01)

### Bug Fixes

- use documented z/OS SQLATTR statement attribute group

## [0.1.7-zos.24](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.24) (2026-05-01)

### Bug Fixes

- restore last non-dropping z/OS query packet shape

## [0.1.7-zos.23](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.23) (2026-05-01)

### Bug Fixes

- use server-reported z/OS location name in package references

## [0.1.7-zos.22](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.22) (2026-05-01)

### Bug Fixes

- match z/OS SQL open-query block parameters

## [0.1.7-zos.21](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.21) (2026-05-01)

### Bug Fixes

- send z/OS open cursor after prepare reply instead of chaining both commands

## [0.1.7-zos.20](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.20) (2026-05-01)

### Bug Fixes

- use z/OS EBCDIC package names and chained prepare-open cursor flow

## [0.1.7-zos.19](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.19) (2026-05-01)

### Bug Fixes

- send z/OS SQLSTT statement groups in JCC format and drain prepare replies before opening cursors

## [0.1.7-zos.18](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.18) (2026-05-01)

### Bug Fixes

- mark z/OS SELECT prepares as read-only cursors and decode z/OS SQLCODE byte order

## [0.1.7-zos.17](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.17) (2026-05-01)

### Bug Fixes

- skip LUW post-auth initialization for Db2 for z/OS servers

## [0.1.7-zos.16](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.16) (2026-05-01)

### Bug Fixes

- omit ACCRDB CRRTKN for z/OS hosts that reject 0x2135

## [0.1.7-zos.15](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.15) (2026-05-01)

### Bug Fixes

- omit CCSIDCMN from ACCRDB TYPDEFOVR for z/OS hosts that reject 0x1191

## [0.1.7-zos.14](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.14) (2026-05-01)

### Bug Fixes

- encode ACCRDB TYPDEFNAM and related character fields as JCC-compatible UTF-8 bytes

## [0.1.7-zos.13](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.13) (2026-05-01)

### Bug Fixes

- omit ACCRDB TYPDEFNAM/TYPDEFOVR by default for z/OS servers that reject type negotiation

## [0.1.7-zos.12](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.12) (2026-05-01)

### Bug Fixes

- default ACCRDB to QTDSQL370 and expose typeDefinitionName for z/OS TYPDEFNAM testing

## [0.1.7-zos.11](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.11) (2026-05-01)

### Bug Fixes

- use JCC-compatible QTDSQLASC type definition negotiation for z/OS ACCRDB

## [0.1.7-zos.10](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.10) (2026-05-01)

### Bug Fixes

- send z/OS RDBNAM as the trimmed location-name length instead of an 18-byte padded value

## [0.1.7-zos.9](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.9) (2026-05-01)

### Bug Fixes

- default JS encrypted authentication to AES for the current Db2 z/OS/JCC path

## [0.1.7-zos.8](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.8) (2026-05-01)

### Bug Fixes

- keep z/OS RDBNAM reserved bytes blank and lock SECMEC 7 AES to an IBM JCC test vector

## [0.1.7-zos.7](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.7) (2026-05-01)

### Bug Fixes

- force JCC-compatible UTF-8 password plaintext for SECMEC 7 AES authentication

## [0.1.7-zos.6](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.6) (2026-05-01)

### Bug Fixes

- use the server security token IV for SECMEC 7 AES encrypted password authentication

## [0.1.7-zos.5](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.5) (2026-05-01)

### Bug Fixes

- publish prerelease npm builds with the zos dist-tag

## [0.1.7-zos.4](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.4) (2026-05-01)

### Bug Fixes

- align AES encrypted authentication with IBM JCC's 64-byte Diffie-Hellman token and AES key derivation

## [0.1.7-zos.3](https://github.com/db2-node/db2-node/releases/tag/zos-secmec7-0.1.7-zos.3) (2026-05-01)

### Bug Fixes

- add AES encrypted credential negotiation for DB2 z/OS authentication

## [0.1.6](https://github.com/db2-node/db2-node/compare/v0.1.5...v0.1.6) (2026-05-01)


### Bug Fixes

* support configurable db2 security mechanism ([cc11e61](https://github.com/db2-node/db2-node/commit/cc11e61d7316a8ddc0fc04e38fb725c3051af97a))

## [0.1.5](https://github.com/db2-node/db2-node/compare/v0.1.4...v0.1.5) (2026-05-01)


### Bug Fixes

* omit RDBNAM from SECCHK for zOS ([0ef44bc](https://github.com/db2-node/db2-node/commit/0ef44bc6eec84b312bd9bec674ebd0b0b354b5dd))

## [0.1.4](https://github.com/db2-node/db2-node/compare/v0.1.3...v0.1.4) (2026-04-30)


### Bug Fixes

* publish zOS auth fixes through npm package ([025e6c9](https://github.com/db2-node/db2-node/commit/025e6c99400a6892d1bac72f96c92adfb3fa753c))

## [0.1.3](https://github.com/db2-node/db2-node/compare/v0.1.2...v0.1.3) (2026-04-30)

### Bug Fixes

- release DB2 zOS authentication support ([2df2565](https://github.com/db2-node/db2-node/commit/2df256539796156f84aeb046d574b7dfaba98011))

## [0.1.2](https://github.com/db2-node/db2-node/compare/v0.1.1...v0.1.2) (2026-04-07)

### Bug Fixes

- return numeric DB2 values as JSON numbers, not strings ([8a495d7](https://github.com/db2-node/db2-node/commit/8a495d78d8bc1116fc09cc026e24406900ce1b05))

## [0.1.1] - 2026-04-06

- Scope the package as `db2-node`
- Bundle prebuilt native binaries into a single npm package
- Add MIT licensing metadata and package license file
- Add `release-please` automation for future releases
- Refresh docs publishing for the renamed GitHub Pages site

## [0.1.0] - 2026-04-06

- Initial public release of `db2-node`
