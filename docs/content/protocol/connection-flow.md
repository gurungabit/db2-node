---
title: "Connection Flow"
weight: 3
---

# Connection Handshake Flow

Establishing a DRDA connection requires a 4-step handshake. db2-node optimizes this to just 2 TCP round trips using DSS chaining.

## Full Handshake Sequence

```
Client (AR)                          DB2 Server (AS)
    |                                       |
    |--- EXCSAT ------------------------------>|  Step 1: Exchange attributes
    |    (EXTNAM, SRVNAM, SRVRLSLV,        |
    |     SRVCLSNM, MGRLVLLS)             |
    |                                       |
    |<-- EXSATRD ------------------------------|  Server responds with its
    |    (EXTNAM, SRVNAM, SRVRLSLV,        |  attributes
    |     SRVCLSNM, MGRLVLLS)             |
    |                                       |
    |--- ACCSEC ------------------------------>|  Step 2: Negotiate security
    |    (SECMEC=0x0003, RDBNAM)           |  mechanism
    |                                       |
    |<-- ACCSECRD -----------------------------|  Server confirms mechanism
    |    (SECMEC, SECTKN if needed)        |
    |                                       |
    |--- SECCHK ------------------------------>|  Step 3: Send credentials
    |    (SECMEC, USRID, PASSWORD)         |
    |                                       |
    |<-- SECCHKRM -----------------------------|  Auth result
    |    (SVRCOD=0x0000 = success)         |
    |                                       |
    |--- ACCRDB ------------------------------>|  Step 4: Access the database
    |    (RDBNAM, PRDID, TYPDEFNAM,        |
    |     TYPDEFOVR, CCSID*)               |
    |                                       |
    |<-- ACCRDBRM -----------------------------|  Database accessed
    |    (SVRCOD, PRDID, TYPDEFNAM,        |
    |     TYPDEFOVR)                       |
    |                                       |
    |    === Connection established ===     |
```

## Optimized Flow (2 Round Trips)

Steps 1+2 and 3+4 are chained using DSS chaining to minimize TCP round trips:

```
Write 1: [DSS(EXCSAT, chained=true)] [DSS(ACCSEC, chained=false)]
Read 1:  [DSS(EXSATRD)] [DSS(ACCSECRD)]

Write 2: [DSS(SECCHK, chained=true)] [DSS(ACCRDB, chained=false)]
Read 2:  [DSS(SECCHKRM)] [DSS(ACCRDBRM)]
```

## Step 1: Exchange Server Attributes (EXCSAT)

The client identifies itself and negotiates protocol capabilities.

**Client sends EXCSAT with**:
- `EXTNAM` — External name (e.g., "db2-node-client")
- `SRVNAM` — Server name identifier
- `SRVRLSLV` — Product release level (e.g., "db2node00100")
- `SRVCLSNM` — Server class name
- `MGRLVLLS` — Manager level list (capability negotiation)

**Server replies with EXSATRD** containing its own attributes and supported manager levels.

## Step 2: Access Security (ACCSEC)

Negotiate which authentication mechanism to use.

**Client sends ACCSEC with**:
- `SECMEC` — Security mechanism (0x0003 = cleartext userid/password)
- `RDBNAM` — Database name (may need EBCDIC encoding)

**Server replies with ACCSECRD** confirming the mechanism.

### Security Mechanisms

| Code | Mechanism | Description |
|------|-----------|-------------|
| `0x0003` | USRIDPWD | Cleartext user ID + password |
| `0x0004` | USRIDONL | User ID only (trusted) |
| `0x0009` | EUSRIDPWD | Encrypted user ID + password |

db2-node uses `USRIDPWD` (0x0003) by default. When TLS is enabled, credentials are encrypted at the transport layer.

## Step 3: Security Check (SECCHK)

Send the actual credentials.

**Client sends SECCHK with**:
- `SECMEC` — Same mechanism as ACCSEC
- `USRID` — User ID
- `PASSWORD` — Password

**Server replies with SECCHKRM** containing:
- `SVRCOD` — Severity code (0x0000 = success, 0x0008 = error)

## Step 4: Access RDB (ACCRDB)

Connect to the specific database and negotiate data encoding.

**Client sends ACCRDB with**:
- `RDBNAM` — Database name
- `PRDID` — Product identifier (e.g., "JCC04200" for compatibility)
- `TYPDEFNAM` — Type definition name (e.g., "QTDSQLX86" for x86 systems)
- `TYPDEFOVR` — Type definition overrides (CCSID settings)
- `CCSIDSBC` — Single-byte CCSID (1208 for UTF-8)
- `CCSIDDBC` — Double-byte CCSID (1200 for UTF-16)
- `CCSIDMBC` — Mixed-byte CCSID (1208 for UTF-8)

**Server replies with ACCRDBRM** confirming database access.

## Common Pitfalls

1. **RDBNAM encoding** — Some DB2 servers expect EBCDIC-encoded database names even after negotiating UTF-8
2. **RDBNAM padding** — The database name must be padded to 18 bytes with EBCDIC spaces (0x40) in some contexts
3. **Product ID** — Identifying as "JCC" (IBM's Java driver) via PRDID enables maximum compatibility
4. **TYPDEFNAM** — Use "QTDSQLX86" for DB2 LUW (Linux/Unix/Windows), "QTDSQLBC" for z/OS
