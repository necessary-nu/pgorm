"""Several pools against one server, and a listener of no pool."""

from collections.abc import Mapping
from pathlib import Path
from typing import Any

from . import _native
from .listen import Listener
from .pool_options import PoolShape, Recycle, TlsMode, _seconds, _shape, _shared, _tls
from .runtime import _USIZE_MAX, Pool, PoolStatus

# [spec:pgorm:req:bindings.multi-pool]
class MultiPool:
    """Several pools against one server, one per key of ``pools``, each tagged
    with its key and shaped by its own options, as Rust's
    ``connect_multi_with`` builds them.

    The connection string, TLS and the options after ``pools`` are every
    pool's; an entry of ``pools`` holds a ``PoolShape``'s, those Rust's
    ``PoolBuilder`` closure sets. Building one connects to nothing, as a
    ``Pool``'s does; its ``async with`` block checks a connection of each.
    Choosing between them is the program's: nothing balances or falls back
    across them.
    """

    def __init__(
        self,
        dsn: str,
        pools: Mapping[str, PoolShape],
        *,
        tls: TlsMode | None = None,
        ca: str | bytes | None = None,
        cafile: str | Path | None = None,
        cert: str | bytes | None = None,
        certfile: str | Path | None = None,
        key: str | bytes | None = None,
        keyfile: str | Path | None = None,
        connect_timeout: float = 10.0,
        statement_cache_size: int = 128,
        recycle: Recycle | None = None,
        recycle_sql: str | None = None,
        statement_cache_channel: str | None = None,
    ) -> None:
        shared = _shared(dsn, recycle, recycle_sql, statement_cache_size, statement_cache_channel, connect_timeout)
        if not isinstance(pools, Mapping):
            raise TypeError("pools maps each pool's tag to its options")
        shapes: dict[str, _native._PoolShape] = {}
        for tag, options in pools.items():
            if not isinstance(tag, str):
                raise TypeError("a pool's tag is a str")
            if not isinstance(options, Mapping):
                raise TypeError(f"pool {tag!r}'s options are a mapping")
            unknown = sorted(set(options) - set(PoolShape.__annotations__))
            if unknown:
                raise TypeError(f"pool {tag!r} takes no option {unknown[0]!r}")
            shapes[tag] = _shape(**options)
        try:
            natives = _native._Pool.multi(dsn, shapes, **_tls(tls, ca, cafile, cert, certfile, key, keyfile), **shared)
        except OverflowError as error:
            raise _native.ConstructionError(str(error)) from None
        self._pools = {tag: Pool._adopt(native) for tag, native in natives}

    def get(self, tag: str) -> Pool | None:
        """The pool tagged ``tag``, or ``None``."""
        return self._pools.get(tag)

    def items(self) -> list[tuple[str, Pool]]:
        """Every pool with its tag, in the tags' order."""
        return list(self._pools.items())

    def status(self) -> dict[str, PoolStatus]:
        """Each pool's status, by tag."""
        return {tag: pool.status() for tag, pool in self._pools.items()}

    @property
    def closed(self) -> bool:
        return all(pool.closed for pool in self._pools.values())

    async def close(self) -> None:
        """Close every pool."""
        for pool in self._pools.values():
            await pool.close()

    aclose = close

    async def __aenter__(self) -> "MultiPool":
        try:
            for pool in self._pools.values():
                await pool.ping()
        except BaseException:
            await self.close()
            raise
        return self

    async def __aexit__(self, *exc: Any) -> None:
        await self.close()

    def __repr__(self) -> str:
        return f"MultiPool(tags={list(self._pools)!r}, closed={self.closed})"


# [spec:pgorm:req:bindings.listen+3]
# [spec:pgorm:req:bindings.argument-errors+1]
async def connect_listener(
    dsn: str,
    *,
    capacity: int = 1024,
    tls: TlsMode | None = None,
    ca: str | bytes | None = None,
    cafile: str | Path | None = None,
    cert: str | bytes | None = None,
    certfile: str | Path | None = None,
    key: str | bytes | None = None,
    keyfile: str | Path | None = None,
    connect_timeout: float = 10.0,
) -> Listener:
    """Open a listener on a connection of its own, belonging to no pool, as
    Rust's ``connect_listener`` does.

    The connection string and TLS options are read as a ``Pool``'s are. The
    listener holds at most ``capacity`` notifications unread; see
    ``Listener``. Closing the listener closes its connection, and nothing
    else does.
    """
    if not isinstance(dsn, str):
        raise TypeError("dsn is a connection string")
    if type(capacity) is not int:
        raise TypeError("capacity is an int")
    if not 1 <= capacity <= _USIZE_MAX:
        raise ValueError("capacity must be a positive integer")
    native = await _native._connect_listener(
        dsn, capacity, connect_timeout=_seconds("connect_timeout", connect_timeout),
        **_tls(tls, ca, cafile, cert, certfile, key, keyfile),
    )
    return Listener(native)
