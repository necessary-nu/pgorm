"""Explicit native DDL builders and schema generation for registered Rust entities.

Construction performs no database work. Execute statements explicitly through
``await pool.execute(statement)`` or ``await connection.execute(statement)``.
Every complete statement is a ``DDL``; an ``ALTER`` that names its object and
no action yet (``PendingAlterTable`` and its kin) is not, and its first action
gives the statement. A statement is made by its function (``create_table``,
``alter_table`` and the rest), never by its class: the classes are exported
for annotations and ``isinstance`` and cannot be called.
"""

from typing import TYPE_CHECKING, Any, Literal, TypeAlias, cast

# [spec:pgorm:req:bindings.schema-api+2]
from .._native import (
    DataType as DataType, ColumnDef as ColumnDef, DDL as DDL, EntitySchema as EntitySchema,
    schema_from_entity as from_entity,
    CreateTable as CreateTable, CreateIndex as CreateIndex,
    create_table as create_table, create_index as create_index, drop_table as drop_table,
    rename_table as rename_table, rename_column as rename_column,
    rename_constraint as rename_constraint, truncate_table as truncate_table,
    comment_on_table as comment_on_table, comment_on_column as comment_on_column,
    drop_index as drop_index,
    PendingAlterTable as PendingAlterTable, AlterTable as AlterTable, alter_table as alter_table,
    CreateType as CreateType, PendingAlterType as PendingAlterType,
    AlterComposite as AlterComposite, RenameAttribute as RenameAttribute,
    create_type as create_type, alter_type as alter_type, drop_type as drop_type,
    CreateSequence as CreateSequence, PendingAlterSequence as PendingAlterSequence,
    AlterSequence as AlterSequence, create_sequence as create_sequence,
    alter_sequence as alter_sequence,
    drop_sequence as drop_sequence, rename_sequence as rename_sequence,
    create_extension as create_extension, drop_extension as drop_extension,
    CreateView as CreateView, CreateRecursiveView as CreateRecursiveView,
    CreateMaterializedView as CreateMaterializedView, CreateTableAs as CreateTableAs,
    create_view as create_view, create_recursive_view as create_recursive_view,
    create_materialized_view as create_materialized_view, create_table_as as create_table_as,
    drop_view as drop_view, refresh_materialized_view as refresh_materialized_view,
    drop_materialized_view as drop_materialized_view,
    create_schema as create_schema, drop_schema as drop_schema, rename_schema as rename_schema,
    CreateDomain as CreateDomain, PendingAlterDomain as PendingAlterDomain,
    create_domain as create_domain, alter_domain as alter_domain, drop_domain as drop_domain,
)


from .._native import Table, _Disjoint as _NativeDisjoint
from .changes import DisjointChange, DisjointGap, DisjointStep

if TYPE_CHECKING:
    from .._registered.disjoints import WritableDisjointView
    from ..models.disjoint import Disjoint, DisjointEntity
    from ..models.one_of import OneOf, OneOfEntity

# A disjoint model or registration whose base holds each row's kind, which
# its schema and kind changes take: a type checker refuses an unkinded one.
# [spec:pgorm:req:bindings.disjoint-unkinded]
KindedDisjoint: TypeAlias = (
    'Disjoint[Literal["kinded"]] | DisjointEntity[Literal["kinded"], Any] '
    "| WritableDisjointView[Any, Any, Any, Any]"
)


def _shape_of(disjoint: object, function: str) -> _NativeDisjoint:
    from .._registered.disjoints import DisjointView
    from ..models.disjoint_query import _Disjoint

    if isinstance(disjoint, DisjointView):
        disjoint = disjoint._entity
    if not isinstance(disjoint, _Disjoint):
        raise TypeError(f"{function} takes a Disjoint or a DisjointEntity")
    return disjoint._native


# [spec:pgorm:req:bindings.disjoint+2]
# [spec:pgorm:req:bindings.disjoint-unkinded]
# [spec:pgorm:req:bindings.codegen-disjoint]
def create_disjoint(disjoint: KindedDisjoint) -> list[DDL]:
    """The statements creating a disjoint model's kind type, tables and the
    constraints that hold each base row to exactly its kind's table, in the
    order they run: pgorm's ``DisjointShape::create``, which
    ``Schema::create_disjoint`` writes for a registered enum. A model column
    whose default the declaration does not state has no ``DEFAULT`` to write,
    so its table is refused; the types the tables' other columns use are not
    among the statements."""
    return list(_shape_of(disjoint, "create_disjoint").statements())


# [spec:pgorm:req:bindings.disjoint-evolve]
def _change(
    disjoint: object, function: str, change: str, value: str, table: str | Table | None = None
) -> DisjointChange:
    native = _shape_of(disjoint, function)
    if type(value) is not str:
        raise TypeError("a kind's value is a str")
    return DisjointChange(
        tuple(
            DisjointStep(
                tuple(statements),
                None if gap is None else DisjointGap(gap[0], Table(gap[2], schema=gap[1])),
            )
            for statements, gap in native.change(change, value, table)
        )
    )


def add_disjoint_kind(disjoint: KindedDisjoint, value: str) -> DisjointChange:
    """Add the kind ``value``, which the declaration holds and the database's
    kind type does not yet: ``DisjointShape::add_kind``, in two steps for a
    kind with a table, since PostgreSQL uses an added value only once the
    transaction adding it has committed."""
    return _change(disjoint, "add_disjoint_kind", "add_kind", value)


def add_disjoint_kind_recreating(disjoint: KindedDisjoint, value: str) -> DisjointChange:
    """Add the kind ``value`` in one step that recreates the kind type and
    rewrites the base and every kind table: ``DisjointShape::add_kind_recreating``."""
    return _change(disjoint, "add_disjoint_kind_recreating", "add_kind_recreating", value)


def remove_disjoint_kind(disjoint: KindedDisjoint, value: str) -> DisjointChange:
    """Remove the value ``value``, which the database's kind type holds and
    the declaration does not, in one step that recreates the kind type and
    fails while any base row has the kind: ``DisjointShape::remove_kind``."""
    return _change(disjoint, "remove_disjoint_kind", "remove_kind", value)


def add_disjoint_kind_table(disjoint: KindedDisjoint, value: str) -> DisjointChange:
    """Give the kind ``value``, a kind without a table in the database, the
    table the declaration gives it, in two steps with a gap between them for
    the rows its base rows lack: ``DisjointShape::add_kind_table``."""
    return _change(disjoint, "add_disjoint_kind_table", "add_kind_table", value)


def remove_disjoint_kind_table(
    disjoint: KindedDisjoint, value: str, table: str | Table
) -> DisjointChange:
    """Drop ``table``, the table of the kind ``value``, which the declaration
    gives none, and the base's generated key columns and foreign key for it,
    in one step that loses every row of the table:
    ``DisjointShape::remove_kind_table``."""
    return _change(disjoint, "remove_disjoint_kind_table", "remove_kind_table", value, table)


# [spec:pgorm:req:bindings.one-of+1]
def create_one_of(one_of: "OneOf | OneOfEntity") -> AlterTable:
    """The ``ALTER TABLE`` adding the ``CHECK`` that holds every row of a
    one-of reference to one target, or to one at most where it is optional:
    pgorm's ``OneOfShape::check``, which ``Schema::create_one_of`` writes for
    a registered enum."""
    from ..models.one_of import _OneOf

    if not isinstance(one_of, _OneOf):
        raise TypeError("create_one_of takes a OneOf or a OneOfEntity")
    return cast(AlterTable, one_of._native.statement())


__all__ = [
    "DataType", "ColumnDef", "DDL", "EntitySchema", "from_entity", "create_disjoint", "create_one_of",
    "add_disjoint_kind", "add_disjoint_kind_recreating", "remove_disjoint_kind",
    "add_disjoint_kind_table", "remove_disjoint_kind_table", "DisjointChange", "DisjointStep",
    "DisjointGap",
    "CreateTable", "create_table", "PendingAlterTable", "AlterTable", "alter_table",
    "drop_table", "rename_table", "rename_column", "rename_constraint", "truncate_table",
    "comment_on_table", "comment_on_column",
    "CreateIndex", "create_index", "drop_index",
    "CreateType", "create_type", "PendingAlterType", "AlterComposite", "RenameAttribute",
    "alter_type", "drop_type",
    "CreateDomain", "create_domain", "PendingAlterDomain", "alter_domain", "drop_domain",
    "CreateSequence", "create_sequence", "PendingAlterSequence", "AlterSequence",
    "alter_sequence", "drop_sequence", "rename_sequence", "create_extension", "drop_extension",
    "CreateView", "create_view", "CreateRecursiveView", "create_recursive_view",
    "CreateMaterializedView", "create_materialized_view", "CreateTableAs", "create_table_as",
    "drop_view", "refresh_materialized_view", "drop_materialized_view",
    "create_schema", "drop_schema", "rename_schema",
]
