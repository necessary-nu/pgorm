from typing import Any, Literal
from asyncio import CancelledError as CancelledError
from ._schema import (
    DataType as DataType, ColumnDef as ColumnDef, DDL as DDL, EntitySchema as EntitySchema,
    schema_from_entity as schema_from_entity,
    _model_schema as _model_schema, _model_table as _model_table, _renamed as _renamed,
    _ModelColumnCasts as _ModelColumnCasts, _model_casts as _model_casts,
    MigrationFile as MigrationFile, migration_file as migration_file,
)
from ._migrations import SqlMigration as SqlMigration, _Migrations as _Migrations
from ._codegen import _models_from_sql as _models_from_sql
from ._schema_tables import (
    CreateTable as CreateTable, CreateIndex as CreateIndex,
    create_table as create_table, create_index as create_index, drop_table as drop_table,
    rename_table as rename_table, rename_column as rename_column,
    rename_constraint as rename_constraint, truncate_table as truncate_table,
    comment_on_table as comment_on_table, comment_on_column as comment_on_column,
    drop_index as drop_index,
)
from ._schema_alter import (
    PendingAlterTable as PendingAlterTable, AlterTable as AlterTable, alter_table as alter_table,
)
from ._schema_types import (
    CreateType as CreateType, PendingAlterType as PendingAlterType,
    AlterComposite as AlterComposite, RenameAttribute as RenameAttribute,
    create_type as create_type, alter_type as alter_type, drop_type as drop_type,
)
from ._schema_sequences import (
    CreateSequence as CreateSequence, PendingAlterSequence as PendingAlterSequence,
    AlterSequence as AlterSequence, create_sequence as create_sequence,
    alter_sequence as alter_sequence,
    drop_sequence as drop_sequence, rename_sequence as rename_sequence,
    create_extension as create_extension, drop_extension as drop_extension,
)
from ._schema_views import (
    CreateView as CreateView, CreateRecursiveView as CreateRecursiveView,
    CreateMaterializedView as CreateMaterializedView, CreateTableAs as CreateTableAs,
    create_view as create_view, create_recursive_view as create_recursive_view,
    create_materialized_view as create_materialized_view, create_table_as as create_table_as,
    drop_view as drop_view, refresh_materialized_view as refresh_materialized_view,
    drop_materialized_view as drop_materialized_view,
)
from ._schema_schemas import (
    create_schema as create_schema, drop_schema as drop_schema, rename_schema as rename_schema,
)
from ._schema_domains import (
    CreateDomain as CreateDomain, PendingAlterDomain as PendingAlterDomain,
    create_domain as create_domain, alter_domain as alter_domain, drop_domain as drop_domain,
)
from ._pipeline_builder import Pipeline as Pipeline, PipelineSource as PipelineSource, PipelineGrouped as PipelineGrouped
from ._pipeline_expr import PipelineExpr as PipelineExpr, PipelineBinder as PipelineBinder, PipelineOver as PipelineOver
from ._pipeline_sources import SourceSelection as SourceSelection, SelectedSources as SelectedSources
from ._pipeline_functions import (
    pipeline_source as pipeline_source, pipeline_literal as pipeline_literal,
    pipeline_alias as pipeline_alias, pipeline_col as pipeline_col,
    pipeline_role as pipeline_role, pipeline_function as pipeline_function,
    pipeline_case as pipeline_case, pipeline_sources as pipeline_sources,
    pipeline_source_tuples as pipeline_source_tuples,
    pipeline_model_col as pipeline_model_col, pipeline_tuples as pipeline_tuples,
)

__version__: str
__pgorm_version__: str
POSTGRES_TARGET: Literal[18, 19]

class PgormError(Exception):
    is_retryable: bool
class ConstructionError(PgormError):
    parameter: int | None
    reason: str | None
class UnsupportedCapabilityError(ConstructionError): ...
class ConnectionError(PgormError): ...
class DatabaseError(PgormError):
    sqlstate: str | None
    message: str | None
    severity: str | None
    detail: str | None
    hint: str | None
    schema: str | None
    table: str | None
    column: str | None
    datatype: str | None
    constraint: str | None
class UniqueViolationError(DatabaseError): ...
class ForeignKeyViolationError(DatabaseError): ...
class CheckViolationError(DatabaseError): ...
class NotNullViolationError(DatabaseError): ...
class ExclusionViolationError(DatabaseError): ...
class DecodeError(PgormError):
    reading: str | None
    reason: str | None
class RollbackFailedError(PgormError):
    cause: BaseException | None
    rollback: PgormError | None
class TimeoutError(PgormError): ...
class LifecycleError(PgormError): ...
class InternalError(PgormError): ...

def capabilities() -> dict[str, Any]: ...
def _rollback_failed(cause: BaseException, rollback: BaseException) -> RollbackFailedError: ...
def require_capability(operation: str) -> None: ...

class TypeName:
    def __init__(self, name: str, *, schema: str | None = ...) -> None: ...
    @property
    def name(self) -> str: ...
    @property
    def schema(self) -> str | None: ...

class Value:
    def __init__(self, value: Any, kind: str | TypeName | CreatedRange | CreatedMultirange | None = ...) -> None: ...
    @staticmethod
    def null(kind: str | TypeName | CreatedRange | CreatedMultirange) -> Value: ...
    @staticmethod
    def json(value: Any) -> Value: ...
    @staticmethod
    def array(kind: str | TypeName | CreatedRange | CreatedMultirange, values: list[Any] | tuple[Any, ...] | None) -> Value: ...
    @property
    def kind(self) -> str: ...
    @property
    def is_null(self) -> bool: ...
    @property
    def value(self) -> Any: ...
    @property
    def type_name(self) -> TypeName | None: ...
    @property
    def created_type(self) -> CreatedRange | CreatedMultirange | None: ...
    @property
    def element_type(self) -> str | TypeName | CreatedRange | CreatedMultirange | None: ...
    def items(self, /) -> list[Value] | None: ...
    def snapshot(self, /) -> dict[str, Any]: ...
    def _kind_key(self, /) -> str: ...

from ._intervals import Interval as Interval
from ._ranges import (
    Range as Range, Multirange as Multirange,
    CreatedRange as CreatedRange, CreatedMultirange as CreatedMultirange,
)
from ._query_types import (
    _identifier as _identifier, Direction as Direction, Nulls as Nulls,
    Compiled as Compiled, LikeExpr as LikeExpr,
    Aliased as Aliased, OrderBy as OrderBy,
)
from ._models import _model_result_names as _model_result_names, _model_count as _model_count
from ._expressions import (
    Expr as Expr, _DeclaredColumn as _DeclaredColumn, Condition as Condition, col as col, bind as bind,
    literal as literal, call as call, func as func, template as template,
    tuple_expr as tuple_expr, domain_value as domain_value, current_date as current_date,
    current_time as current_time, current_timestamp as current_timestamp,
)
from ._statements import (
    GroupingElement as GroupingElement,
    Select as Select, Join as Join, LockStrength as LockStrength, LockWait as LockWait,
)
from ._subqueries import (
    array as array, exists as exists, scalar as scalar,
    Quantified as Quantified, any_ as any_, all_ as all_,
)
from ._case import (
    SearchedCase as SearchedCase, SimpleCase as SimpleCase, CaseOperand as CaseOperand,
    case_when as case_when, case_of as case_of,
)
from ._windows import (
    FrameType as FrameType, FrameExclusion as FrameExclusion, Frame as Frame,
    FramePrecedingStart as FramePrecedingStart, FrameCurrentRowStart as FrameCurrentRowStart,
    FrameFollowingStart as FrameFollowingStart, Window as Window,
    ExtendedWindow as ExtendedWindow, Windowed as Windowed,
    WindowFunction as WindowFunction, window_function as window_function,
)
from ._json import (
    JsonInput as JsonInput, JsonKind as JsonKind, JsonExistsBehavior as JsonExistsBehavior,
    JsonValueBehavior as JsonValueBehavior, JsonQueryBehavior as JsonQueryBehavior,
    JsonDefault as JsonDefault, JsonTableBehavior as JsonTableBehavior,
    JsonTableColumn as JsonTableColumn, json_table as json_table, json_exists as json_exists, json_value as json_value,
    json_query as json_query, json_object as json_object, json_array as json_array,
    json_array_query as json_array_query, json_objectagg as json_objectagg,
    json_arrayagg as json_arrayagg, json_parse as json_parse, json_scalar as json_scalar,
    json_serialize as json_serialize, format_json as format_json,
    is_json as is_json, is_not_json as is_not_json,
)
from ._results import Record as Record, Field as Field
from ._entity_raw import EntityRawQuery as EntityRawQuery
from ._entities import Entity as Entity, EntityQuery as EntityQuery, entity as entity, entities as entities
from ._entity_columns import EntityColumn as EntityColumn
from ._entity_reads import EntityRelation as EntityRelation, EntitySelection as EntitySelection, EntityPaginator as EntityPaginator, EntityCursor as EntityCursor, _RowDecoder as _RowDecoder
from ._versions import (
    EntityUpdateMany as EntityUpdateMany, EntityInsert as EntityInsert,
    Change as Change, Upserted as Upserted, _change as _change, _inserted as _inserted,
    _updated as _updated, EntityUpdate as EntityUpdate,
    EntityInsertMany as EntityInsertMany,
)
from ._entity_try_inserts import EntityTryInsert as EntityTryInsert, EntityTryInsertMany as EntityTryInsertMany
from ._graphs import EntityGraph as EntityGraph, EntityGraphQuery as EntityGraphQuery, EntityGraphCursor as EntityGraphCursor, graph as graph, graphs as graphs
from ._entity_models import EntityModel as EntityModel, ActiveModel as ActiveModel, ActiveValue as ActiveValue, ActiveState as ActiveState
from ._disjoint import (
    _Disjoint as _Disjoint, _DisjointChange as _DisjointChange, _disjoint_model as _disjoint_model,
    _disjoint_entity as _disjoint_entity, _disjoint_entities as _disjoint_entities,
)
from ._one_of import (
    _OneOf as _OneOf, _one_of_model as _one_of_model, _one_of_entity as _one_of_entity,
    _one_of_entities as _one_of_entities,
)
from ._writes import (
    Insert as Insert, Update as Update, Delete as Delete, RawSQL as RawSQL,
    sql as sql, prql as prql, _raw_select as _raw_select, _batch_execute as _batch_execute,
    _describe as _describe,
    Conflict as Conflict, ConflictTarget as ConflictTarget, ConflictUpdate as ConflictUpdate,
)
from ._merge import (
    MergeAction as MergeAction, Overriding as Overriding,
    MergeUpdate as MergeUpdate, MergeInsert as MergeInsert, PendingMerge as PendingMerge,
    Merge as Merge, With as With, SearchOrder as SearchOrder, Search as Search, Cycle as Cycle,
    ReturningRow as ReturningRow, merge as merge,
)
from ._copy import (
    CopyFrom as CopyFrom, CopyTo as CopyTo, _CopyIn as _CopyIn, _CopyOut as _CopyOut,
    _copy_in as _copy_in, _copy_out as _copy_out, _copy_columns as _copy_columns,
)
from ._copy_rows import (
    CopyColumn as CopyColumn, CopyRowEncoder as CopyRowEncoder, CopyRowDecoder as CopyRowDecoder,
)
from ._listen import (
    Notification as Notification, _Listener as _Listener, _connect_listener as _connect_listener,
)
from ._retrying import _Retrying as _Retrying
from ._runtime import (
    _Pool as _Pool, _PoolShape as _PoolShape, _HookConnection as _HookConnection,
    _Connection as _Connection, _Stream as _Stream,
    _Transaction as _Transaction,
    STATEMENT_CACHE_CHANNEL as STATEMENT_CACHE_CHANNEL,
)
from ._from_items import (
    Table as Table, FromItem as FromItem, SampleMethod as SampleMethod,
    TableFunction as TableFunction,
)
from ._metrics import (
    METRICS_HOOKS as METRICS_HOOKS, QueryContext as QueryContext,
    QueryFingerprint as QueryFingerprint, _MetricsAnswer as _MetricsAnswer,
)
