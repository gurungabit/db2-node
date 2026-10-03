#!/usr/bin/env bash
set -euo pipefail

# A local CLP connection can succeed before the restarted TCP listener is ready.
# Probe the CI database through a temporary TCP catalog entry instead.
docker exec -i "${DB2_CONTAINER:-db2-wire-test}" su - db2inst1 <<'DB2_READY'
set -euo pipefail
cleanup() {
    db2 connect reset >/dev/null 2>&1 || true
    db2 terminate >/dev/null 2>&1 || true
    db2 uncatalog database CI_READY >/dev/null 2>&1 || true
    db2 uncatalog node CI_TCP >/dev/null 2>&1 || true
}
trap cleanup EXIT
db2 catalog tcpip node CI_TCP remote 127.0.0.1 server 50000
db2 catalog database testdb as CI_READY at node CI_TCP
db2 terminate
for attempt in $(seq 1 24); do
    if db2 connect to CI_READY user db2inst1 using db2wire_test_pw && db2 -x 'VALUES 1'; then
        echo 'Db2 TCP connection and SQL execution are ready.'
        exit 0
    fi
    db2 terminate >/dev/null 2>&1 || true
    sleep 5
done
echo 'Db2 TCP connection did not become ready within 120 seconds.' >&2
exit 1
DB2_READY
