# DRDA Protocol

db2-node implements the IBM DRDA (Distributed Relational Database Architecture) wire protocol. DRDA is an open standard published by The Open Group that defines how clients communicate with DB2 databases over TCP.

This section provides a deep dive into the protocol internals.

## Protocol Layers

DRDA communication is structured in layers:

```
+--------------------------------------------------+
|  Application (SQL queries, results)               |
+--------------------------------------------------+
|  DDM (Distributed Data Management)                |
|  Command/reply objects with code points            |
+--------------------------------------------------+
|  DSS (Data Stream Structure)                      |
|  Framing: length, type, correlation ID             |
+--------------------------------------------------+
|  TCP / TLS Transport                              |
+--------------------------------------------------+
```

1. **DSS** is the lowest layer — it wraps every message in a 6-byte header with length, type, and correlation ID
2. **DDM** objects live inside DSS frames — they carry commands (EXCSAT, ACCSEC, etc.) and their parameters as nested TLV structures
3. **FD:OCA** defines how result set data (column values) is encoded within DDM reply objects

## Sections

- [DSS Framing](dss-framing.md) — The wire frame format
- [DDM Objects](ddm-objects.md) — Command and reply message structure
- [Connection Flow](connection-flow.md) — The 4-step handshake sequence
- [Query Flow](query-flow.md) — How SELECT, INSERT, UPDATE, DELETE work
- [Data Encoding](data-encoding.md) — FD:OCA, type system, EBCDIC
- [Code Points](code-points.md) — Reference table of all DRDA code points

## Reference Materials

### Specifications
- [DRDA Volume 1: Architecture](https://pubs.opengroup.org/onlinepubs/009608899/toc.pdf) — Overall protocol flows
- [DRDA Volume 2: FD:OCA](https://pubs.opengroup.org/onlinepubs/9699939399/toc.pdf) — Result set data encoding
- [DRDA Volume 3: DDM](https://pubs.opengroup.org/onlinepubs/9690989699/toc.pdf) — Command/reply message format

### Open-Source Implementations
- [Apache Derby](https://github.com/apache/derby) — Java DRDA server + client (`org.apache.derby.impl.drda`)
- [kanrichan/ibm_db](https://github.com/kanrichan/ibm_db/tree/main/drda) — Go DRDA implementation
- Wireshark DRDA dissector — Filter: `drda`
