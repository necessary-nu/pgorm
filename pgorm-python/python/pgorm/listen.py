"""A dedicated listening connection and the notifications it receives."""
from __future__ import annotations

import asyncio
from types import TracebackType

from . import _native


# [spec:pgorm:req:bindings.listen+2]
# [spec:pgorm:req:bindings.cancellation]
class Listener:
    """A connection of its own that receives LISTEN notifications.

    Open one with ``await pool.listener()``. It uses the pool's configuration
    and TLS but none of its connections, and closing the pool closes it.
    Iterate it with ``async for`` to receive ``Notification`` objects in the
    order PostgreSQL sent them; ``listen`` and ``unlisten`` may run while a
    pull is pending.

    At most ``capacity`` notifications wait unread. One more closes the
    listener, and iteration raises ``ConnectionError`` after yielding those
    waiting. A failed or terminated connection raises its error the same way.
    Nothing reconnects: a notification sent while nothing listened is lost,
    so a reader that sees the error must re-read what it was tracking.
    Closing the listener ends iteration.
    """

    def __init__(self, native: _native._Listener) -> None:
        if not isinstance(native, _native._Listener):
            raise TypeError("listeners must be opened by a pool")
        self._native = native
        self._ended = False

    @property
    def closed(self) -> bool:
        return self._ended or self._native.closed()

    async def listen(self, channel: str) -> None:
        """Receive the notifications sent on ``channel``, named exactly.

        The name is sent as a quoted identifier, so its case and punctuation
        are kept; one longer than 63 bytes raises ``ConstructionError``, and
        one that is no ``str`` ``TypeError``.
        Cancelling this closes the listener, since whether the server acted
        on it is then unknown.
        """
        await self._native.listen(channel)

    async def unlisten(self, channel: str) -> None:
        """Stop receiving ``channel``'s notifications; see ``listen``."""
        await self._native.unlisten(channel)

    async def unlisten_all(self) -> None:
        """Stop receiving notifications on every channel."""
        await self._native.unlisten_all()

    def __aiter__(self) -> Listener:
        return self

    async def __anext__(self) -> _native.Notification:
        """Wait for the next notification; a cancelled wait loses nothing."""
        if self._ended:
            raise StopAsyncIteration
        try:
            notification = await self._native.next()
        except asyncio.CancelledError:
            raise
        except BaseException:
            if self._native.closed():
                self._ended = True
            raise
        if notification is None:
            self._ended = True
            raise StopAsyncIteration
        return notification

    async def aclose(self) -> None:
        """Close the connection; a pending or later pull ends iteration."""
        await self._native.close()

    close = aclose

    async def __aenter__(self) -> Listener:
        if self.closed:
            raise _native.LifecycleError("listener is closed")
        return self

    async def __aexit__(
        self,
        exc_type: type[BaseException] | None,
        exc: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        await self.aclose()

    # [spec:pgorm:req:python.truth-testing+5]
    def __bool__(self) -> bool:
        raise _native.ConstructionError("listeners cannot be tested as Python booleans")

    def __repr__(self) -> str:
        return f"Listener(closed={self.closed})"
