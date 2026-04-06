# DRDA Code Point Reference

Code points are 2-byte identifiers used in DDM objects to identify commands, replies, and parameters. This is a reference of the code points used by db2-node.

## Connection / Handshake

| Code Point | Name | Description |
|-----------|------|-------------|
| `0x1041` | EXCSAT | Exchange Server Attributes |
| `0x1443` | EXSATRD | Exchange Server Attributes Reply Data |
| `0x106D` | ACCSEC | Access Security |
| `0x14AC` | ACCSECRD | Access Security Reply Data |
| `0x106E` | SECCHK | Security Check |
| `0x1219` | SECCHKRM | Security Check Reply Message |
| `0x2001` | ACCRDB | Access RDB (connect to database) |
| `0x2201` | ACCRDBRM | Access RDB Reply Message |

## Parameters

| Code Point | Name | Description |
|-----------|------|-------------|
| `0x115E` | EXTNAM | External Name |
| `0x116D` | SRVNAM | Server Name |
| `0x115A` | SRVRLSLV | Server Product Release Level |
| `0x1147` | SRVCLSNM | Server Class Name |
| `0x1404` | MGRLVLLS | Manager Level List |
| `0x11A2` | SECMEC | Security Mechanism |
| `0x11DC` | SECTKN | Security Token |
| `0x11A0` | USRID | User ID |
| `0x11A1` | PASSWORD | Password |
| `0x2110` | RDBNAM | RDB Name (database name) |
| `0x112E` | PRDID | Product Specific Identifier |
| `0x002F` | TYPDEFNAM | Type Definition Name |
| `0x0035` | TYPDEFOVR | Type Definition Overrides |
| `0x119C` | CCSIDSBC | CCSID Single-Byte Characters |
| `0x119D` | CCSIDDBC | CCSID Double-Byte Characters |
| `0x119E` | CCSIDMBC | CCSID Mixed-Byte Characters |
| `0x210F` | RDBACCCL | RDB Access Manager Class |

## SQL Operations

| Code Point | Name | Description |
|-----------|------|-------------|
| `0x200D` | PRPSQLSTT | Prepare SQL Statement |
| `0x2414` | SQLSTT | SQL Statement (carries SQL text) |
| `0x2412` | SQLDTA | SQL Data (parameter values) |
| `0x200C` | OPNQRY | Open Query (SELECT) |
| `0x2006` | CNTQRY | Continue Query (fetch more rows) |
| `0x2005` | CLSQRY | Close Query |
| `0x200A` | EXCSQLIMM | Execute SQL Immediate |
| `0x200B` | EXCSQLSTT | Execute SQL Statement (prepared) |
| `0x200E` | RDBCMM | RDB Commit |
| `0x200F` | RDBRLLBCK | RDB Rollback |

## Reply Messages

| Code Point | Name | Description |
|-----------|------|-------------|
| `0x2408` | SQLCARD | SQL Communications Area Reply Data |
| `0x2411` | SQLDARD | SQL Descriptor Area Reply Data |
| `0x241B` | QRYDTA | Query Answer Set Data |
| `0x241A` | QRYDSC | Query Answer Set Description |
| `0x1149` | SVRCOD | Severity Code |
| `0x220B` | ENDQRYRM | End of Query Reply Message |
| `0x2205` | OPNQRYRM | Open Query Reply Message |
| `0x2218` | RDBUPDRM | RDB Update Reply Message |
| `0x124C` | SYNTAXRM | Syntax Error Reply Message |
| `0x1245` | PRCCNVRM | Processing Conversion Reply Message |

## Manager Code Points

Used in MGRLVLLS negotiation to declare supported protocol capabilities.

| Code Point | Name | Description |
|-----------|------|-------------|
| `0x1403` | AGENT | Agent (connection) Manager |
| `0x2407` | SQLAM | SQL Application Manager |
| `0x240F` | RDB | Relational Database Manager |
| `0x1440` | SECMGR | Security Manager |
| `0x1474` | CMNTCPIP | TCP/IP Communication Manager |

## Security Mechanisms

| Code | Name | Description |
|------|------|-------------|
| `0x0003` | USRIDPWD | User ID and Password (cleartext) |
| `0x0004` | USRIDONL | User ID only |
| `0x0005` | USRIDNWPWD | User ID with New Password |
| `0x0009` | EUSRIDPWD | Encrypted User ID and Password |

## Severity Codes (SVRCOD)

| Value | Meaning |
|-------|---------|
| `0x0000` | Info — success |
| `0x0004` | Warning |
| `0x0008` | Error |
| `0x0010` | Severe error |
| `0x0014` | Access violation |
| `0x0018` | Critical error |
| `0x001C` | Session terminated |

## Full Reference

The complete list of 200+ code points can be found in:
- Apache Derby [`CodePoint.java`](https://github.com/apache/derby/blob/trunk/java/org.apache.derby.drda/org/apache/derby/impl/drda/CodePoint.java)
- [kanrichan/ibm_db](https://github.com/kanrichan/ibm_db/tree/main/drda) Go implementation
