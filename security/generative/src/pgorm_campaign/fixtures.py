"""Owned PostgreSQL containers and deterministic subject/reference baselines."""

import asyncio
import contextlib
import json
import os
import secrets
import time
from dataclasses import dataclass, field
from pathlib import Path

from . import baseline, process

IMAGE = "postgres:19.0-trixie@sha256:766ddae4967301b9802b341bb596905f11dd5c558149690a7a4d80c2f61bbcf9"
LABEL = "pgorm.generative=disposable"
SETTINGS = """
SELECT json_build_object(
  'version', version(),
  'encoding', current_setting('server_encoding'),
  'collation', (SELECT datcollate FROM pg_database WHERE datname = current_database()),
  'ctype', (SELECT datctype FROM pg_database WHERE datname = current_database()),
  'timezone', current_setting('TimeZone'),
  'search_path', current_setting('search_path'),
  'standard_conforming_strings', current_setting('standard_conforming_strings'),
  'statement_timeout', current_setting('statement_timeout'),
  'lock_timeout', current_setting('lock_timeout'),
  'idle_in_transaction_session_timeout', current_setting('idle_in_transaction_session_timeout'),
  'superuser', (SELECT rolsuper FROM pg_roles WHERE rolname = current_user),
  'create_database', (SELECT rolcreatedb FROM pg_roles WHERE rolname = current_user),
  'create_role', (SELECT rolcreaterole FROM pg_roles WHERE rolname = current_user),
  'bypass_rls', (SELECT rolbypassrls FROM pg_roles WHERE rolname = current_user)
);
"""

# The server-wide deadlines set at startup are the programs' budget: a
# generated statement that runs away is cut off at two seconds. Provisioning
# and resets are the controller's own work, bounded instead by the
# `docker exec` deadline in `_sql`, so an administrative session takes its own
# limits for its length. Inheriting the programs' two seconds let one TRUNCATE
# on a loaded host deactivate every worker's fixture and turn the rest of a
# run into incompletes, and let one CREATE DATABASE abort a run before it
# began — statements about the host rather than about pgorm. The campaign
# role's session is left alone, so the settings a run records are the ones
# its programs ran under.
ADMIN_DEADLINES = "SET statement_timeout = '25s';\nSET lock_timeout = '10s';\n"


class FixtureFailure(RuntimeError):
    pass


class CleanupFailure(FixtureFailure):
    pass


@dataclass(frozen=True)
class Pair:
    worker: int
    subject: str = field(repr=False)
    reference: str = field(repr=False)
    databases: tuple[str, str]


# [spec:pgorm:req:generative.fixtures]
# [spec:pgorm:req:generative.isolation]
class Fixture:
    """The constructor accepts resource limits, never an external database URL."""

    def __init__(self, artifacts, *, workers=1, run=process.run):
        if type(workers) is not int or not 1 <= workers <= 8:
            raise ValueError("fixture worker count must be between 1 and 8")
        self.artifacts = Path(artifacts)
        self.workers = workers
        self.name = "pgorm-generative-" + secrets.token_hex(8)
        self._run = run
        self._attempted = False
        self._active = False
        self._pairs = []
        self._locks = [asyncio.Lock() for _ in range(workers)]
        self.report = {
            "passed": False,
            "state": "new",
            "image": IMAGE,
            "cleanup_errors": [],
        }

    def _record(self):
        self.artifacts.mkdir(parents=True, exist_ok=True)
        (self.artifacts / "fixture.json").write_text(
            json.dumps(self.report, indent=2) + "\n"
        )

    async def _docker(self, *args, **kwargs):
        return await self._run("docker", *args, **kwargs)

    async def _sql(self, database, sql, *, role="postgres"):
        if role == "postgres":
            sql = ADMIN_DEADLINES + sql
        result = await self._docker(
            "exec",
            "-i",
            self.name,
            "psql",
            "-X",
            "-q",
            "-At",
            "-v",
            "ON_ERROR_STOP=1",
            "-U",
            role,
            "-d",
            database,
            input=sql,
            timeout=30,
        )
        return result.stdout.strip()

    async def _ready(self):
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            result = await self._docker(
                "exec",
                self.name,
                "pg_isready",
                "-h",
                "127.0.0.1",
                "-U",
                "postgres",
                check=False,
                timeout=5,
            )
            if result.returncode == 0:
                return
            await asyncio.sleep(0.1)
        raise FixtureFailure("owned PostgreSQL fixture did not become ready")

    async def _provision(self, port):
        password = secrets.token_hex(24)
        await self._sql(
            "postgres",
            f"""
CREATE ROLE campaign LOGIN PASSWORD '{password}'
 NOSUPERUSER NOCREATEDB NOCREATEROLE NOREPLICATION NOBYPASSRLS NOINHERIT;
ALTER ROLE campaign SET search_path = fixture, pg_catalog;
ALTER ROLE campaign SET standard_conforming_strings = on;
ALTER ROLE campaign SET timezone = 'UTC';
ALTER ROLE campaign SET statement_timeout = '2s';
ALTER ROLE campaign SET lock_timeout = '500ms';
ALTER ROLE campaign SET idle_in_transaction_session_timeout = '5s';
REVOKE CONNECT ON DATABASE postgres FROM PUBLIC;
REVOKE CONNECT ON DATABASE template1 FROM PUBLIC;
""",
        )
        for worker in range(self.workers):
            databases = tuple(
                f"worker_{worker}_{side}" for side in ("subject", "reference")
            )
            urls = []
            for name in databases:
                await self._sql(
                    "postgres", f"CREATE DATABASE {name} TEMPLATE template0;"
                )
                await self._sql(
                    name,
                    f"""
REVOKE ALL ON DATABASE {name} FROM PUBLIC;
GRANT CONNECT, CREATE ON DATABASE {name} TO campaign;
REVOKE ALL ON SCHEMA public FROM PUBLIC;
""",
                )
                urls.append(
                    f"postgresql://campaign:{password}@127.0.0.1:{port}/{name}?sslmode=disable"
                )
            self._pairs.append(Pair(worker, *urls, databases))
        self.report["settings"] = json.loads(
            await self._sql(self._pairs[0].databases[0], SETTINGS, role="campaign")
        )
        if any(
            self.report["settings"][name]
            for name in ("superuser", "create_database", "create_role", "bypass_rls")
        ):
            raise FixtureFailure("campaign role has forbidden privileges")

    async def start(self):
        if self._attempted:
            raise FixtureFailure("fixture instances cannot be restarted")
        self.report.update(state="starting", workers=self.workers, container=self.name)
        self._record()
        self._attempted = True
        environment = {**os.environ, "POSTGRES_PASSWORD": secrets.token_hex(24)}
        try:
            await self._docker(
                "run",
                "--detach",
                "--name",
                self.name,
                "--label",
                LABEL,
                "--publish",
                "127.0.0.1::5432",
                "--network",
                "bridge",
                "--memory",
                "512m",
                "--cpus",
                "2",
                "--pids-limit",
                "256",
                "--read-only",
                "--security-opt",
                "no-new-privileges",
                # The 19 image keeps its data under /var/lib/postgresql/19/docker
                # and refuses to start with anything mounted at .../data.
                "--mount",
                "type=volume,destination=/var/lib/postgresql",
                "--tmpfs",
                "/var/run/postgresql",
                "--tmpfs",
                "/tmp",
                "--env",
                "POSTGRES_PASSWORD",
                "--env",
                "POSTGRES_INITDB_ARGS=-E UTF8 --locale=C",
                IMAGE,
                "-c",
                "max_connections=48",
                "-c",
                "statement_timeout=2000",
                "-c",
                "lock_timeout=500",
                "-c",
                "timezone=UTC",
                environment=environment,
                timeout=120,
            )
            await self._ready()
            mapping = (await self._docker("port", self.name, "5432/tcp")).stdout.strip()
            if not mapping.startswith("127.0.0.1:"):
                raise FixtureFailure("fixture is not bound exclusively to loopback")
            port = int(mapping.removeprefix("127.0.0.1:"))
            if not 1 <= port <= 65535:
                raise FixtureFailure("invalid fixture port")
            await self._provision(port)
            self._active = True
            definition = baseline.default()
            for worker in range(self.workers):
                await self.reset(worker, definition, rebuild=True)
            self.report.update(
                state="ready",
                setup_complete=True,
                baseline_sha256=baseline.digest(definition),
            )
            (self.artifacts / "baseline.json").write_text(
                json.dumps(definition, ensure_ascii=False, indent=2) + "\n"
            )
            self.report["image_id"] = (
                await self._docker("inspect", self.name, "--format", "{{.Image}}")
            ).stdout.strip()
            self._record()
            return self
        except BaseException:
            await asyncio.shield(self.close())
            raise

    def pair(self, worker=0):
        if (
            not self._active
            or type(worker) is not int
            or not 0 <= worker < self.workers
        ):
            raise FixtureFailure("requested worker has no active owned fixture")
        return self._pairs[worker]

    async def reset(self, worker, definition, *, rebuild=False):
        """Reset both databases; callers close pooled clients before a DDL rebuild."""
        pair = self.pair(worker)
        sql = baseline.render(definition) if rebuild else baseline.restore(definition)
        async with self._locks[worker]:
            # A failed side never leaves a usable pair for the next program.
            try:
                for database in pair.databases:
                    await self._sql(database, sql)
            except BaseException:
                self._active = False
                self.report.update(state="reset-failed", reset_failed=True)
                self._record()
                raise

    async def close(self):
        self._active = False
        if not self._attempted or self.report["state"] == "closed":
            return
        errors = []
        # Failure to read diagnostics does not excuse leaving the container alive.
        with contextlib.suppress(Exception):
            result = await self._docker(
                "logs", self.name, check=False, output_limit=8 * 2**20
            )
            (self.artifacts / "postgres.log").write_text(result.stdout + result.stderr)
        try:
            result = await self._docker("inspect", self.name, check=False)
            if result.returncode:
                if "No such object" not in result.stderr:
                    raise CleanupFailure(
                        "could not establish whether the fixture still exists"
                    )
            else:
                info = json.loads(result.stdout)[0]
                if info["Config"]["Labels"].get("pgorm.generative") != "disposable":
                    raise CleanupFailure(
                        "refusing to remove a container without the ownership label"
                    )
                await self._docker("rm", "--force", "--volumes", self.name)
        except Exception as error:
            errors.append(type(error).__name__ + ": " + str(error))
        self._pairs.clear()
        passed = (
            bool(self.report.get("setup_complete"))
            and not self.report.get("reset_failed")
            and not errors
        )
        self.report.update(
            state="cleanup-failed" if errors else "closed",
            cleanup_errors=errors,
            passed=passed,
        )
        self._record()
        if errors:
            raise CleanupFailure("owned fixture cleanup failed; see fixture.json")

    async def __aenter__(self):
        return await self.start()

    async def __aexit__(self, *_):
        await asyncio.shield(self.close())
