/**
 * DDL over pgorm-query's builders: tables, their columns, keys and
 * constraints and their alterations, indexes, types, domains, sequences,
 * views, materialized views and tables created from a query, extensions and
 * comments.
 *
 * A DDL statement takes no parameters, so each renders with every value it
 * carries — a column's `DEFAULT`, a `CHECK`'s operands — written as
 * pgorm-query's escaped literal, and runs through `execute` as SQL text.
 * `inspect()` gives that SQL and no values. Constructing a statement does no
 * I/O; nothing runs DDL but a terminal.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.schema]
// [spec:pgorm:req:bindings.schema-api+3]

import type { CreatedMultirange, CreatedRange, TypeName } from "./index.d.ts";
import type { Builder, Expr, Operand, Predicate } from "./expressions.d.ts";
import type { BuiltinType, DataType } from "./json.d.ts";
import type { Select, Table } from "./select.d.ts";
import type { Entity } from "./entities.d.ts";
import type { Model, Relation } from "./models.d.ts";

/**
 * A table or sequence a statement names: a name, or a {@link Table} without
 * an alias, whose schema qualifies it.
 */
export type TableRef = string | Table;

/** A type a statement names: a name, or a {@link TypeName}, whose schema qualifies it. */
export type TypeRef = string | TypeName;

export type ColumnType = DataType | BuiltinType | TypeName | CreatedRange | CreatedMultirange;

export type Collation = string | { readonly name: string; readonly schema?: string };

/** One column, or a non-empty list of them in order. */
export type KeyColumns = string | readonly [string, ...string[]];

/** When a constraint is checked: row by row, at each statement's end, or at commit. */
export type Deferrability = "notDeferrable" | "deferrableInitiallyImmediate" | "deferrableInitiallyDeferred";

/** PostgreSQL 18's `ENFORCED` / `NOT ENFORCED`; a `NOT ENFORCED` constraint is recorded and never checked. */
export type Enforcement = "enforced" | "notEnforced";

/** What a drop does with the objects that depend on what it drops. */
export type DropBehavior = "restrict" | "cascade";

export type ReferentialAction = "noAction" | "restrict" | "cascade" | "setNull" | "setDefault";

/**
 * A sequence's options, a standalone sequence's and an identity column's
 * alike: integers within `bigint`, a bound's `null` being its `NO` form.
 */
export interface SequenceOptions {
  readonly incrementBy?: number | bigint;
  readonly minValue?: number | bigint | null;
  readonly maxValue?: number | bigint | null;
  readonly startWith?: number | bigint;
  readonly cache?: number | bigint;
  readonly cycle?: boolean;
}

/** A `CHECK`'s options. `noInherit` keeps it from tables that inherit this one. */
export interface CheckOptions {
  readonly name?: string;
  readonly enforcement?: Enforcement;
  readonly noInherit?: boolean;
}

/**
 * A key's options. `withoutOverlaps` names PostgreSQL 18's period column, a
 * range or multirange, written last: rows may share the other columns only
 * while their periods do not overlap.
 */
export interface KeyOptions {
  readonly name?: string;
  readonly include?: readonly string[];
  readonly deferrability?: Deferrability;
  readonly withoutOverlaps?: string;
}

/** A unique key's options, which alone take `NULLS NOT DISTINCT`. */
export interface UniqueOptions extends KeyOptions {
  readonly nullsNotDistinct?: boolean;
}

/**
 * A foreign key's options. `period` is PostgreSQL 18's `PERIOD` pair,
 * `[column, referenced column]`, written last on both sides.
 */
export interface ForeignKeyOptions {
  readonly name?: string;
  readonly onDelete?: ReferentialAction;
  readonly onUpdate?: ReferentialAction;
  readonly deferrability?: Deferrability;
  readonly enforcement?: Enforcement;
  readonly period?: readonly [string, string];
}

/** What adds a constraint to a table that has rows: `notValid` leaves them unchecked. */
export interface NotValidOption {
  readonly notValid?: boolean;
}

/** A complete DDL statement. It inspects as it runs, with no values. */
export declare class DDL extends Builder {
  protected constructor();
}

/**
 * A table's column: its name and type, then each clause in the order it is
 * added. The type is left out only for `modifyColumn`, which then does not
 * retype the column.
 */
export declare class ColumnDef {
  constructor(name: string, type?: ColumnType);
  /** The column's one `NOT NULL` constraint, named and kept from inheriting tables as asked. */
  notNull(options?: { readonly name?: string; readonly noInherit?: boolean }): ColumnDef;
  null(): ColumnDef;
  /** `DEFAULT`: an expression, or a value written as an escaped literal. */
  default(value: Operand): ColumnDef;
  check(condition: Expr, options?: CheckOptions): ColumnDef;
  /** The kind is always named: PostgreSQL 17 refuses a generated column without one, and 18 reads it as virtual. */
  generated(expression: Expr, kind: "stored" | "virtual"): ColumnDef;
  /** The form PostgreSQL recommends over the serial family; its sequence takes a standalone sequence's options. */
  identity(generation: "always" | "byDefault", options?: SequenceOptions): ColumnDef;
  /** The serial family in place of an integer type; prefer {@link ColumnDef.identity}. */
  autoIncrement(): ColumnDef;
  collate(collation: string, options?: { readonly schema?: string }): ColumnDef;
}

/** `CREATE TABLE`; every method returns a new statement. */
export declare class CreateTable extends DDL {
  /** A column, which needs its type. */
  column(column: ColumnDef): CreateTable;
  ifNotExists(): CreateTable;
  /** The table's one primary key; a later call replaces it. */
  primaryKey(columns: KeyColumns, options?: KeyOptions): CreateTable;
  unique(columns: KeyColumns, options?: UniqueOptions): CreateTable;
  /** Columns paired in order with as many columns of the table they reference. */
  foreignKey(columns: KeyColumns, references: TableRef, refColumns: KeyColumns, options?: ForeignKeyOptions): CreateTable;
  check(condition: Expr, options?: CheckOptions): CreateTable;
}

export declare function createTable(table: TableRef): CreateTable;

/** The actions an `ALTER TABLE` takes, before its first and after it alike. */
export interface AlterTableActions {
  addColumn(column: ColumnDef, options?: { readonly ifNotExists?: boolean }): AlterTable;
  /**
   * The column's type, nullability, default, `CHECK` or identity changed. A
   * generated expression, the serial family or a collation without a type,
   * which no such action writes, is refused.
   */
  modifyColumn(column: ColumnDef): AlterTable;
  dropColumn(name: string): AlterTable;
  addPrimaryKey(columns: KeyColumns, options?: KeyOptions): AlterTable;
  addUnique(columns: KeyColumns, options?: UniqueOptions): AlterTable;
  addForeignKey(
    columns: KeyColumns,
    references: TableRef,
    refColumns: KeyColumns,
    options?: ForeignKeyOptions & NotValidOption,
  ): AlterTable;
  addCheck(condition: Expr, options?: CheckOptions & NotValidOption): AlterTable;
  /** PostgreSQL 18's table-level `NOT NULL`, the one spelling that can be `NOT VALID`. */
  addNotNull(
    column: string,
    options?: { readonly name?: string; readonly noInherit?: boolean } & NotValidOption,
  ): AlterTable;
  /** Which kind the name holds — a key, a foreign key, a `CHECK`, a `NOT NULL` — is the server's knowledge. */
  dropConstraint(name: string, options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior }): AlterTable;
  /** Checks the rows a `NOT VALID` constraint left unchecked; the server refuses any other kind. */
  validateConstraint(name: string): AlterTable;
  /** Whether a `NOT NULL` passes to inheriting tables, or whether a foreign key is enforced. */
  alterConstraint(name: string, change: "inherit" | "noInherit" | "enforced" | "notEnforced"): AlterTable;
  /** A generated column's new expression, which the rows already written take. */
  setExpression(column: string, expression: Expr): AlterTable;
  /** A stored generated column made plain, keeping its values. */
  dropExpression(column: string, options?: { readonly ifExists?: boolean }): AlterTable;
}

/**
 * An `ALTER TABLE` before its first action. PostgreSQL parses none without
 * one, so it has no `inspect()`, and a terminal refuses it with a
 * `ConstructionError`.
 */
export declare class PendingAlterTable {
  #private;
  private constructor();
}
export interface PendingAlterTable extends AlterTableActions {}

/** An `ALTER TABLE` with at least one action, applied in order; every method returns a new one. */
export declare class AlterTable extends DDL {}
export interface AlterTable extends AlterTableActions {}

export declare function alterTable(table: TableRef): PendingAlterTable;

export declare function dropTable(
  tables: TableRef | readonly [TableRef, ...TableRef[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

/** The new name is bare, since a rename cannot move a table to another schema. */
export declare function renameTable(table: TableRef, name: string): DDL;

export declare function renameColumn(table: TableRef, name: string, newName: string): DDL;

/** A constraint of any kind renamed, a key's index with it. */
export declare function renameConstraint(table: TableRef, name: string, newName: string): DDL;

export declare function truncateTable(table: TableRef): DDL;

export declare function commentOnTable(table: TableRef, text: string): DDL;

export declare function commentOnColumn(table: TableRef, column: string, text: string): DDL;

/**
 * An index entry: a column's name, an expression, or either with an order and
 * an operator class.
 */
export type IndexColumn =
  | string
  | Expr
  | {
    readonly on: string | Expr;
    readonly order?: "asc" | "desc";
    readonly operatorClass?: string;
  };

/** `CREATE INDEX`; every method returns a new statement. */
export declare class CreateIndex extends DDL {
  column(column: IndexColumn): CreateIndex;
  unique(): CreateIndex;
  /** `NULLS NOT DISTINCT`, which PostgreSQL defines for a unique index alone, so the index becomes one. */
  nullsNotDistinct(): CreateIndex;
  ifNotExists(): CreateIndex;
  /** The access method: `btree`, `hash`, `gin`, `gist`, `spgist`, `brin`, or another the server has. */
  using(method: string): CreateIndex;
  /** Columns carried in the index's leaves without being indexed; appends. */
  include(columns: readonly string[]): CreateIndex;
  /** The partial index's predicate, ANDed to one already there. */
  where(predicate: Predicate): CreateIndex;
}

/** An index over its first entry, named by the options or by PostgreSQL. */
export declare function createIndex(
  table: TableRef,
  column: IndexColumn,
  options?: { readonly name?: string },
): CreateIndex;

/** An index dropped from its table's schema. */
export declare function dropIndex(
  table: TableRef,
  name: string,
  options?: { readonly ifExists?: boolean },
): DDL;

export interface CollationOption {
  readonly collation?: Collation;
}

/**
 * A registered entity's schema as pgorm's own entity schema path writes it:
 * `Schema::create_table_from_entity` with its enum, index and comment
 * statements, each in the order pgorm writes them.
 */
// [spec:pgorm:req:bindings.schema-from-entity]
export declare class EntitySchema {
  private constructor();
  /** `CREATE TABLE`, with its primary key, unique keys and foreign keys. */
  readonly table: CreateTable;
  /** `CREATE TYPE .. AS ENUM` for each enum type its columns name. */
  readonly enums: readonly CreateType[];
  /** `CREATE INDEX` for each column it indexes. */
  readonly indexes: readonly CreateIndex[];
  /** `COMMENT ON` its table and its columns. */
  readonly comments: readonly DDL[];
}

/**
 * The schema of a registered entity, by pgorm's own entity schema path. A
 * table that path refuses, such as one whose foreign key PostgreSQL cannot
 * build, is a `ConstructionError` carrying the Rust message.
 */
export declare function fromEntity(entity: Entity<any>): EntitySchema;

/**
 * The schema of a declared model's table, written by pgorm's own `Schema`
 * from the `TableShape` the model describes: its `CREATE TABLE` with its
 * primary key, unique keys and a foreign key for each `belongsTo` relation
 * in `relations`, the `CREATE TYPE` of each enum its columns use, an index
 * for each `indexed` column, and its comments. A relation of another kind, or
 * one starting at another table, and a column filled by a `default` the
 * declaration does not state, are a `ConstructionError`.
 */
// [spec:pgorm:req:bindings.schema-from-model]
export declare function fromModel(model: Model<any>, options?: { readonly relations?: readonly Relation<any, any, any>[] }): EntitySchema;

/**
 * `CREATE TYPE`: a shell type until a kind is chosen. What a type is, is one
 * slot, so choosing a kind replaces the one before.
 */
export declare class CreateType extends DDL {
  /** An enumeration, with no labels until `values` appends them. */
  asEnum(): CreateType;
  /** Labels appended to an enumeration, which the type becomes: data of at most 63 bytes, written as literals. */
  values(labels: readonly string[]): CreateType;
  /** A composite, with no attributes until `attribute` appends them. */
  asComposite(): CreateType;
  /** An attribute appended to a composite, which the type becomes. */
  attribute(name: string, type: ColumnType, options?: CollationOption): CreateType;
  /** A range over `subtype`, the one option PostgreSQL requires. */
  asRange(
    subtype: ColumnType,
    options?: CollationOption & {
      readonly subtypeOpclass?: string;
      readonly subtypeDiff?: string;
      readonly multirangeTypeName?: TypeRef;
    },
  ): CreateType;
}

export declare function createType(name: TypeRef): CreateType;

/** The attribute changes a composite's `ALTER TYPE` takes. */
export interface AttributeChanges {
  addAttribute(name: string, type: ColumnType, options?: CollationOption): AlterComposite;
  dropAttribute(name: string, options?: { readonly ifExists?: boolean }): AlterComposite;
  alterAttribute(name: string, type: ColumnType, options?: CollationOption): AlterComposite;
}

/**
 * An `ALTER TYPE` before its change. PostgreSQL parses none without one, so
 * it has no `inspect()`, and a terminal refuses it.
 */
export declare class PendingAlterType {
  private constructor();
  /** A label added to an enumeration: at its end, or before or after one. */
  addValue(label: string, options?: { readonly before: string } | { readonly after: string }): DDL;
  /** The type renamed; the new name is bare, the type staying in its schema. */
  renameTo(name: string): DDL;
  renameValue(label: string, newLabel: string): DDL;
  /** An attribute renamed: PostgreSQL takes `RENAME` as an `ALTER TYPE`'s sole action. */
  renameAttribute(name: string, newName: string): RenameAttribute;
}
export interface PendingAlterType extends AttributeChanges {}

/** A composite's attribute changes, applied in order; every method returns a new statement. */
export declare class AlterComposite extends DDL {
  /** `CASCADE`, which carries the changes into typed tables, or `RESTRICT`; the last call wins. */
  behavior(behavior: DropBehavior): AlterComposite;
}
export interface AlterComposite extends AttributeChanges {}

/** A composite's attribute renamed. */
export declare class RenameAttribute extends DDL {
  behavior(behavior: DropBehavior): RenameAttribute;
}

export declare function alterType(name: TypeRef): PendingAlterType;

export declare function dropType(
  names: TypeRef | readonly [TypeRef, ...TypeRef[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

// [spec:pgorm:req:bindings.schema-domains+2]

/**
 * `CREATE DOMAIN`; every method returns a new statement. The clauses render
 * in one order whatever order they are given in.
 */
export declare class CreateDomain extends DDL {
  /** The collation the domain's values compare under, for a base type that has one. */
  collate(collation: string, options?: { readonly schema?: string }): CreateDomain;
  /** `DEFAULT`: an expression, or a value written as an escaped literal; a later default replaces it. */
  default(value: Operand): CreateDomain;
  /** `NOT NULL`, replacing a `null()`: PostgreSQL refuses the two together. */
  notNull(): CreateDomain;
  /** `NULL`, the default, replacing a `notNull()`. */
  null(): CreateDomain;
  /**
   * A `CHECK` over {@link domainValue}, appended. It takes no `notValid`:
   * PostgreSQL has no `NOT VALID` at `CREATE DOMAIN`.
   */
  check(condition: Expr, options?: { readonly name?: string }): CreateDomain;
}

/** A domain over `base`: any column type, another domain's `TypeName` included. */
export declare function createDomain(name: TypeRef, base: ColumnType): CreateDomain;

/**
 * An `ALTER DOMAIN` before its action. PostgreSQL parses none without one,
 * nor one with two, so it has no `inspect()`, a terminal refuses it, and each
 * action gives the whole statement. `ALTER DOMAIN` has no `IF EXISTS`.
 */
export declare class PendingAlterDomain {
  private constructor();
  /** The default columns of the domain take from now on, where they have none of their own. */
  setDefault(value: Operand): DDL;
  dropDefault(): DDL;
  /** Refused while a column of the domain holds a null (`23502`). */
  setNotNull(): DDL;
  dropNotNull(): DDL;
  /**
   * A `CHECK` added: checked against the values already held (`23514` for one
   * that fails), or, `notValid`, left unchecked until `validateConstraint`.
   */
  addCheck(condition: Expr, options?: { readonly name?: string } & NotValidOption): DDL;
  dropConstraint(name: string, options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior }): DDL;
  renameConstraint(name: string, newName: string): DDL;
  /** Checks the values a `NOT VALID` check left unchecked (`23514` for one that fails). */
  validateConstraint(name: string): DDL;
  /** The domain renamed; the new name is bare, the domain staying in its schema. */
  renameTo(name: string): DDL;
}

export declare function alterDomain(name: TypeRef): PendingAlterDomain;

/** `DROP DOMAIN`: one domain or several; `"restrict"`, said or not, refuses one a column is typed with. */
export declare function dropDomain(
  names: TypeRef | readonly [TypeRef, ...TypeRef[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

/**
 * `VALUE`: inside a domain's `CHECK`, the value tested. PostgreSQL reads it
 * anywhere else as a column named `value`.
 */
export declare function domainValue(): Expr;

export type SequenceType = "smallint" | "integer" | "bigint";

/** The clauses a sequence statement takes, creating the sequence or altering it. */
export interface SequenceClauses<S> {
  asType(type: SequenceType): S;
  /** At least one option, merged into those set; a later one for a clause replaces the earlier. */
  options(options: SequenceOptions): S;
  /** `OWNED BY` a table's column, which takes the sequence with it when dropped. */
  ownedBy(table: TableRef, column: string): S;
  /** `OWNED BY NONE`, which releases it. */
  ownedBy(table: null): S;
}

/** `CREATE SEQUENCE`; every method returns a new statement. */
export declare class CreateSequence extends DDL {
  ifNotExists(): CreateSequence;
}
export interface CreateSequence extends SequenceClauses<CreateSequence> {}

/**
 * An `ALTER SEQUENCE` before its first clause. PostgreSQL parses none
 * without one, so it has no `inspect()`, and a terminal refuses it.
 */
export declare class PendingAlterSequence {
  private constructor();
  /** `RESTART`, at `value` when one is given. */
  restart(value?: number | bigint): AlterSequence;
}
export interface PendingAlterSequence extends SequenceClauses<AlterSequence> {}

/** An `ALTER SEQUENCE` with at least one clause; every method returns a new one. */
export declare class AlterSequence extends DDL {
  ifExists(): AlterSequence;
  restart(value?: number | bigint): AlterSequence;
}
export interface AlterSequence extends SequenceClauses<AlterSequence> {}

export declare function createSequence(name: TableRef): CreateSequence;

export declare function alterSequence(name: TableRef): PendingAlterSequence;

export declare function dropSequence(
  names: TableRef | readonly [TableRef, ...TableRef[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

export declare function renameSequence(name: TableRef, newName: string): DDL;

export declare function createExtension(
  name: string,
  options?: {
    readonly ifNotExists?: boolean;
    readonly schema?: string;
    readonly version?: string;
    readonly cascade?: boolean;
  },
): DDL;

export declare function dropExtension(
  name: string,
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

// [spec:pgorm:req:bindings.schema-schemas]

/** `CREATE SCHEMA`: a schema is named by one identifier, never qualified. */
export declare function createSchema(
  name: string,
  options?: { readonly ifNotExists?: boolean },
): DDL;

/** `DROP SCHEMA`: one schema or several; `"restrict"`, said or not, refuses a schema that holds anything. */
export declare function dropSchema(
  names: string | readonly [string, ...string[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

/** `ALTER SCHEMA ... RENAME TO`, which PostgreSQL gives no `IF EXISTS`. */
export declare function renameSchema(name: string, newName: string): DDL;

// [spec:pgorm:req:bindings.schema-views+2]

/**
 * The options a view takes, plain or recursive: each written only when set,
 * the method giving a new statement.
 */
export interface ViewOptions<S> {
  /**
   * `OR REPLACE`: the new query keeps the view's columns in place, with
   * their names and types, and may add columns after them; the view's options
   * and check option are replaced by this statement's.
   */
  orReplace(): S;
  /** Names appended to the column list, renaming the query's leading columns. */
  columns(names: readonly string[]): S;
  /** The view's conditions run before any a query over it supplies. */
  securityBarrier(): S;
  /** The relations it reads are checked against the querying role. */
  securityInvoker(): S;
}

/**
 * `CREATE VIEW` over a {@link Select}, written with its values inlined: a
 * view's definition is SQL text and binds nothing.
 */
export declare class CreateView extends DDL {
  /**
   * `WITH LOCAL CHECK OPTION` or `WITH CASCADED CHECK OPTION`: a row written
   * through the view must be one it shows (`44000` otherwise). Only an
   * automatically updatable view takes one.
   */
  checkOption(option: "local" | "cascaded"): CreateView;
}
export interface CreateView extends ViewOptions<CreateView> {}

/**
 * `CREATE RECURSIVE VIEW`, whose query reads the view by name. It has no
 * check option, which PostgreSQL refuses on a recursive view.
 */
export declare class CreateRecursiveView extends DDL {}
export interface CreateRecursiveView extends ViewOptions<CreateRecursiveView> {}

/** The clauses a materialized view and `CREATE TABLE AS` share. */
export interface RelationAsOptions<S> {
  /** `IF NOT EXISTS`: an existing relation of the name is left alone, its rows with it. */
  ifNotExists(): S;
  /** Names appended to the column list, renaming the query's leading columns. */
  columns(names: readonly string[]): S;
  /** `WITH NO DATA`: the query's columns and none of its rows. */
  withNoData(): S;
}

/**
 * `CREATE MATERIALIZED VIEW`: the rows the query returned, held until a
 * refresh. One created `withNoData` cannot be read until it is refreshed.
 */
export declare class CreateMaterializedView extends DDL {}
export interface CreateMaterializedView extends RelationAsOptions<CreateMaterializedView> {}

/** `CREATE TABLE AS`: a table of the query's columns and rows, tied to it no further. */
export declare class CreateTableAs extends DDL {}
export interface CreateTableAs extends RelationAsOptions<CreateTableAs> {}

export declare function createView(name: TableRef, query: Select): CreateView;

/** A recursive view names at least one column, which its query's rows are called. */
export declare function createRecursiveView(
  name: TableRef,
  columns: string | readonly [string, ...string[]],
  query: Select,
): CreateRecursiveView;

export declare function dropView(
  names: TableRef | readonly [TableRef, ...TableRef[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

export declare function createMaterializedView(name: TableRef, query: Select): CreateMaterializedView;

/**
 * `REFRESH MATERIALIZED VIEW`: `concurrently` replaces the rows without
 * blocking reads, which needs a unique index on the view and a populated
 * view; `withNoData` empties it. PostgreSQL refuses the two together, and so
 * does this function.
 */
export declare function refreshMaterializedView(
  name: TableRef,
  options?: { readonly concurrently?: boolean; readonly withNoData?: boolean },
): DDL;

export declare function dropMaterializedView(
  names: TableRef | readonly [TableRef, ...TableRef[]],
  options?: { readonly ifExists?: boolean; readonly behavior?: DropBehavior },
): DDL;

export declare function createTableAs(name: TableRef, query: Select): CreateTableAs;
