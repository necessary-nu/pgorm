/**
 * Disjoint models: a base model whose every row has a kind, and a model for
 * each kind that carries data of its own, keyed by the base row's key.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.disjoint+2]
// [spec:pgorm:req:bindings.disjoint-unkinded]

import type { Compiled, OrderBy, Predicate } from "./expressions.d.ts";
import type { KeyOf, Model, RowOf, UpdateOf } from "./models.d.ts";
import type { ActiveModel, EntityRecord } from "./entities.d.ts";
import type { AlterTable, CreateTable, CreateType, DDL } from "./schema.d.ts";
import type { Table } from "./select.d.ts";
import type { OperationOptions, Queryable, RowStream } from "./index.d.ts";

/** Each kind's value, in order, to the model of its table or `null` for a kind with none. */
export type Kinds = { readonly [kind: string]: Model<any> | null };

/**
 * A row of a disjoint read over `B` and `K`: the base's record, the kind's
 * value, and the kind's record or `null` for a kind with no table.
 */
export type DisjointRow<B extends Model<any>, K extends Kinds> = {
  [V in keyof K & string]: [RowOf<B>, V, K[V] extends Model<any> ? RowOf<K[V]> : null];
}[keyof K & string];

/** A row of a registered disjoint enum's read: its entities' records. */
export type DisjointEntityRow = [EntityRecord, string, EntityRecord | null];

export interface DisjointOptions {
  /** The base's kind field, `"kind"` by default. */
  readonly kind?: string;
  /** The kind type's name, `{name}_kind` by default. */
  readonly enumName?: string;
  /** The kind type's schema, none by default; a model's enum column names its schema. */
  readonly schemaName?: string | null;
  readonly unkinded?: false;
}

/** An unkinded disjoint model's options: its base holds no kind, so nothing names one. */
export interface UnkindedDisjointOptions {
  readonly unkinded: true;
}

export interface DisjointDescription {
  readonly name: string;
  /** The base model's name. */
  readonly base: string;
  readonly kindField: string | null;
  readonly kindType: { readonly schema: string | null; readonly name: string | null };
  readonly kinds: readonly { readonly value: string; readonly model: string | null }[];
}

export interface DisjointInspectOptions {
  readonly terminal?: "all" | "one";
}

/**
 * A disjoint model over `B`, its kinds `K`. Made by {@link disjoint}; every
 * statement it builds is pgorm's `DisjointShape`'s.
 */
export declare class Disjoint<B extends Model<any> = Model<any>, K extends Kinds = Kinds> {
  private constructor();
  readonly name: string;
  readonly base: B;
  describe(): DisjointDescription;
  /** Every base row with its kind's row, in one statement. */
  find(): DisjointQuery<DisjointRow<B, K>, keyof K & string>;
  /** The base row of `key`, which gives every key field and no other. */
  findByKey(key: KeyOf<B>): DisjointQuery<DisjointRow<B, K>, keyof K & string>;
  /**
   * Insert a value of the kind `kind`: the base row from `base`, its kind
   * field holding `kind`, and for a kind with a table its row from `values`.
   */
  insert<V extends keyof K & string>(
    base: UpdateOf<B>,
    kind: V,
    values?: K[V] extends Model<any> ? UpdateOf<K[V]> : null,
  ): DisjointWrite<DisjointRow<B, K>>;
  /** Update a value of the kind `kind`, found by the key `base` holds. */
  update<V extends keyof K & string>(
    base: UpdateOf<B>,
    kind: V,
    values?: K[V] extends Model<any> ? UpdateOf<K[V]> : null,
  ): DisjointWrite<DisjointRow<B, K>>;
  /** Change the kind of the row of `key` to `kind`, its new row from `values`. */
  changeKind<V extends keyof K & string>(
    key: KeyOf<B>,
    kind: V,
    values?: K[V] extends Model<any> ? UpdateOf<K[V]> : null,
  ): DisjointWrite<DisjointRow<B, K>>;
}

/**
 * An unkinded disjoint model over `B`, its kinds `K`: a base with no kind
 * field, each row of the kind whose table has a row for it, `null` in `K`
 * standing for the kind of a row with none. It is read-only, so it has no
 * writes, and `createDisjoint` takes none.
 */
export interface UnkindedDisjoint<B extends Model<any> = Model<any>, K extends Kinds = Kinds> {
  readonly name: string;
  readonly base: B;
  describe(): DisjointDescription;
  /** Every base row with its kind's row, in one statement. */
  find(): DisjointQuery<DisjointRow<B, K>, keyof K & string>;
  /** The base row of `key`, which gives every key field and no other. */
  findByKey(key: KeyOf<B>): DisjointQuery<DisjointRow<B, K>, keyof K & string>;
}

/**
 * Declare a disjoint model `name` over `base`, `kinds` mapping each kind's
 * value, in order, to the model of its table or to `null`; `{ unkinded: true }`
 * declares an unkinded, read-only one. Declaring one sends nothing; what
 * pgorm's shape refuses is a `ConstructionError`.
 */
export declare function disjoint<B extends Model<any>, const K extends Kinds>(
  name: string,
  base: B,
  kinds: K,
  options: UnkindedDisjointOptions,
): UnkindedDisjoint<B, K>;
export declare function disjoint<B extends Model<any>, const K extends Kinds>(
  name: string,
  base: B,
  kinds: K,
  options?: DisjointOptions,
): Disjoint<B, K>;

/**
 * A downstream crate's `#[derive(Disjoint)]` enum the module registers, each
 * row an `R` and each kind a `V`, its writes taking the base's ActiveModel
 * `B` and a kind's `A`: a generated module types them from the
 * registration.
 */
export declare class DisjointEntity<
  R = DisjointEntityRow,
  V extends string = string,
  B extends ActiveModel<any> = ActiveModel<any>,
  A extends ActiveModel<any> = ActiveModel<any>,
> {
  private constructor();
  readonly name: string;
  describe(): DisjointDescription;
  find(): DisjointQuery<R, V>;
  /** The base row of `key`, which names every base key column by its SQL name, and nothing else. */
  findByKey(key: { readonly [column: string]: unknown }): DisjointQuery<R, V>;
  insert(base: B, kind: V, active?: A | null): DisjointWrite<R>;
  update(base: B, kind: V, active?: A | null): DisjointWrite<R>;
  changeKind(key: { readonly [column: string]: unknown }, kind: V, active?: A | null): DisjointWrite<R>;
}

/**
 * A downstream crate's `#[derive(Disjoint)]` enum declared `unkinded`, which
 * is read-only: `DisjointEntity` without its writes.
 */
export interface UnkindedDisjointEntity<R = DisjointEntityRow, V extends string = string> {
  readonly name: string;
  describe(): DisjointDescription;
  find(): DisjointQuery<R, V>;
  /** The base row of `key`, which names every base key column by its SQL name, and nothing else. */
  findByKey(key: { readonly [column: string]: unknown }): DisjointQuery<R, V>;
}

/**
 * The disjoint enum the module registers as `name`: an unkinded one refuses
 * every write at run time, and a generated module types it as an
 * `UnkindedDisjointEntity`.
 */
export declare function disjointEntity(name: string): DisjointEntity;

/** The names of the disjoint enums the module registers. */
export declare function disjointEntities(): string[];

/** A read of a disjoint model, each row an `R`, its kinds' values `V`. */
export declare class DisjointQuery<R, V extends string = string> {
  private constructor();
  where(predicate: Predicate): DisjointQuery<R, V>;
  orderBy(...orderings: OrderBy[]): DisjointQuery<R, V>;
  /** At most `count` rows; `null` reads every row. */
  limit(count: number | null): DisjointQuery<R, V>;
  /** The rows after the first `count`; `null` skips none. */
  offset(count: number | null): DisjointQuery<R, V>;
  /** Only base rows of the kind `kind`. */
  kind(kind: V): DisjointQuery<R, V>;
  /** Only base rows of one of `kinds`. */
  kinds(kinds: readonly V[]): DisjointQuery<R, V>;
  /** Only base rows of the kind `kind` whose row passes `predicate`. */
  filterKind(kind: V, predicate: Predicate): DisjointQuery<R, V>;
  inspect(options?: DisjointInspectOptions): Compiled;
  all(db: Queryable, options?: OperationOptions): Promise<R[]>;
  /** The first row; none is a `DecodeError`. */
  one(db: Queryable, options?: OperationOptions): Promise<R>;
  stream(db: Queryable, options?: OperationOptions): RowStream<R>;
  /** The base rows the read matches, counted as pgorm's paginator counts. */
  count(db: Queryable, options?: OperationOptions): Promise<number>;
  paginate(pageSize: number): DisjointPaginator<R>;
  /** A keyset cursor ordered by the base's `fields`. */
  cursor(...fields: string[]): DisjointCursor<R>;
}

/** An insert, update or change of kind: one statement, reading back the written row. */
export declare class DisjointWrite<R> {
  private constructor();
  inspect(): Compiled;
  /** Send the write; a write that touches no row is a `DecodeError`. */
  returningModel(db: Queryable, options?: OperationOptions): Promise<R>;
}

/** Pages of a disjoint read, numbered from zero. */
export declare class DisjointPaginator<R> {
  private constructor();
  inspect(page: number | "count"): Compiled;
  fetchPage(db: Queryable, page: number, options?: OperationOptions): Promise<R[]>;
  numItems(db: Queryable, options?: OperationOptions): Promise<number>;
  numPages(db: Queryable, options?: OperationOptions): Promise<number>;
  /** The rows the pages hold and the pages, as a model query's paginator gives them. */
  numItemsAndPages(db: Queryable, options?: OperationOptions): Promise<{ readonly items: number; readonly pages: number }>;
  pages(db: Queryable, options?: OperationOptions): AsyncGenerator<R[], void, undefined>;
}

/** A keyset cursor over a disjoint read; every method returns a new cursor. */
export declare class DisjointCursor<R> {
  private constructor();
  after(...values: unknown[]): DisjointCursor<R>;
  before(...values: unknown[]): DisjointCursor<R>;
  first(count: number): DisjointCursor<R>;
  last(count: number): DisjointCursor<R>;
  asc(): DisjointCursor<R>;
  desc(): DisjointCursor<R>;
  inspect(): Compiled;
  all(db: Queryable, options?: OperationOptions): Promise<R[]>;
}

/**
 * The statements creating a disjoint model's kind type, tables and the
 * constraints that hold each base row to exactly its kind's table, in the
 * order they run.
 */
export declare function createDisjoint(
  model: Disjoint<any, any> | DisjointEntity<any, any, any, any>,
): (CreateType | CreateTable | AlterTable)[];

// [spec:pgorm:req:bindings.disjoint-evolve]
/**
 * What the program writes between two steps of a {@link DisjointChange}: a
 * row in `table` for every base row of the kind `kind`, which the next step
 * refuses the change without.
 */
export interface DisjointGap {
  readonly kind: string;
  readonly table: Table;
}

/**
 * One step of a {@link DisjointChange}: statements to run in order in one
 * transaction, committed before the next step runs, and the rows the program
 * writes once it commits, if any.
 */
export interface DisjointStep {
  readonly statements: readonly (CreateType | CreateTable | AlterTable | DDL)[];
  readonly gap: DisjointGap | null;
}

/**
 * A change to a disjoint model's kinds, as the steps that make it, in the
 * order they run. A step that recreates the kind type changes what a
 * statement prepared before it returns, so a program running the steps
 * itself clears its pool's statement caches after each.
 */
export interface DisjointChange {
  readonly steps: readonly DisjointStep[];
}

/**
 * Statements frozen as the text of a migration written as a `.sql` file,
 * made by {@link migrationFile} and {@link migrationFiles} alone.
 */
// [spec:pgorm:req:bindings.migration-file]
export declare class MigrationFile {
  private constructor();
  /** The file's stem, and the migration's name. */
  readonly name: string;
  /** The file's text, which the migration runs whole. */
  readonly text: string;
  /** Whether the text opens with `-- pgorm: commits-after`. */
  readonly commitsAfter: boolean;
  /**
   * Write `<name>.sql` into `directory` if no file of that name is there,
   * answering whether it wrote one: a file already there is never rewritten.
   * A name that is no plain file stem is a `ConstructionError`, and any other
   * failure to write an `Error`.
   */
  freeze(directory: string | URL): boolean;
}

/**
 * `statements` frozen as the migration `name`, each ending `;` on a line of
 * its own, opening with `-- pgorm: commits-after` where `commitsAfter`.
 */
export declare function migrationFile(
  name: string,
  statements: readonly DDL[],
  options?: { readonly commitsAfter?: boolean },
): MigrationFile;

/**
 * A kind change a kind-change function returned, frozen as migration files
 * as pgorm freezes it: a file per step, named `name` for one step and
 * `<name>_<n>` from 1 otherwise, every step but the last opening with
 * `-- pgorm: commits-after`, and a step leaving rows to the program ending
 * with a comment saying which.
 */
export declare function migrationFiles(change: DisjointChange, name: string): MigrationFile[];

/**
 * Add the kind `value`, which the declaration holds and the database's kind
 * type does not yet: in two steps for a kind with a table, since PostgreSQL
 * uses an added value only once the transaction adding it has committed.
 */
export declare function addDisjointKind(
  model: Disjoint<any, any> | DisjointEntity<any, any, any, any>,
  value: string,
): DisjointChange;

/** Add the kind `value` in one step that recreates the kind type and rewrites the base and every kind table. */
export declare function addDisjointKindRecreating(
  model: Disjoint<any, any> | DisjointEntity<any, any, any, any>,
  value: string,
): DisjointChange;

/**
 * Remove the value `value`, which the database's kind type holds and the
 * declaration does not, in one step that recreates the kind type and fails
 * while any base row has the kind.
 */
export declare function removeDisjointKind(
  model: Disjoint<any, any> | DisjointEntity<any, any, any, any>,
  value: string,
): DisjointChange;

/**
 * Give the kind `value`, a kind without a table in the database, the table the
 * declaration gives it, in two steps with a gap between them for the rows its
 * base rows lack.
 */
export declare function addDisjointKindTable(
  model: Disjoint<any, any> | DisjointEntity<any, any, any, any>,
  value: string,
): DisjointChange;

/**
 * Drop `table`, the table of the kind `value`, which the declaration gives
 * none, and the base's generated key columns and foreign key for it, in one
 * step that loses every row of the table.
 */
export declare function removeDisjointKindTable(
  model: Disjoint<any, any> | DisjointEntity<any, any, any, any>,
  value: string,
  table: string | Table,
): DisjointChange;
