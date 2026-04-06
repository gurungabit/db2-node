#!/usr/bin/env bash
# Generate self-signed CA and server certificates for DB2 TLS testing.
# Outputs: ca.pem, ca-key.pem, server.pem, server-key.pem, server.p12
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
cd "$DIR"

# Clean old certs
rm -f ca.pem ca-key.pem server.pem server-key.pem server.p12

echo "==> Generating CA key and certificate..."
openssl req -new -x509 -days 3650 -nodes \
    -keyout ca-key.pem \
    -out ca.pem \
    -subj "/CN=DB2 Test CA/O=db2-wire-test" \
    2>/dev/null

echo "==> Generating server key and CSR..."
openssl req -new -nodes \
    -keyout server-key.pem \
    -out server.csr \
    -subj "/CN=localhost/O=db2-wire-test" \
    2>/dev/null

# SAN extension for localhost + 127.0.0.1
cat > server-ext.cnf <<EOF
[v3_ext]
subjectAltName = DNS:localhost, IP:127.0.0.1
basicConstraints = CA:FALSE
keyUsage = digitalSignature, keyEncipherment
extendedKeyUsage = serverAuth
EOF

echo "==> Signing server certificate with CA..."
openssl x509 -req -days 3650 \
    -in server.csr \
    -CA ca.pem -CAkey ca-key.pem -CAcreateserial \
    -out server.pem \
    -extfile server-ext.cnf -extensions v3_ext \
    2>/dev/null

echo "==> Creating PKCS12 bundle for DB2 GSKit import..."
openssl pkcs12 -export \
    -in server.pem -inkey server-key.pem -CAfile ca.pem -chain \
    -out server.p12 \
    -passout pass:db2test \
    -name "db2server" \
    2>/dev/null

# Cleanup temporaries
rm -f server.csr server-ext.cnf ca.srl

echo "==> Certificates generated:"
ls -la ca.pem server.pem server.p12
echo "Done."
