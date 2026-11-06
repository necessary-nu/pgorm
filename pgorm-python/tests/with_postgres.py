"""Run a command against a disposable PostgreSQL server with verified TLS.

Usage: python with_postgres.py python -m unittest discover -s pgorm-python/tests
Requires Docker and OpenSSL. The public package has neither dependency.
"""

from contextlib import contextmanager
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import uuid

IMAGE = "postgres:18.6-bookworm@sha256:afc7e2d441324c0388fa80c3d24f733b4194a4eb7f47dd8ee2b08eb1a24a647c"

# How long the server may take to accept connections, and then to turn TLS on.
# A ready server ends the wait at once; a heavily loaded machine can take
# minutes to start a container. PGORM_TEST_READY_SECONDS replaces the bound.
READY_SECONDS = float(os.environ.get("PGORM_TEST_READY_SECONDS", "180"))

# The role that authenticates by a client certificate alone, over TLS.
CERT_USER = "pgorm_cert"


def run(*command, input=None):
    return subprocess.run(command, input=input, text=True, check=True, capture_output=True).stdout.strip()


def certificates(root):
    run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-days", "2",
        "-subj", "/CN=pgorm-python-test-ca", "-addext", "basicConstraints=critical,CA:TRUE",
        "-keyout", str(root / "ca.key"), "-out", str(root / "ca.pem"))
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-subj", "/CN=localhost",
        "-keyout", str(root / "server.key"), "-out", str(root / "server.csr"))
    (root / "server.ext").write_text(
        "subjectAltName=DNS:localhost,IP:127.0.0.1\n"
        "basicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature,keyEncipherment\n"
        "extendedKeyUsage=serverAuth\n"
    )
    run("openssl", "x509", "-req", "-in", str(root / "server.csr"),
        "-CA", str(root / "ca.pem"), "-CAkey", str(root / "ca.key"), "-CAcreateserial",
        "-days", "2", "-extfile", str(root / "server.ext"), "-out", str(root / "server.crt"))
    # A client certificate for CERT_USER, whom the server lets in by it alone.
    run("openssl", "req", "-newkey", "rsa:2048", "-nodes", "-subj", f"/CN={CERT_USER}",
        "-keyout", str(root / "client.key"), "-out", str(root / "client.csr"))
    (root / "client.ext").write_text(
        "basicConstraints=critical,CA:FALSE\n"
        "keyUsage=critical,digitalSignature,keyEncipherment\n"
        "extendedKeyUsage=clientAuth\n"
    )
    run("openssl", "x509", "-req", "-in", str(root / "client.csr"),
        "-CA", str(root / "ca.pem"), "-CAkey", str(root / "ca.key"), "-CAcreateserial",
        "-days", "2", "-extfile", str(root / "client.ext"), "-out", str(root / "client.crt"))


def enable_tls(name, root):
    # The image's data directory depends on its release (18 keeps it under
    # /var/lib/postgresql/18/docker), so ask the server where it is.
    data = run("docker", "exec", name, "psql", "-At", "-U", "postgres", "-d", "pgorm_python",
               "-c", "SHOW data_directory")
    for filename in ("server.crt", "server.key", "ca.pem"):
        run("docker", "cp", str(root / filename), f"{name}:{data}/{filename}")
    run("docker", "exec", name, "chown", "postgres:postgres", f"{data}/server.crt", f"{data}/server.key",
        f"{data}/ca.pem")
    run("docker", "exec", name, "chmod", "600", f"{data}/server.key")
    # The first matching line decides, so CERT_USER's comes before the image's
    # password lines: over TLS it is let in by its certificate, which
    # ssl_ca_file has the server ask every TLS client for.
    run("docker", "exec", name, "sed", "-i", f"1i hostssl all {CERT_USER} all cert", f"{data}/pg_hba.conf")
    run("docker", "exec", "-i", name, "psql", "-v", "ON_ERROR_STOP=1", "-U", "postgres", "-d", "pgorm_python",
        input="ALTER SYSTEM SET ssl = 'on';\n"
              "ALTER SYSTEM SET ssl_cert_file = 'server.crt';\n"
              "ALTER SYSTEM SET ssl_key_file = 'server.key';\n"
              "ALTER SYSTEM SET ssl_ca_file = 'ca.pem';\n"
              f"CREATE ROLE {CERT_USER} LOGIN;\n"
              "SELECT pg_reload_conf();\n")
    deadline = time.monotonic() + READY_SECONDS
    while run("docker", "exec", name, "psql", "-At", "-U", "postgres", "-d", "pgorm_python", "-c", "SHOW ssl") != "on":
        if time.monotonic() >= deadline:
            raise RuntimeError("PostgreSQL TLS configuration did not become active")
        time.sleep(0.05)


# [spec:pgorm:req:bindings.connections+3/test]
@contextmanager
def fixture():
    name = f"pgorm-python-{uuid.uuid4().hex[:12]}"
    with tempfile.TemporaryDirectory(prefix="pgorm-python-tls-") as directory:
        root = Path(directory)
        certificates(root)
        run("docker", "run", "--detach", "--rm", "--name", name,
            "--label", "pgorm.python.tests=true", "-e", "POSTGRES_PASSWORD=pgorm-python-test",
            "-e", "POSTGRES_DB=pgorm_python", "-p", "127.0.0.1::5432", IMAGE)
        try:
            deadline = time.monotonic() + READY_SECONDS
            while True:
                ready = subprocess.run(["docker", "exec", name, "pg_isready", "-h", "127.0.0.1",
                                        "-U", "postgres", "-d", "pgorm_python"], capture_output=True)
                if ready.returncode == 0:
                    break
                if time.monotonic() >= deadline:
                    raise RuntimeError("PostgreSQL did not become ready: " + run("docker", "logs", name))
                time.sleep(0.1)
            enable_tls(name, root)
            port = run("docker", "port", name, "5432/tcp").rsplit(":", 1)[1]
            yield {
                **os.environ,
                "PGORM_TEST_DSN": f"postgres://postgres:pgorm-python-test@127.0.0.1:{port}/pgorm_python?sslmode=disable",
                "PGORM_TEST_CA": str(root / "ca.pem"),
                "PGORM_TEST_CLIENT_CERT": str(root / "client.crt"),
                "PGORM_TEST_CLIENT_KEY": str(root / "client.key"),
                "PGORM_TEST_CERT_USER": CERT_USER,
            }
        finally:
            run("docker", "stop", "--time", "2", name)


if __name__ == "__main__":
    if len(sys.argv) < 2:
        raise SystemExit("usage: with_postgres.py COMMAND [ARG ...]")
    with fixture() as environment:
        status = subprocess.run(sys.argv[1:], env=environment).returncode
    raise SystemExit(status)
