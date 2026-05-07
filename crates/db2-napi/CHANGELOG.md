# Changelog

All notable changes to this package will be documented in this file.

## [0.1.7-zos.198](https://github.com/gurungabit/db2-node/releases/tag/v0.1.7-zos.198) (2026-05-06)

### Documentation

- document production-ready z/OS LOB cleanup modes, soak validation, and recommended environment flags

## [0.1.7-zos.54](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.54) (2026-05-02)

### Bug Fixes

- fetch z/OS SELECT-star CLOB values in smaller stitched chunks to avoid hangs and truncation

## [0.1.7-zos.53](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.53) (2026-05-02)

### Bug Fixes

- preserve z/OS rewritten CLOB column names and decode ROWID as hex text

## [0.1.7-zos.52](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.52) (2026-05-02)

### Bug Fixes

- restore chained z/OS SELECT execution and auto-expand simple SELECT-star CLOB columns

## [0.1.7-zos.51](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.51) (2026-05-02)

### Bug Fixes

- automatically materialize z/OS CLOB result columns as string values

## [0.1.7-zos.50](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.50) (2026-05-02)

### Bug Fixes

- decode z/OS CLOB locators and ROWID columns without hanging fetches

## [0.1.7-zos.49](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.49) (2026-05-02)

### Bug Fixes

- show z/OS GRAPHIC column metadata as CHAR/VARCHAR while preserving raw Db2 type

## [0.1.7-zos.48](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.48) (2026-05-02)

### Bug Fixes

- apply scanned z/OS SQLDARD names when they match QRYDSC descriptors

## [0.1.7-zos.47](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.47) (2026-05-02)

### Bug Fixes

- extract z/OS column names from repeated DB/table/schema/name SQLDARD pattern

## [0.1.7-zos.46](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.46) (2026-05-02)

### Bug Fixes

- include SQLDARD column-name candidate diagnostics without changing metadata

## [0.1.7-zos.45](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.45) (2026-05-02)

### Bug Fixes

- avoid applying partial z/OS name scans as result metadata

## [0.1.7-zos.44](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.44) (2026-05-02)

### Bug Fixes

- scan z/OS SQLDOPTGRP names without relying on SQLTYPE parsing

## [0.1.7-zos.43](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.43) (2026-05-02)

### Bug Fixes

- prefer real z/OS SQLDARD names over generated COL fallbacks

## [0.1.7-zos.42](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.42) (2026-05-02)

### Bug Fixes

- recover z/OS SQLDARD column names from padded descriptor tables

## [0.1.7-zos.41](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.41) (2026-05-02)

### Bug Fixes

- decode compact z/OS decimal descriptors without shifting row offsets

## [0.1.7-zos.40](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.40) (2026-05-02)

### Bug Fixes

- include z/OS row decode diagnostics for pending QRYDTA blocks

## [0.1.7-zos.39](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.39) (2026-05-02)

### Bug Fixes

- prefer QRYDSC physical descriptors when decoding query row data

## [0.1.7-zos.38](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.38) (2026-05-02)

### Bug Fixes

- decode z/OS graphic row lengths as bytes and prefer standard SQLDARD names

## [0.1.7-zos.37](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.37) (2026-05-02)

### Bug Fixes

- decode compact z/OS QRYDSC numeric row values as big-endian

## [0.1.7-zos.36](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.36) (2026-05-02)

### Bug Fixes

- encode z/OS SQLSTT and SQLATTR as SQLAM 7 nullable single-byte strings

## [0.1.7-zos.35](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.35) (2026-05-02)

### Bug Fixes

- omit malformed SQLATTR from z/OS chained direct SELECT and name query reply diagnostics

## [0.1.7-zos.34](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.34) (2026-05-02)

### Bug Fixes

- include query reply diagnostics in result objects

## [0.1.7-zos.33](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.33) (2026-05-02)

### Bug Fixes

- request a non-empty z/OS query block on direct SELECT open

## [0.1.7-zos.32](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.32) (2026-05-02)

### Bug Fixes

- fetch z/OS query data after sparse open-query replies

## [0.1.7-zos.31](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.31) (2026-05-02)

### Bug Fixes

- decode standard z/OS SQLDARD descriptors and big-endian row values

## [0.1.7-zos.30](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.30) (2026-05-01)

### Bug Fixes

- match JCC z/OS open-query block size for direct SELECT

## [0.1.7-zos.29](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.29) (2026-05-01)

### Bug Fixes

- match JCC-style chained z/OS prepare and open query flow

## [0.1.7-zos.28](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.28) (2026-05-01)

### Bug Fixes

- use z/OS SYSLVL02 package token for dynamic SQL package references

## [0.1.7-zos.27](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.27) (2026-05-01)

### Bug Fixes

- use low z/OS dynamic package section for direct SELECT queries

## [0.1.7-zos.26](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.26) (2026-05-01)

### Bug Fixes

- send z/OS SQLSTT text length without terminator bytes

## [0.1.7-zos.25](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.25) (2026-05-01)

### Bug Fixes

- use documented z/OS SQLATTR statement attribute group

## [0.1.7-zos.24](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.24) (2026-05-01)

### Bug Fixes

- restore last non-dropping z/OS query packet shape

## [0.1.7-zos.23](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.23) (2026-05-01)

### Bug Fixes

- use server-reported z/OS location name in package references

## [0.1.7-zos.22](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.22) (2026-05-01)

### Bug Fixes

- match z/OS SQL open-query block parameters

## [0.1.7-zos.21](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.21) (2026-05-01)

### Bug Fixes

- send z/OS open cursor after prepare reply instead of chaining both commands

## [0.1.7-zos.20](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.20) (2026-05-01)

### Bug Fixes

- use z/OS EBCDIC package names and chained prepare-open cursor flow

## [0.1.7-zos.19](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.19) (2026-05-01)

### Bug Fixes

- send z/OS SQLSTT statement groups in JCC format and drain prepare replies before opening cursors

## [0.1.7-zos.18](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.18) (2026-05-01)

### Bug Fixes

- mark z/OS SELECT prepares as read-only cursors and decode z/OS SQLCODE byte order

## [0.1.7-zos.17](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.17) (2026-05-01)

### Bug Fixes

- skip LUW post-auth initialization for Db2 for z/OS servers

## [0.1.7-zos.16](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.16) (2026-05-01)

### Bug Fixes

- omit ACCRDB CRRTKN for z/OS hosts that reject 0x2135

## [0.1.7-zos.15](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.15) (2026-05-01)

### Bug Fixes

- omit CCSIDCMN from ACCRDB TYPDEFOVR for z/OS hosts that reject 0x1191

## [0.1.7-zos.14](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.14) (2026-05-01)

### Bug Fixes

- encode ACCRDB TYPDEFNAM and related character fields as JCC-compatible UTF-8 bytes

## [0.1.7-zos.13](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.13) (2026-05-01)

### Bug Fixes

- omit ACCRDB TYPDEFNAM/TYPDEFOVR by default for z/OS servers that reject type negotiation

## [0.1.7-zos.12](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.12) (2026-05-01)

### Bug Fixes

- default ACCRDB to QTDSQL370 and expose typeDefinitionName for z/OS TYPDEFNAM testing

## [0.1.7-zos.11](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.11) (2026-05-01)

### Bug Fixes

- use JCC-compatible QTDSQLASC type definition negotiation for z/OS ACCRDB

## [0.1.7-zos.10](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.10) (2026-05-01)

### Bug Fixes

- send z/OS RDBNAM as the trimmed location-name length instead of an 18-byte padded value

## [0.1.7-zos.9](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.9) (2026-05-01)

### Bug Fixes

- default JS encrypted authentication to AES for the current Db2 z/OS/JCC path

## [0.1.7-zos.8](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.8) (2026-05-01)

### Bug Fixes

- keep z/OS RDBNAM reserved bytes blank and lock SECMEC 7 AES to an IBM JCC test vector

## [0.1.7-zos.7](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.7) (2026-05-01)

### Bug Fixes

- force JCC-compatible UTF-8 password plaintext for SECMEC 7 AES authentication

## [0.1.7-zos.6](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.6) (2026-05-01)

### Bug Fixes

- use the server security token IV for SECMEC 7 AES encrypted password authentication

## [0.1.7-zos.5](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.5) (2026-05-01)

### Bug Fixes

- publish prerelease npm builds with the zos dist-tag

## [0.1.7-zos.4](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.4) (2026-05-01)

### Bug Fixes

- align AES encrypted authentication with IBM JCC's 64-byte Diffie-Hellman token and AES key derivation

## [0.1.7-zos.3](https://github.com/gurungabit/db2-node/releases/tag/zos-secmec7-0.1.7-zos.3) (2026-05-01)

### Bug Fixes

- add AES encrypted credential negotiation for DB2 z/OS authentication

## [0.1.6](https://github.com/gurungabit/db2-node/compare/v0.1.5...v0.1.6) (2026-05-01)


### Bug Fixes

* support configurable db2 security mechanism ([cc11e61](https://github.com/gurungabit/db2-node/commit/cc11e61d7316a8ddc0fc04e38fb725c3051af97a))

## [0.1.5](https://github.com/gurungabit/db2-node/compare/v0.1.4...v0.1.5) (2026-05-01)


### Bug Fixes

* omit RDBNAM from SECCHK for zOS ([0ef44bc](https://github.com/gurungabit/db2-node/commit/0ef44bc6eec84b312bd9bec674ebd0b0b354b5dd))

## [0.1.4](https://github.com/gurungabit/db2-node/compare/v0.1.3...v0.1.4) (2026-04-30)


### Bug Fixes

* publish zOS auth fixes through npm package ([025e6c9](https://github.com/gurungabit/db2-node/commit/025e6c99400a6892d1bac72f96c92adfb3fa753c))

## [0.1.3](https://github.com/gurungabit/db2-node/compare/v0.1.2...v0.1.3) (2026-04-30)

### Bug Fixes

- release DB2 zOS authentication support ([2df2565](https://github.com/gurungabit/db2-node/commit/2df256539796156f84aeb046d574b7dfaba98011))

## [0.1.2](https://github.com/gurungabit/db2-node/compare/v0.1.1...v0.1.2) (2026-04-07)

### Bug Fixes

- return numeric DB2 values as JSON numbers, not strings ([8a495d7](https://github.com/gurungabit/db2-node/commit/8a495d78d8bc1116fc09cc026e24406900ce1b05))

## [0.1.1] - 2026-04-06

- Scope the package as `@gurungabit/db2-node`
- Bundle prebuilt native binaries into a single npm package
- Add MIT licensing metadata and package license file
- Add `release-please` automation for future releases
- Refresh docs publishing for the renamed GitHub Pages site

## [0.1.0] - 2026-04-06

- Initial public release of `@gurungabit/db2-node`
