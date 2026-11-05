// @ts-check
// @ts-self-types="./index.d.ts"

// pgorm from Node.js and Deno. This module loads the native addon, hands it
// the error and value classes it rejects with and builds values from, and is
// what an application imports.
// [spec:pgorm:def:bindings.api]
// [spec:pgorm:req:bindings.surface+2]
// [spec:pgorm:req:napi.naming+5]
// [spec:pgorm:def:napi.api+1]
// [spec:pgorm:req:napi.typing+1]

import { loadAddon } from "./native.js";
import { installConnections } from "./operations.js";
import { makeError } from "./errors.js";
import { install } from "./values.js";

// Dates and times are Temporal's, which Node.js carries from version 26.
// [spec:pgorm:req:napi.runtimes+1]
if (typeof globalThis.Temporal !== "object") {
  throw new Error(
    "pgorm-napi needs a runtime with Temporal as a global: Node.js 26 or later, or a Deno that carries it",
  );
}

const native = loadAddon();

native.setErrorFactory(makeError);
install(native);
installConnections(native);

export { connect, Connection, Pool, Queryable, Transaction } from "./connections.js";
export { RowStream } from "./streams.js";
export { Listener } from "./listen.js";
export { CopyFrom, copyFrom, CopyOutStream, CopyTo, copyTo } from "./copy.js";
export { CopyRowStream } from "./copy-rows.js";
export { CaseOperand, caseOf, caseWhen, Condition, SearchedCase, SimpleCase } from "./conditions.js";
export { Aliased, all, any, array, bind, call, col, exists, Expr, literal, OrderBy, Quantified, scalar, tuple } from "./expressions.js";
export { Builder } from "./builder.js";
export { LikeExpr, likeExpr } from "./predicates.js";
export { FromItem, Select, select, Table, With } from "./select.js";
export { GroupingElement } from "./grouping.js";
export { TableFunction, tableFunction } from "./table-function.js";
export { Delete, deleteFrom, Insert, insert, ReturningRow, Update, update } from "./writes.js";
export { Conflict, ConflictTarget, ConflictUpdate } from "./conflicts.js";
export { Merge, merge, MergeAction, MergeInsert, MergeUpdate, PendingMerge } from "./merge.js";
export { DataType } from "./data-type.js";
export {
  formatJson,
  isJson,
  isNotJson,
  jsonArray,
  jsonArrayagg,
  jsonArrayQuery,
  JsonDefault,
  jsonDefault,
  jsonExists,
  JsonInput,
  jsonObject,
  jsonObjectagg,
  jsonParse,
  jsonQuery,
  jsonScalar,
  jsonSerialize,
  jsonValue,
} from "./json.js";
export { jsonTable, JsonTableColumn } from "./json-table.js";
export {
  ExtendedWindow,
  Frame,
  FrameCurrentRowStart,
  FrameFollowingStart,
  FramePrecedingStart,
  FrameType,
  Window,
  Windowed,
  WindowFunction,
  windowFunction,
} from "./windows.js";
export { Column, column, ModelColumn } from "./model-columns.js";
export { Model, model } from "./models.js";
export { ModelQuery } from "./model-queries.js";
export {
  FromExpr,
  fromExpr,
  ModelProjection,
  ModelSelector,
  PartialModel,
  partialModel,
  Skip,
  skip,
} from "./model-projections.js";
export { ModelDelete, ModelInsert, ModelInsertMany, ModelRows, ModelUpdate } from "./model-writes.js";
export { Relation } from "./relations.js";
export { Graph } from "./graphs.js";
export { Cursor, Paginator } from "./cursors.js";
export { createDisjoint, disjoint, Disjoint, DisjointEntity, disjointEntities, disjointEntity } from "./disjoint.js";
export {
  addDisjointKind,
  addDisjointKindRecreating,
  addDisjointKindTable,
  removeDisjointKind,
  removeDisjointKindTable,
} from "./disjoint-schema.js";
export { MigrationFile, migrationFile, migrationFiles } from "./migration-file.js";
export { createOneOf, OneOf, oneOf, OneOfEntity, oneOfEntities, oneOfEntity } from "./one-of.js";
export { DisjointCursor, DisjointPaginator, DisjointQuery, DisjointWrite } from "./disjoint-query.js";
export { ActiveModel, entities, Entity, entity } from "./entities.js";
export { EntitySchema, fromEntity } from "./entity-schema.js";
export { EntityColumn, EntityQuery } from "./entity-queries.js";
export { EntityInsert, EntityInsertMany, EntityUpdate, EntityUpdateMany } from "./entity-versions.js";
export { EntityGraph, EntityGraphCursor, EntityGraphQuery, graph, graphs } from "./entity-graphs.js";
export {
  ColumnDef,
  commentOnColumn,
  commentOnTable,
  createTable,
  CreateTable,
  DDL,
  dropTable,
  renameColumn,
  renameConstraint,
  renameTable,
  truncateTable,
} from "./schema.js";
export { AlterTable, alterTable, PendingAlterTable } from "./schema-alter.js";
export {
  AlterComposite,
  alterType,
  createType,
  CreateType,
  dropType,
  PendingAlterType,
  RenameAttribute,
} from "./schema-types.js";
export {
  alterDomain,
  createDomain,
  CreateDomain,
  domainValue,
  dropDomain,
  PendingAlterDomain,
} from "./schema-domains.js";
export {
  AlterSequence,
  alterSequence,
  createExtension,
  createIndex,
  CreateIndex,
  createSchema,
  createSequence,
  CreateSequence,
  dropExtension,
  dropIndex,
  dropSchema,
  dropSequence,
  PendingAlterSequence,
  renameSchema,
  renameSequence,
} from "./schema-objects.js";
export {
  CreateMaterializedView,
  createMaterializedView,
  CreateRecursiveView,
  createRecursiveView,
  CreateTableAs,
  createTableAs,
  CreateView,
  createView,
  dropMaterializedView,
  dropView,
  refreshMaterializedView,
} from "./schema-views.js";
export * as pipeline from "./pipeline.js";
export {
  CheckViolationError,
  ConnectionError,
  ConstructionError,
  DatabaseError,
  DecodeError,
  ExclusionViolationError,
  ForeignKeyViolationError,
  InternalError,
  LifecycleError,
  NotNullViolationError,
  PgormError,
  TimeoutError,
  UniqueViolationError,
} from "./errors.js";
export {
  CreatedMultirange,
  CreatedRange,
  Decimal,
  Interval,
  Multirange,
  Range,
  TypeName,
  Uuid,
  Value,
} from "./values.js";

/** @type {string} */
export const version = native.version;

/** @type {string} */
export const STATEMENT_CACHE_CHANNEL = native.statementCacheChannel;
