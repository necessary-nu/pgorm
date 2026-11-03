/**
 * The Rust entities and graph shapes an application's own native module
 * registers, reached by name.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.entities+4]

import type { Compiled, ComparisonOperand, Expr, Operand, OrderBy, Predicate } from "./expressions.d.ts";
import type { Conflict, ConflictUpdate } from "./writes.d.ts";
import type { Change, Upserted } from "./models.d.ts";
import type { OperationOptions, PlainValue, Queryable, Value } from "./index.d.ts";

/** A registered entity's record when no generated module types it: its values by SQL column name. */
export type EntityRecord = { readonly [column: string]: PlainValue };

/**
 * What a registration's types are: the record its reads and writes give, and
 * what each column takes as a value, by SQL column name.
 */
export interface EntityTypes<R extends object = EntityRecord, I extends object = { readonly [column: string]: unknown }> {
  readonly record: R;
  readonly input: I;
}

/**
 * Each registered entity's {@link EntityTypes}, by registration name. Empty
 * in the binding; a declaration file generated for an application's
 * entities augments it, so `entity(name)` types its records and refuses a
 * name the application does not register.
 */
// deno-lint-ignore no-empty-interface
export interface Registrations {}

/**
 * Each registered graph's row, by registration name: the root's record
 * alone, or a tuple of every source's, an `Opt` slot's `| null`. Augmented
 * as {@link Registrations} is.
 */
// deno-lint-ignore no-empty-interface
export interface GraphRegistrations {}

/**
 * Each registered source tuple's row, by registration name: a tuple of each
 * source's record or `null`. Augmented as {@link Registrations} is.
 */
// deno-lint-ignore no-empty-interface
export interface SourceRegistrations {}

/** The names `entity` takes: any string, until a generated module lists them. */
export type EntityName = [keyof Registrations] extends [never] ? string : keyof Registrations & string;

/** The names `graph` takes: any string, until a generated module lists them. */
export type GraphName = [keyof GraphRegistrations] extends [never] ? string : keyof GraphRegistrations & string;

/** The {@link EntityTypes} of the entity registered as `N`. */
export type TypesOf<N> = N extends keyof Registrations ? Registrations[N] extends EntityTypes<any, any> ? Registrations[N]
  : EntityTypes
  : EntityTypes;

/** The row of the graph registered as `N`. */
export type GraphRowOf<N> = N extends keyof GraphRegistrations ? GraphRegistrations[N]
  : EntityRecord | (EntityRecord | null)[];

/** The record of a registration's types. */
export type EntityRecordOf<T extends EntityTypes<any, any>> = T["record"];
/** A column of a registration's types. */
export type EntityColumnName<T extends EntityTypes<any, any>> = keyof T["record"] & string;
/** What a column of a registration's types takes. */
export type EntityInputOf<T extends EntityTypes<any, any>, C> = C extends keyof T["input"] ? T["input"][C] : unknown;

/** The names of the entities this module registers. */
export declare function entities(): string[];

/**
 * The entity this module registers as `name`; a name it does not register is
 * a {@link ConstructionError}.
 */
export declare function entity<N extends EntityName>(name: N): Entity<TypesOf<N>>;

declare const operands: unique symbol;

/**
 * A registered entity's column, its comparisons the column's own `ColumnTrait`
 * methods, each taking `I` ({@link ComparisonOperand}).
 *
 * `eq`, `ne`, `gt`, `gte`, `lt` and `lte` compare with a value of the
 * column's declared kind, a range or multirange column's `Range` or
 * `Multirange` included, written through its `save_as`, or with an
 * expression, `any(..)` or `all(..)` as it stands. `isIn` and `isNotIn` test
 * membership in a list of values, each read as the column's declared kind as
 * a compared value is, or in a subquery's rows, by the column's `ColumnTrait`:
 * a column whose domain is over an enum is a member as the enum. A list
 * holding an expression is a member as written.
 */
export declare class EntityColumn<I = Operand> extends Expr {
  #private;
  /** The column's SQL name. */
  readonly name: string;
  /** The values its comparisons take, for the types alone. */
  readonly [operands]?: I;
}

/** What a registered entity's column is compared with: a value of its input type, never `null`. */
export type ColumnOperand<T extends EntityTypes<any, any>, C> = unknown extends EntityInputOf<T, C> ? Operand
  : Exclude<EntityInputOf<T, C>, null> | Value | Expr;

/** A registration's description: its table, columns, key, relations and Rust types. */
export interface EntityDescription {
  readonly name: string;
  readonly schema: string | null;
  readonly table: string;
  readonly columns: readonly {
    readonly name: string;
    /** The key the entity's Rust field serializes under. */
    readonly jsonKey: string;
    readonly sqlType: string;
    readonly rustType: string | null;
    readonly nullable: boolean;
    readonly primaryKey: boolean;
    /** The kind its values bind and read as, as a result spells it; `null` for a type that takes a `Value`. */
    readonly kind: string | null;
    /** An enum column's labels, in declaration order; `null` for any other. */
    readonly values: readonly string[] | null;
  }[];
  readonly primaryKey: readonly string[];
  /** Whether the key's last column is a period matched `WITHOUT OVERLAPS`. */
  readonly primaryKeyWithoutOverlaps: boolean;
  readonly relations: readonly {
    readonly name: string;
    readonly type: "hasOne" | "hasMany";
    readonly from: { readonly schema: string | null; readonly table: string };
    readonly to: { readonly schema: string | null; readonly table: string };
    readonly columns: readonly (readonly [string, string])[];
    readonly period: readonly [string, string] | null;
    /** Whether a join by the relation needs every column pair to match (`all`) or any one of them (`any`). */
    readonly conditionType: "all" | "any";
    /** Whether an `on_condition` adds a condition of its own: a Rust closure, reported as present or absent only. */
    readonly onCondition: boolean;
    readonly enforcement: "enforced" | "notEnforced" | null;
    readonly deferrability: "notDeferrable" | "deferrableInitiallyImmediate" | "deferrableInitiallyDeferred" | null;
  }[];
  readonly rust: { readonly entity: string; readonly model: string; readonly activeModel: string; readonly column: string };
}

/**
 * A registered entity: its `Select<E>`, its ActiveModels, and the writes
 * that return a row's two versions. Its records are frozen plain objects
 * keyed by SQL column name, behind which the module keeps the Rust model.
 */
export declare class Entity<T extends EntityTypes<any, any> = EntityTypes> {
  private constructor();
  /** The name it is registered under. */
  readonly name: string;
  /** The SQL column names its records are keyed by, in column order. */
  readonly columns: readonly EntityColumnName<T>[];
  describe(): EntityDescription;
  /** A column as its `ColumnTrait` names it. */
  col<C extends EntityColumnName<T>>(column: C): EntityColumn<ColumnOperand<T, C>>;
  /** `E::find()`. */
  find(): EntityQuery<EntityRecordOf<T>>;
  /** An ActiveModel from `ActiveModelBehavior::new`, its defaults included. */
  active(): ActiveModel<T>;
  /** The record's ActiveModel by the real `IntoActiveModel`, every column `unchanged`. */
  intoActive(record: EntityRecordOf<T>): ActiveModel<T>;
  /** A copy of the record with `ModelTrait::set` applied; nothing is written. */
  withValue<C extends EntityColumnName<T>>(record: EntityRecordOf<T>, column: C, value: EntityInputOf<T, C> | Value): EntityRecordOf<T>;
  /** One column of a record as a `Value`, its kind as the entity declares it. */
  tagged(record: EntityRecordOf<T>, column: EntityColumnName<T>): Value;
  /** An update of the ActiveModel's row by its key. */
  update(active: ActiveModel<T>): EntityUpdate<EntityRecordOf<T>>;
  /** An update of every row its condition admits. */
  updateMany(): EntityUpdateMany<T>;
  /** An insert of one ActiveModel. */
  insert(active: ActiveModel<T>): EntityInsert<EntityRecordOf<T>>;
  /** An insert of ActiveModels; a batch of none writes nothing. */
  insertMany(actives: readonly ActiveModel<T>[]): EntityInsert<EntityRecordOf<T>>;
}

/** A registered entity's `Select<E>`. Every method returns a new query. */
export declare class EntityQuery<R = EntityRecord> {
  private constructor();
  /** Rows that also satisfy `predicate`, by `QueryFilter::filter`. */
  where(predicate: Predicate): EntityQuery<R>;
  orderBy(...orderings: OrderBy[]): EntityQuery<R>;
  /** `null` removes it. */
  limit(count: number | null): EntityQuery<R>;
  offset(count: number | null): EntityQuery<R>;
  /** The SQL and values a terminal sends, `one` and `optional` with the limit Rust narrows to one row. */
  inspect(terminal?: "all" | "one" | "optional"): Compiled;
  /** Every row, by `Select::all`. */
  all(db: Queryable, options?: OperationOptions): Promise<R[]>;
  /** The first row, by `Select::one`; none is a {@link DecodeError}. */
  one(db: Queryable, options?: OperationOptions): Promise<R>;
  /** The first row, or `null`, by `Select::one_opt`. */
  optional(db: Queryable, options?: OperationOptions): Promise<R | null>;
}

/** An ActiveModel column's state, and its value unless it is `notSet`. */
export type ActiveState<V = PlainValue> =
  | { readonly state: "notSet" }
  | { readonly state: "set" | "unchanged"; readonly value: V };

/**
 * A registered entity's ActiveModel. Every change returns a new one; its
 * writes are `ActiveModelTrait`'s, the application's `ActiveModelBehavior`
 * hooks around them, a hook's refusal a {@link ConstructionError}.
 */
export declare class ActiveModel<T extends EntityTypes<any, any> = EntityTypes> {
  private constructor();
  /** The name of the entity it is an ActiveModel of. */
  readonly entityName: string;
  get<C extends EntityColumnName<T>>(column: C): ActiveState<EntityRecordOf<T>[C]>;
  /** The column `set` to a value converted to its declared kind. */
  set<C extends EntityColumnName<T>>(column: C, value: EntityInputOf<T, C> | Value): ActiveModel<T>;
  notSet(column: EntityColumnName<T>): ActiveModel<T>;
  /** The column back to `unchanged`, if it held a value. */
  reset(column: EntityColumnName<T>): ActiveModel<T>;
  /** `ActiveModelTrait::insert`: the inserted record. */
  insert(db: Queryable, options?: OperationOptions): Promise<EntityRecordOf<T>>;
  /** `ActiveModelTrait::update`: the updated record. */
  update(db: Queryable, options?: OperationOptions): Promise<EntityRecordOf<T>>;
  /** `ActiveModelTrait::delete`: the rows deleted. */
  delete(db: Queryable, options?: OperationOptions): Promise<number>;
}

/** An update of one ActiveModel's row by its key. */
export declare class EntityUpdate<R = EntityRecord> {
  private constructor();
  /** The row before and after, by `UpdateOne::exec_returning_change`, no hook running. */
  returningChange(db: Queryable, options?: OperationOptions): Promise<Change<R>>;
}

/** An update of every row a condition admits, which needs `where` or `allRows()` before it runs. */
export declare class EntityUpdateMany<T extends EntityTypes<any, any> = EntityTypes> {
  private constructor();
  /** The column set to a value through its `save_as`, or to an expression as written. */
  set<C extends EntityColumnName<T>>(column: C, value: EntityInputOf<T, C> | Value | Expr): EntityUpdateMany<T>;
  where(predicate: Predicate): EntityUpdateMany<T>;
  allRows(): EntityUpdateMany<T>;
  /** Each written row before and after, by `UpdateMany::exec_returning_changes`. */
  returningChanges(db: Queryable, options?: OperationOptions): Promise<Change<EntityRecordOf<T>>[]>;
}

/** An insert of ActiveModels, with the conflict clause it takes. */
export declare class EntityInsert<R = EntityRecord> {
  private constructor();
  onConflict(action: Conflict | ConflictUpdate): EntityInsert<R>;
  /** What the insert of one row did, or `null` where its conflict clause wrote nothing. */
  returningUpsert(db: Queryable, options?: OperationOptions): Promise<Upserted<R> | null>;
  /** What the insert did with each row it wrote. */
  returningUpserts(db: Queryable, options?: OperationOptions): Promise<Upserted<R>[]>;
}

/** The names of the graph shapes this module registers. */
export declare function graphs(): string[];

/** The graph shape this module registers as `name`. */
export declare function graph<N extends GraphName>(name: N): EntityGraph<GraphRowOf<N>>;

/** A registered graph's description: its Rust shape, and each source's entity and slot kind. */
export interface GraphDescription {
  readonly name: string;
  readonly rustShape: string;
  readonly sources: readonly { readonly index: number; readonly entity: string; readonly slot: "root" | "Req" | "Opt" }[];
}

/** A registered `SelectGraph` shape. */
export declare class EntityGraph<Row = EntityRecord | (EntityRecord | null)[]> {
  private constructor();
  readonly name: string;
  describe(): GraphDescription;
  /** The graph built by its factory, each slot under the alias given for it, `g1`, `g2`, .. by default. */
  find(options?: { readonly aliases?: readonly string[] }): EntityGraphQuery<Row>;
}

/** A query over a registered graph. Every method returns a new query. */
export declare class EntityGraphQuery<Row = EntityRecord | (EntityRecord | null)[]> {
  private constructor();
  readonly aliases: readonly string[];
  where(predicate: Predicate): EntityGraphQuery<Row>;
  orderBy(...orderings: OrderBy[]): EntityGraphQuery<Row>;
  /** A column of decoded source `source` — 0 the root, i the i-th slot — qualified as the query names it. */
  col(source: number, column: string): Expr;
  inspect(terminal?: "all" | "optional"): Compiled;
  all(db: Queryable, options?: OperationOptions): Promise<Row[]>;
  /** The first row or `null`: no row, apart from a row whose optional slot is absent. */
  optional(db: Queryable, options?: OperationOptions): Promise<Row | null>;
  /** A keyset cursor ordered by a root column, every source's primary key after it. */
  cursor(column: string): EntityGraphCursor<Row>;
}

/** A keyset cursor over a registered graph. Every method returns a new cursor. */
export declare class EntityGraphCursor<Row = EntityRecord | (EntityRecord | null)[]> {
  private constructor();
  before(value: unknown): EntityGraphCursor<Row>;
  after(value: unknown): EntityGraphCursor<Row>;
  /** Short of the row whose whole key this is: the order column, the root's other key, then each slot's. */
  beforeWith(...values: unknown[]): EntityGraphCursor<Row>;
  afterWith(...values: unknown[]): EntityGraphCursor<Row>;
  first(rows: number): EntityGraphCursor<Row>;
  last(rows: number): EntityGraphCursor<Row>;
  asc(): EntityGraphCursor<Row>;
  desc(): EntityGraphCursor<Row>;
  all(db: Queryable, options?: OperationOptions): Promise<Row[]>;
}

export {};
