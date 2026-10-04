#!/usr/bin/env python3
"""Run issue #25 regressions with disposable users on existing Db2 containers.

Requires the built Node addon and `npm ci --prefix tests/node`. Defaults to the
shared Db2 12.1 (including verified TLS) and 11.5 test servers. It never changes
server configuration or existing users. Only the public TLS CA is needed.
"""

import argparse
import json
import os
from pathlib import Path
import secrets
import signal
import string
import subprocess


ROOT = Path(__file__).resolve().parents[1]


def docker(container, *args, input=None):
    result = subprocess.run(
        ["docker", "exec", "-i", container, *args], input=input,
        text=True, capture_output=True,
    )
    if result.returncode:
        # Never include a command containing credentials in the exception.
        raise RuntimeError(f"{container}: {result.stdout}{result.stderr}")
    return result.stdout


def clp(container, sql):
    return docker(container, "su", "-", "db2inst1", "-c", "db2 -st", input=sql)


def run_server(container, port, tls_port, ca_cert):
    prefix = "i25" + secrets.token_hex(3)
    passwords = [
        ("alphanumeric", "Abc123xyz"), ("exclamation", "Abc123!x"),
        ("caret", "Abc123^x"), ("left bracket", "Abc123[x"),
        ("right bracket", "Abc123]x"), ("pipe", "Abc123|x"),
        ("reported punctuation", "Abc123!^[]|x"),
        ("all ASCII punctuation", "Abc123" + string.punctuation + "xyz"),
    ]
    fixtures = [
        {"label": label, "user": f"{prefix}{i}", "password": password}
        for i, (label, password) in enumerate(passwords)
    ]
    created = []
    granted = []
    try:
        for fixture in fixtures:
            user = fixture["user"]
            docker(container, "useradd", "-M", user)
            created.append(user)  # Track immediately, even if password/grant fails.
            docker(container, "chpasswd", input=f"{user}:{fixture['password']}\n")
            granted.append(user)
            clp(container, f"CONNECT TO TESTDB;\nGRANT CONNECT ON DATABASE TO USER {user};\nCONNECT RESET;\n")
            # Db2 delimited password syntax escapes embedded double quotes.
            password = fixture["password"].replace('"', '""')
            clp(container, f'CONNECT TO TESTDB USER {user} USING "{password}";\nCONNECT RESET;\n')
        print(f"{container}: CLP authenticated all {len(fixtures)} password fixtures", flush=True)
        env = {
            **os.environ, "DB2_TEST_HOST": "localhost", "DB2_TEST_PORT": str(port),
            "DB2_TEST_DATABASE": "TESTDB", "DB2_TEST_CREDENTIAL_FIXTURES": json.dumps(fixtures),
            "DB2_TEST_SSL_PORT": str(tls_port) if tls_port else "",
            "DB2_TEST_CA_CERT": str(ca_cert),
        }
        return subprocess.run(
            ["node", "--import", "tsx", "--test", "credential-encoding.test.ts"],
            cwd=ROOT / "tests/node", env=env,
        ).returncode
    finally:
        errors = []
        for user in reversed(created):
            if user in granted:
                try:
                    clp(container, f"CONNECT TO TESTDB;\nREVOKE CONNECT ON DATABASE FROM USER {user};\nCONNECT RESET;\n")
                except Exception as error:
                    errors.append(str(error))
            try:
                docker(container, "userdel", user)
            except Exception as error:
                errors.append(str(error))
        if errors:
            raise RuntimeError("Disposable user cleanup failed: " + "\n".join(errors))
        print(f"{container}: removed all {len(created)} disposable users and grants", flush=True)


def interrupted(signum, frame):
    raise KeyboardInterrupt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server", action="append", help="container:TCP-port[:TLS-port] (repeatable)")
    parser.add_argument("--ca-cert", type=Path, default=ROOT / "docker/tls/ca.pem")
    args = parser.parse_args()
    servers = args.server or ["db2-wire-test:50000:50001", "db2-wire-test-115:50002"]
    signal.signal(signal.SIGTERM, interrupted)
    failed = False
    for server in servers:
        container, port, *tls = server.split(":")
        tls_port = int(tls[0]) if tls else None
        if tls_port and not args.ca_cert.is_file():
            parser.error("Verified TLS requires --ca-cert pointing to the server's public CA")
        failed |= run_server(container, int(port), tls_port, args.ca_cert.resolve()) != 0
    return int(failed)


if __name__ == "__main__":
    raise SystemExit(main())
