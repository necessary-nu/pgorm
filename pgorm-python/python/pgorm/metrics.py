"""pgorm's metrics layer: a pool that reports each statement, acquisition and
transaction to a collector the program writes.

An ``InstrumentedPool`` is pgorm's ``InstrumentedPool`` over the pool it
wraps, so the timing, and which hook hears what, are pgorm's own. Each hook
runs on the pool's event loop, and the statement it reports on finishes once
the hook has: a plain hook once it returns, an async one once it completes.
"""

import asyncio
from collections.abc import Awaitable, Callable
from functools import partial
import inspect
import logging
from typing import Protocol, Self

from . import _native
from ._native import PgormError, QueryContext, QueryFingerprint
from .runtime import Pool

__all__ = [
    "InstrumentedPool",
    "LoggingMetrics",
    "MetricsCollector",
    "NoOpMetrics",
    "QueryContext",
    "QueryFingerprint",
]


# [spec:pgorm:req:bindings.metrics]
class MetricsCollector(Protocol):
    """What an ``InstrumentedPool`` reports to: Rust's seven hooks, each a
    plain method or an async one.

    ``duration`` is in seconds. ``record_query_error`` and
    ``record_connection_error`` are given the exception the caller is raised,
    not raised. ``record_transaction_rollback`` hears every transaction or
    savepoint that ended without committing, a failed ``COMMIT`` included.
    """

    def record_query_success(
        self, query: QueryContext, duration: float, rows: int | None
    ) -> Awaitable[None] | None: ...

    def record_query_error(
        self, query: QueryContext, duration: float, error: PgormError
    ) -> Awaitable[None] | None: ...

    def record_connection_acquired(self, duration: float) -> Awaitable[None] | None: ...

    def record_connection_error(
        self, duration: float, error: PgormError
    ) -> Awaitable[None] | None: ...

    def record_transaction_begin(self, duration: float) -> Awaitable[None] | None: ...

    def record_transaction_commit(self, duration: float) -> Awaitable[None] | None: ...

    def record_transaction_rollback(self, duration: float) -> Awaitable[None] | None: ...


class NoOpMetrics:
    """A collector whose every hook does nothing.

    A collector that cares about some hooks only subclasses it and overrides
    those. A hook left as this class wrote it is never run, so it costs the
    statement nothing beyond pgorm's own timing.
    """

    def record_query_success(
        self, query: QueryContext, duration: float, rows: int | None
    ) -> Awaitable[None] | None:
        return None

    def record_query_error(
        self, query: QueryContext, duration: float, error: PgormError
    ) -> Awaitable[None] | None:
        return None

    def record_connection_acquired(self, duration: float) -> Awaitable[None] | None:
        return None

    def record_connection_error(self, duration: float, error: PgormError) -> Awaitable[None] | None:
        return None

    def record_transaction_begin(self, duration: float) -> Awaitable[None] | None:
        return None

    def record_transaction_commit(self, duration: float) -> Awaitable[None] | None:
        return None

    def record_transaction_rollback(self, duration: float) -> Awaitable[None] | None:
        return None


def _elapsed(duration: float) -> str:
    return f"{duration * 1000:.3f}ms"


def _identity(query: QueryContext) -> str:
    fingerprint = query.fingerprint
    return "" if fingerprint is None else f" [{fingerprint}]"


class LoggingMetrics:
    """A collector writing each report to ``logger``, ``pgorm.metrics`` unless
    another is given, at the levels Rust's ``LoggingMetrics`` writes its
    ``tracing`` events at: ``DEBUG`` for a successful query, a connection, a
    transaction's begin and its commit; ``WARNING`` for a failed query and a
    rollback; ``ERROR`` for a failure to get a connection. A query's message
    ends with its fingerprint when it has one.
    """

    def __init__(self, logger: logging.Logger | None = None) -> None:
        self.logger = logger if logger is not None else logging.getLogger("pgorm.metrics")

    def record_query_success(self, query: QueryContext, duration: float, rows: int | None) -> None:
        if not self.logger.isEnabledFor(logging.DEBUG):
            return
        counted = "" if rows is None else f", {rows} rows"
        self.logger.debug(
            "Query %s succeeded in %s%s%s",
            query.operation, _elapsed(duration), counted, _identity(query),
        )

    def record_query_error(self, query: QueryContext, duration: float, error: PgormError) -> None:
        if self.logger.isEnabledFor(logging.WARNING):
            self.logger.warning(
                "Query %s failed after %s: %s%s",
                query.operation, _elapsed(duration), error, _identity(query),
            )

    def record_connection_acquired(self, duration: float) -> None:
        self.logger.debug("Connection acquired in %s", _elapsed(duration))

    def record_connection_error(self, duration: float, error: PgormError) -> None:
        self.logger.error("Connection failed after %s: %s", _elapsed(duration), error)

    def record_transaction_begin(self, duration: float) -> None:
        self.logger.debug("Transaction began in %s", _elapsed(duration))

    def record_transaction_commit(self, duration: float) -> None:
        self.logger.debug("Transaction committed in %s", _elapsed(duration))

    def record_transaction_rollback(self, duration: float) -> None:
        self.logger.warning("Transaction rolled back in %s", _elapsed(duration))


type _Hook = Callable[..., object]

# The tasks async hooks run in, held until they finish: the event loop keeps
# only a weak reference to a task.
_running: set[asyncio.Future[object]] = set()


def _failed(name: str, error: BaseException) -> None:
    """Hand a hook's failure to the event loop's exception handler, which
    reports an exception raised where nothing awaits it."""
    asyncio.get_running_loop().call_exception_handler({
        "message": f"pgorm's metrics hook {name} raised",
        "exception": error,
    })


def _settled(name: str, answer: Callable[[], None], task: asyncio.Future[object]) -> None:
    _running.discard(task)
    try:
        if not task.cancelled() and (error := task.exception()) is not None:
            _failed(name, error)
    finally:
        answer()


# [spec:pgorm:req:bindings.metrics]
def _dispatch(name: str, hook: _Hook, args: tuple[object, ...], answer: Callable[[], None]) -> None:
    """Run one hook on the pool's event loop and ``answer`` once it has
    finished, which lets the statement it reports on go on. Its failure is
    reported and never reaches that statement."""
    try:
        result = hook(*args)
        if inspect.isawaitable(result):
            task = asyncio.ensure_future(result)
            _running.add(task)
            task.add_done_callback(partial(_settled, name, answer))
            return
    except Exception as error:
        _failed(name, error)
    except BaseException:
        answer()
        raise
    answer()


def _hooks(metrics: object) -> list[_Hook | None]:
    """The collector's seven hooks in pgorm's order, ``None`` for each it left
    as ``NoOpMetrics`` wrote it."""
    hooks: list[_Hook | None] = []
    for name in _native.METRICS_HOOKS:
        hook = getattr(metrics, name, None)
        if not callable(hook):
            raise TypeError(f"a metrics collector has a {name} method")
        unchanged = getattr(hook, "__func__", None) is getattr(NoOpMetrics, name)
        hooks.append(None if unchanged else hook)
    return hooks


# [spec:pgorm:req:bindings.metrics]
class InstrumentedPool[M: MetricsCollector](Pool):
    """``pool``, reporting to ``metrics`` what runs through it.

    It is the same pool: its connections, its size and its closing are the
    wrapped pool's, and closing either closes both. A statement, a registered
    entity's or a model's query, a pipeline, a stream, a COPY's start and a
    notification run on it, or on a connection it lends, or in a transaction
    or savepoint on one, are reported, and so is each acquisition and each
    transaction's begin and end. ``inner`` runs the same work unreported, and
    a listener is not reported at all.
    """

    def __init__(self, pool: Pool, metrics: M) -> None:
        if not isinstance(pool, Pool):
            raise TypeError("an InstrumentedPool wraps a Pool")
        self._native = pool._native.instrumented(_hooks(metrics), _dispatch)
        self._inner = pool
        self._metrics = metrics

    @property
    def inner(self) -> Pool:
        """The pool this one wraps, whose work is not reported."""
        return self._inner

    @property
    def metrics(self) -> M:
        """The collector this pool reports to."""
        return self._metrics

    async def __aenter__(self) -> Self:
        await super().__aenter__()
        return self

    def __repr__(self) -> str:
        return f"InstrumentedPool(closed={self.closed})"
