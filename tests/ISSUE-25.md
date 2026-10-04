# Credential encoding regression (#25)

The server's DRDA manager negotiation selects credential encoding independently
of the database's SQL data CCSIDs. [IBM documents that DRDA credentials use
CCSID 500 unless Unicode is negotiated](https://www.ibm.com/support/pages/drda-user-id-and-password-not-being-transmitted-correctly-when-containing-characters-%C2%AC-%C2%A2).
The standard IBM-500 mapping is also implemented by [Apache Derby's DRDA
CCSID manager](https://github.com/apache/derby/blob/trunk/java/org.apache.derby.client/org/apache/derby/client/net/EbcdicCcsidManager.java).

Observed with disposable users on the existing Linux test containers:

| Server | EXSATRD release | UNICODEMGR reply | Selected SECMEC |
| --- | --- | --- | --- |
| Db2 12.1.0.0 | SQL12010 | 0 | 3 |
| Db2 11.5.9.0 | SQL11059 | 0 | 3 |

Local CLP accepts each reported punctuation password. A raw SECCHK with the
caret password encoded as 037 returns `SVRCOD=8, SECCHKCD=0x0F`; the same
credentials encoded as 500 return `SVRCOD=0, SECCHKCD=0`. UTF-8 is rejected
without Unicode negotiation.

Standard IBM-500 alone does not handle ASCII `|` correctly on these LUW servers.
A successful remote **IBM CLI** connection on each version sends the password
`Abc123!^[]|x` as:

```text
c1 82 83 f1 f2 f3 4f 5f 4a 5a 6a a7
```

Thus LUW's native client uses the ASCII-compatible conversion with `|` at
`0x6A`, rather than standard IBM-500's `0xBB`. Raw SECCHK confirms that the
server accepts `0x6A` and rejects `0xBB` for that ASCII password. The driver's
LUW conversion is selected from an identified `SQL...` product release, before
SECCHK; it is not selected by retrying a failed password. Standard IBM-500 stays
separate for z/OS and unidentified servers, and UTF-8 remains selected when
`UNICODEMGR=1208` is negotiated. Explicit 037 and UTF-8 overrides are preserved.

Run the local regressions after building the Node addon and installing `tsx`:

```sh
npm ci --prefix crates/db2-napi
npm run build:debug --prefix crates/db2-napi
npm ci --prefix tests/node
python3 tools/test-credential-encoding.py --ca-cert docker/tls/ca.pem
```

The runner targets existing `db2-wire-test:50000:50001` and
`db2-wire-test-115:50002` containers by default. `--server container:TCP[:TLS]`
can be repeated. It creates uniquely named disposable OS users, sets only their
passwords, grants CONNECT, verifies CLP authentication, and removes users and
grants in `finally`, including after test failures. It needs the server's public
CA for TLS; it never changes shared server settings or existing accounts.

Coverage includes each reported punctuation character, their combination, an
alphanumeric control, and all printable ASCII punctuation in one password;
TCP SECMEC 3, explicit 500, verified TLS SECMEC 3, and TLS requests for SECMEC 7
and the default SECMEC 9. The latter negotiate SECMEC 3 on these servers, within
verified TLS. Wire assertions check the negotiated manager and exact SECCHK
bytes, and verify that AES/DES SECMEC 7/9 requests over TCP stop before SECCHK
when offered plaintext. Negative tests retain certificate/hostname validation
and reject unnegotiated UTF-8 and explicit 037 for the caret fixture.

Protocol tests cover standard and LUW 500, all ASCII and Latin-1 mappings,
user-only SECCHK, Unicode negotiation, explicit overrides, and SECMEC 7/9 DES
and AES ciphertext against independently computed Node/OpenSSL vectors. The
existing z/OS SECMEC 7 AES source-CCSID behavior is retained.

Live encrypted authentication and z/OS validation require separately configured
servers. Both shared Linux servers use `AUTHENTICATION=SERVER`, and this runner
does not reconfigure them. Non-ASCII OS passwords are outside the live matrix.
