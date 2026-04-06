# Query Execution Flows

This page describes the DRDA message sequences for different SQL operations.

## Simple Query (Non-prepared)

Use EXCSQLIMM for statements without parameters: `CREATE TABLE`, `INSERT INTO ... VALUES (...)`, `DROP`, etc.

```
Client                                    DB2
  |--- EXCSQLIMM + SQLSTT ----------------->|
  |    (SQL text in SQLSTT object)          |
  |                                         |
  |<-- SQLCARD -----------------------------|  (rows affected, SQLSTATE)
```

The SQL text is carried in a separate SQLSTT object DSS (type=Object) that follows the EXCSQLIMM request DSS.

## Prepared Statement with SELECT

```
Client                                    DB2
  |--- PRPSQLSTT + SQLSTT ----------------->|  Prepare
  |                                         |
  |<-- SQLDARD ------------------------------|  Column descriptions
  |                                         |
  |--- OPNQRY ----------------------------->|  Open cursor (execute)
  |    (+ SQLDTA if parameters)             |
  |                                         |
  |<-- OPNQRYRM + QRYDSC + QRYDTA ----------|  First batch of rows
  |                                         |
  |--- CNTQRY ----------------------------->|  Fetch more rows
  |                                         |
  |<-- QRYDTA ------------------------------|  More rows
  |    ...repeat until...                   |
  |<-- ENDQRYRM ----------------------------|  No more rows
  |                                         |
  |--- CLSQRY ----------------------------->|  Close cursor
```

### SQLDARD (SQL Descriptor Area Reply Data)

The SQLDARD reply from PRPSQLSTT contains column metadata:
- Number of columns
- For each column: name, type, length, precision, scale, nullability

### QRYDSC (Query Answer Set Description)

Describes how row data is encoded in QRYDTA:
- FD:OCA triplets defining the data format
- Column order and encoding details

### QRYDTA (Query Answer Set Data)

Contains the actual row data. See [Data Encoding](data-encoding.md) for details on how rows are encoded.

### CNTQRY (Continue Query)

When the initial QRYDTA doesn't contain all rows, send CNTQRY to fetch more. The `fetchSize` configuration controls how many rows DB2 returns per batch.

### ENDQRYRM (End of Query)

Signals that no more rows are available. The cursor may be auto-closed by the server.

## Prepared Statement with INSERT/UPDATE/DELETE

```
Client                                    DB2
  |--- PRPSQLSTT + SQLSTT ----------------->|  Prepare
  |                                         |
  |<-- SQLDARD ------------------------------|  Parameter descriptions
  |                                         |
  |--- EXCSQLSTT + SQLDTA ----------------->|  Execute with param values
  |                                         |
  |<-- SQLCARD ------------------------------|  Rows affected
```

### SQLDTA (SQL Data)

SQLDTA carries parameter values for prepared statements. Parameters are encoded according to their DB2 types as described in the SQLDARD parameter descriptors.

## Transaction Control

### Commit

```
Client                   DB2
  |--- RDBCMM ------------>|
  |<-- SQLCARD -------------|
```

### Rollback

```
Client                   DB2
  |--- RDBRLLBCK ---------->|
  |<-- SQLCARD -------------|
```

## Error Handling

When a SQL error occurs, the server returns a SQLCARD with:
- `SQLCODE` — Numeric error code (negative = error, positive = warning, 0 = success)
- `SQLSTATE` — 5-character state code (e.g., "42601" for syntax error)
- `SQLERRMC` — Error message tokens separated by `0xFF` bytes

For protocol-level errors, the server returns specific reply messages:
- `SYNTAXRM` — DRDA syntax error in the request
- `PRCCNVRM` — Processing conversion error
- `SQLERRRM` — SQL error reply
