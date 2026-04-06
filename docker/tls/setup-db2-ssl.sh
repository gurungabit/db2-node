#!/usr/bin/env bash
# Configure SSL/TLS in a running DB2 Community container.
# Prerequisites: container "db2-wire-test" is running and healthy,
#                certs are generated in this directory.
set -euo pipefail

CONTAINER="db2-wire-test"
DIR="$(cd "$(dirname "$0")" && pwd)"
SSL_PORT="${DB2_TEST_SSL_PORT:-50001}"

echo "==> Checking for certificates..."
for f in ca.pem server.p12; do
    if [ ! -f "$DIR/$f" ]; then
        echo "Missing $DIR/$f — run generate-certs.sh first"
        exit 1
    fi
done

echo "==> Copying certificates into container..."
docker cp "$DIR/ca.pem" "$CONTAINER:/tmp/ca.pem"
docker cp "$DIR/server.p12" "$CONTAINER:/tmp/server.p12"
docker exec "$CONTAINER" bash -c "chmod 644 /tmp/ca.pem /tmp/server.p12"

echo "==> Creating GSKit keystore and configuring DB2 SSL..."
docker exec "$CONTAINER" bash -c '
set -euo pipefail

SSL_PORT='"$SSL_PORT"'

# Discover db2inst1 home directory
DB2HOME=$(su - db2inst1 -c "echo \$HOME")
echo "DB2 instance home: $DB2HOME"

KS_DIR="$DB2HOME/keystore"
KEYDB="$KS_DIR/db2server.kdb"
STASH="$KS_DIR/db2server.sth"
KS_PASS="db2test"

# Create keystore directory
su - db2inst1 -c "mkdir -p $KS_DIR"

# Remove old keystore if present
su - db2inst1 -c "rm -f $KS_DIR/db2server.*"

# Create a CMS keystore
su - db2inst1 -c "gsk8capicmd_64 -keydb -create \
    -db $KEYDB -pw $KS_PASS -type cms -stash"

# Import PKCS12 (server cert + key + CA chain) into the CMS keystore
su - db2inst1 -c "gsk8capicmd_64 -cert -import \
    -db /tmp/server.p12 -pw $KS_PASS -type pkcs12 \
    -target $KEYDB -target_pw $KS_PASS -target_type cms" || {
    # Fallback: add CA cert separately then import from PKCS12
    echo "Direct import failed, trying with explicit CA..."
    su - db2inst1 -c "gsk8capicmd_64 -cert -add \
        -db $KEYDB -pw $KS_PASS \
        -file /tmp/ca.pem -label \"DB2 Test CA\" -trust enable"
    su - db2inst1 -c "gsk8capicmd_64 -cert -import \
        -db /tmp/server.p12 -pw $KS_PASS -type pkcs12 \
        -target $KEYDB -target_pw $KS_PASS -target_type cms"
}

# Find the certificate label
CERT_LIST=$(su - db2inst1 -c "gsk8capicmd_64 -cert -list -db $KEYDB -pw $KS_PASS")
echo "Certificates in keystore:"
echo "$CERT_LIST"

# Use the first personal certificate (not a CA/signer cert)
# GSKit list format: "- label" for personal certs, "! label" for trusted
LABEL=$(su - db2inst1 -c "gsk8capicmd_64 -cert -list personal -db $KEYDB -pw $KS_PASS" \
    | grep "^-" | head -1 | sed "s/^-[[:space:]]*//" | sed "s/[[:space:]]*$//")

if [ -z "$LABEL" ]; then
    # Fallback: extract from full list — personal certs start with "-\t"
    LABEL=$(echo "$CERT_LIST" | grep "^-" | head -1 | sed "s/^-[[:space:]]*//" | sed "s/[[:space:]]*$//")
fi

echo "Using certificate label: [$LABEL]"

# Set default cert
su - db2inst1 -c "gsk8capicmd_64 -cert -setdefault \
    -db $KEYDB -pw $KS_PASS -label \"$LABEL\"" 2>/dev/null || true

# Configure DB2 for SSL
su - db2inst1 -c "db2 update dbm cfg using SSL_SVR_KEYDB $KEYDB"
su - db2inst1 -c "db2 update dbm cfg using SSL_SVR_STASH $STASH"
su - db2inst1 -c "db2 update dbm cfg using SSL_SVR_LABEL \"$LABEL\""
su - db2inst1 -c "db2 update dbm cfg using SSL_SVCENAME $SSL_PORT"

# Set the comm protocols to include SSL
CURRENT_PROTO=$(su - db2inst1 -c "db2set DB2COMM" 2>/dev/null | tr -d " " || echo "")
echo "Current DB2COMM: [$CURRENT_PROTO]"
if [[ "$CURRENT_PROTO" != *"SSL"* ]]; then
    if [ -z "$CURRENT_PROTO" ]; then
        su - db2inst1 -c "db2set DB2COMM=TCPIP,SSL"
    else
        su - db2inst1 -c "db2set DB2COMM=${CURRENT_PROTO},SSL"
    fi
fi

echo "==> Restarting DB2 to apply SSL configuration..."
su - db2inst1 -c "db2stop force" || true
sleep 2
su - db2inst1 -c "db2start"
sleep 5

# Reconnect to database to verify
su - db2inst1 -c "db2 connect to testdb" || {
    echo "First connect attempt failed, retrying..."
    sleep 5
    su - db2inst1 -c "db2 connect to testdb"
}

echo "==> DB2 SSL setup complete. SSL on port $SSL_PORT"
'

echo "==> Verifying SSL port is listening..."
sleep 3
if docker exec "$CONTAINER" bash -c "netstat -tlnp 2>/dev/null | grep -q :${SSL_PORT}"; then
    echo "SSL port $SSL_PORT is open."
else
    echo "WARNING: SSL port $SSL_PORT not yet listening. DB2 may need more time."
    sleep 5
    docker exec "$CONTAINER" bash -c "netstat -tlnp 2>/dev/null || echo 'netstat not available'"
fi

echo "Done. Test with: DB2_TEST_SSL_PORT=$SSL_PORT cargo test --test tls_test"
