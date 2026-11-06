"""What a pool's options are read as before the native side takes them: TLS
and its PEM files, the shape one pool takes of its own, and its hooks with the
connection each is given."""

import inspect
from collections.abc import Awaitable, Callable
from pathlib import Path
from typing import Any, Literal, TypedDict

from . import _native
from .results import Query, Record

TlsMode = Literal["disable", "prefer", "require", "verify-ca", "verify-full"]
"""How a pool uses TLS, named as libpq's ``sslmode`` names it."""

QueueMode = Literal["fifo", "lifo"]
"""Whether a pool lends the idle connection returned first or last."""

Recycle = Literal["verified", "fast", "clean"]
"""How a pool checks a returned connection before it lends it again."""


# [spec:pgorm:req:bindings.pool-hooks]
class HookConnection:
    """The connection a pool hook is about, while the hook runs.

    Its statements run on that connection, through pgorm's statement cache, as
    a ``Connection``'s do. Once the hook returns the connection is the pool's,
    and every statement raises ``LifecycleError``.
    """

    def __init__(self, native: "_native._HookConnection") -> None:
        if not isinstance(native, _native._HookConnection):
            raise TypeError("a HookConnection is given to a pool hook")
        self._native = native

    @property
    def age(self) -> float:
        """Seconds since the connection opened."""
        return self._native.age

    @property
    def recycle_count(self) -> int:
        """How many times the pool has recycled the connection."""
        return self._native.recycle_count

    @property
    def closed(self) -> bool:
        """Whether the hook has returned."""
        return self._native.closed()

    async def execute(self, query: Query) -> int:
        return await self._native.execute(query)

    async def query(self, query: Query) -> list[Record]:
        return await self._native.fetch(query, "all")

    async def one(self, query: Query) -> Record:
        return await self._native.fetch(query, "one")

    async def optional(self, query: Query) -> Record | None:
        return await self._native.fetch(query, "optional")

    def __repr__(self) -> str:
        return f"HookConnection(closed={self.closed})"


PoolHook = Callable[[HookConnection], Awaitable[None] | None]
"""A pool hook: called with the connection, it may be ``async``; raising
refuses the connection."""


def _hook(hook: PoolHook | None, name: str) -> Callable[[Any], Awaitable[None]] | None:
    """``hook`` as the native side calls it: a coroutine function run on the
    pool's loop, awaiting what the hook returns."""
    if hook is None:
        return None
    if not callable(hook):
        raise TypeError(f"{name} is a callable taking a HookConnection")

    async def run(native: "_native._HookConnection") -> None:
        outcome = hook(HookConnection(native))
        if inspect.isawaitable(outcome):
            await outcome

    return run


def _pem(value: object, path: object, name: str, what: str, read: str) -> bytes | None:
    """PEM given as data, or read from the file ``path`` names, never both."""
    if value is not None and not isinstance(value, (str, bytes)):
        raise TypeError(f"{name} is PEM data, as str or bytes")
    if value is not None and path is not None:
        raise _native.ConstructionError(f"pass {what} as {name} or as {name}file, not both")
    if isinstance(value, str):
        return value.encode()
    if isinstance(value, bytes):
        return value
    if path is None:
        return None
    if not isinstance(path, (str, Path)):
        raise TypeError(f"{name}file is a path, as str or Path")
    try:
        return Path(path).read_bytes()
    except OSError:
        raise _native.ConstructionError(f"could not read the {read} file") from None


def _seconds(name: str, value: object) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} is a number of seconds")
    return float(value)


def _optional_seconds(name: str, value: object) -> float | None:
    if value is None:
        return None
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise TypeError(f"{name} is a number of seconds or None")
    return float(value)


class PoolShape(TypedDict, total=False):
    """What one pool of a ``MultiPool`` takes of its own: the options Rust's
    ``PoolBuilder`` closure sets, and how long acquiring waits."""

    max_size: int
    acquire_timeout: float
    wait_timeout: float | None
    create_timeout: float | None
    recycle_timeout: float | None
    queue_mode: QueueMode
    post_create: PoolHook | None
    pre_recycle: PoolHook | None
    post_recycle: PoolHook | None


def _shape(
    max_size: int = 10,
    acquire_timeout: float = 30.0,
    wait_timeout: float | None = None,
    create_timeout: float | None = None,
    recycle_timeout: float | None = None,
    queue_mode: QueueMode = "fifo",
    post_create: PoolHook | None = None,
    pre_recycle: PoolHook | None = None,
    post_recycle: PoolHook | None = None,
) -> "_native._PoolShape":
    if type(max_size) is not int:
        raise TypeError("max_size is an int")
    if max_size < 0:
        raise _native.ConstructionError("max_size must be a nonnegative integer")
    if queue_mode not in ("fifo", "lifo"):
        raise TypeError("queue_mode is 'fifo' or 'lifo'")
    try:
        return _native._PoolShape(
            max_size=max_size,
            acquire_timeout=_seconds("acquire_timeout", acquire_timeout),
            wait_timeout=_optional_seconds("wait_timeout", wait_timeout),
            create_timeout=_optional_seconds("create_timeout", create_timeout),
            recycle_timeout=_optional_seconds("recycle_timeout", recycle_timeout),
            queue_mode=queue_mode,
            post_create=_hook(post_create, "post_create"),
            pre_recycle=_hook(pre_recycle, "pre_recycle"),
            post_recycle=_hook(post_recycle, "post_recycle"),
        )
    except OverflowError as error:
        raise _native.ConstructionError(str(error)) from None


def _tls(
    tls: object, ca: str | bytes | None, cafile: str | Path | None, cert: str | bytes | None,
    certfile: str | Path | None, key: str | bytes | None, keyfile: str | Path | None,
) -> dict[str, Any]:
    """The TLS options as the native side takes them, files read."""
    if tls not in (None, "disable", "prefer", "require", "verify-ca", "verify-full"):
        raise TypeError("tls is 'disable', 'prefer', 'require', 'verify-ca', 'verify-full' or None")
    return {
        "tls": tls,
        "ca_pem": _pem(ca, cafile, "ca", "the CA", "CA certificate"),
        "cert_pem": _pem(cert, certfile, "cert", "the client certificate", "client certificate"),
        "key_pem": _pem(key, keyfile, "key", "the client key", "client key"),
    }


def _shared(
    dsn: object, recycle: object, recycle_sql: object, statement_cache_size: object,
    statement_cache_channel: object, connect_timeout: object,
) -> dict[str, Any]:
    """The options every pool built from one connection string shares."""
    if not isinstance(dsn, str):
        raise TypeError("dsn is a connection string")
    if recycle not in (None, "verified", "fast", "clean"):
        raise TypeError("recycle is 'verified', 'fast', 'clean' or None")
    if recycle_sql is not None:
        if not isinstance(recycle_sql, str):
            raise TypeError("recycle_sql is a str or None")
        if recycle is not None:
            raise TypeError("recycle and recycle_sql both say how to recycle: give one")
    if type(statement_cache_size) is not int:
        raise TypeError("statement_cache_size is an int")
    if statement_cache_size < 0:
        raise _native.ConstructionError("statement_cache_size must be a nonnegative integer")
    if statement_cache_channel is not None and not isinstance(statement_cache_channel, str):
        raise TypeError("statement_cache_channel is a str or None")
    return {
        "recycle": recycle or "verified",
        "recycle_sql": recycle_sql,
        "statement_cache_size": statement_cache_size,
        "statement_cache_channel": statement_cache_channel,
        "connect_timeout": _seconds("connect_timeout", connect_timeout),
    }
