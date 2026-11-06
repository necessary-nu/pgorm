"""pgorm-migration's runner: a program's migrations applied in order, each
once, and recorded in the ledger, as Rust's ``Migrations`` applies them."""

from __future__ import annotations

import enum
import logging
from collections.abc import Sequence
from typing import TYPE_CHECKING, NamedTuple, Protocol, runtime_checkable

from .. import _native
from .._native import ConstructionError, SqlMigration

if TYPE_CHECKING:
    from ..runtime import Connection, Pool
    from ..transactions import Transaction


# [spec:pgorm:req:bindings.migrations]
@runtime_checkable
class MigrationTrait(Protocol):
    """A migration written in Python: a ``name``, the migration's identity in
    the ledger, and an ``up`` the runner awaits in the batch's transaction.
    An optional ``checksum`` (a ``str`` or ``None``) is recorded and checked
    as Rust's ``MigrationTrait::checksum`` is, and an optional
    ``commits_after`` (``False`` unless given) commits the batch once the
    migration has run. All three are read once, as ``Migrations`` is built."""

    @property
    def name(self) -> str: ...

    async def up(self, tx: Transaction) -> None: ...


class MigrationStatus(enum.Enum):
    """Whether the ledger records a migration applied."""

    Pending = "pending"
    Applied = "applied"

    def __bool__(self) -> bool:
        raise ConstructionError("a migration status cannot be tested as a Python boolean")


class Migration(NamedTuple):
    """A migration's name and its status, as pgorm-migration's ``Migration``
    reports them."""

    name: str
    status: MigrationStatus


async def _run_up(migration: MigrationTrait, native: _native._Transaction) -> None:
    """What the runner awaits for a Python migration: its ``up``, given the
    transaction the runner lends it."""
    from ..transactions import Transaction

    await migration.up(Transaction(native, None))


# [spec:pgorm:req:bindings.migrations]
class Migrations:
    """A migrator built at run time, pgorm-migration's ``Migrations``: the
    migrations it applies, in order, each a ``SqlMigration`` or a migration
    written in Python, and where its ledger lives. ``migration_table_name``
    and ``migration_table_schema`` place the ledger, ``pgorm_migrations``
    wherever ``search_path`` resolves it unless given, and
    ``statement_cache_channel`` names the channel a batch that applied a
    migration notifies, ``STATEMENT_CACHE_CHANNEL`` unless given; ``None``
    notifies nothing."""

    def __init__(
        self,
        migrations: Sequence[SqlMigration | MigrationTrait],
        *,
        migration_table_name: str | None = None,
        migration_table_schema: str | None = None,
        statement_cache_channel: str | None = _native.STATEMENT_CACHE_CHANNEL,
    ) -> None:
        self._native = _native._Migrations(
            list(migrations), migration_table_name, migration_table_schema, statement_cache_channel
        )

    async def up(self, pool: Pool, steps: int | None = None) -> None:
        """Apply the pending migrations, at most ``steps`` of them when given,
        as pgorm-migration's ``up`` applies them: in one transaction on a
        connection of ``pool``'s own, under the advisory lock, committing
        after a migration whose ``commits_after`` is true, then clearing the
        pool's statement caches and notifying the channel. A migration's
        exception rolls the batch back and is raised as itself."""
        await self._native.up(pool, steps)

    async def install(self, executor: Pool | Connection | Transaction) -> None:
        """Create the ledger, and its schema, where either is missing, and
        adopt a ledger SeaORM left under ``seaql_migrations``."""
        await self._native.install(executor)

    async def get_migration_with_status(self, executor: Pool | Connection | Transaction) -> list[Migration]:
        """Every migration with its status, read from the ledger and nothing
        else: a database never migrated reports every migration pending, and
        gains no ledger. An edited migration whose checksum no longer matches,
        and a ledger row with no migration, are a ``ConstructionError``."""
        found = await self._native.status(executor)
        return [
            Migration(name, MigrationStatus.Applied if applied else MigrationStatus.Pending)
            for name, applied in found
        ]

    async def get_pending_migrations(self, executor: Pool | Connection | Transaction) -> list[Migration]:
        """The migrations the ledger does not record, in order."""
        return [
            migration
            for migration in await self.get_migration_with_status(executor)
            if migration.status is MigrationStatus.Pending
        ]

    async def get_applied_migrations(self, executor: Pool | Connection | Transaction) -> list[Migration]:
        """The migrations the ledger records, in order."""
        return [
            migration
            for migration in await self.get_migration_with_status(executor)
            if migration.status is MigrationStatus.Applied
        ]

    async def status(
        self, executor: Pool | Connection | Transaction, logger: logging.Logger | None = None
    ) -> None:
        """Log each migration's name and status at ``INFO`` to ``logger``,
        ``pgorm.migration`` unless another is given, as Rust's ``status``
        writes its ``tracing`` events."""
        logger = logger if logger is not None else logging.getLogger("pgorm.migration")
        logger.info("Checking migration status")
        for migration in await self.get_migration_with_status(executor):
            logger.info("Migration '%s'... %s", migration.name, migration.status.name)

    def __bool__(self) -> bool:
        raise ConstructionError("a migrator cannot be tested as a Python boolean")

    def __repr__(self) -> str:
        return f"Migrations({self._native!r})"


__all__ = ["Migration", "MigrationStatus", "MigrationTrait", "Migrations", "SqlMigration"]
