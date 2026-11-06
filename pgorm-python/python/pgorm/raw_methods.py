"""The raw-SQL methods a pool, a connection and a transaction share."""

from __future__ import annotations

from typing import TYPE_CHECKING, cast

from . import _native

if TYPE_CHECKING:
    from .models.model import Model
    from .runtime import Connection, Pool
    from .transactions import Transaction


class RawStatements:
    """``batch_execute`` and ``verify``, which a pool runs on a connection of
    its own, a connection on itself and a transaction inside itself, each
    through the native handle it holds."""

    async def batch_execute(self, script: str) -> None:
        """Run ``script``, statements separated by ``;`` with no parameters,
        as Rust's ``batch_execute`` does: sent as it is through the
        simple-query protocol, nothing prepared. It is for DDL and fixed
        scripts, never for values a user supplied."""
        await _native._batch_execute(getattr(self, "_native"), script)

    async def verify(self, statement: _native.RawSQL, model: Model) -> None:
        """Check ``model`` against the result the server describes for
        ``statement``, without running it, as Rust's ``verify`` does: every
        field's column must be returned, of a type the field's kind reads,
        or ``DecodeError`` names the field. The statement's values are
        neither bound nor checked."""
        from .raw import verify

        await verify(cast("Pool | Connection | Transaction", self), statement, model)
