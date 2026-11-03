"""Explicit native DDL builders and schema generation for registered Rust entities.

Construction performs no database work. Execute statements explicitly through
``await pool.execute(statement)`` or ``await connection.execute(statement)``.
Every complete statement is a ``DDL``; an ``ALTER`` that names its object and
no action yet (``PendingAlterTable`` and its kin) is not, and its first action
gives the statement. A statement is made by its function (``create_table``,
``alter_table`` and the rest), never by its class: the classes are exported
for annotations and ``isinstance`` and cannot be called.
"""

from typing import TYPE_CHECKING, Literal

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


if TYPE_CHECKING:
    from ..models.disjoint import Disjoint, DisjointEntity


# [spec:pgorm:req:bindings.disjoint+1]
# [spec:pgorm:req:bindings.disjoint-unkinded]
def create_disjoint(
    disjoint: 'Disjoint[Literal["kinded"]] | DisjointEntity[Literal["kinded"]]',
) -> list[DDL]:
    """The statements creating a disjoint model's kind type, tables and the
    constraints that hold each base row to exactly its kind's table, in the
    order they run: pgorm's ``DisjointShape::create``, which
    ``Schema::create_disjoint`` writes for a registered enum. A model column
    whose default the declaration does not state has no ``DEFAULT`` to write,
    so its table is refused; the types the tables' other columns use are not
    among the statements."""
    from ..models.disjoint_query import _Disjoint

    if not isinstance(disjoint, _Disjoint):
        raise TypeError("create_disjoint takes a Disjoint or a DisjointEntity")
    return list(disjoint._native.statements())


__all__ = [
    "DataType", "ColumnDef", "DDL", "EntitySchema", "from_entity", "create_disjoint",
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
