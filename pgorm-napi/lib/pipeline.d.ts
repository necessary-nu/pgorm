/**
 * pgorm's PRQL-shaped pipeline: relation-to-relation stages over sources,
 * compiled through prqlc to PostgreSQL SQL and run as any statement is.
 *
 * Values reach the SQL by one of two routes, and the spelling says which:
 * {@link literal} writes one into the SQL; any other value an operand is
 * given is bound as a parameter by the stage that takes it. A stage's `With`
 * form calls its function once with a {@link Binder}, whose placeholders
 * belong to that stage alone: one used anywhere else is a `LifecycleError`.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.pipeline+1]
// [spec:pgorm:req:bindings.pipeline-terminals]

import type { Builder, BindValue, Compiled, Expr } from "./expressions.d.ts";
import type { Table } from "./select.d.ts";
import type { Entity, EntityColumn, EntityRecord, SourceRegistrations } from "./entities.d.ts";
import type { OperationOptions, Queryable, QueryOptions, RowFor, TypeName } from "./index.d.ts";
import type { Columns, FieldOf, FieldOperand, Model, Row, RowOf } from "./models.d.ts";

/** An operand: a pipeline expression, or a value the stage that takes it binds. */
export type PipelineOperand = PipelineExpr | BindValue;

/**
 * A relation a pipeline reads: a table, read under its alias when it has one,
 * a table's name, a pipeline, a named source, a declared model's table, read
 * under the model's alias when it has one, or a registered entity's table,
 * its schema from the registration.
 */
export type SourceInput = Table | string | Pipeline | Source | Model<any> | Entity<any>;

/** A name a stage introduces or a column has: an identifier, or an {@link alias}. */
export type PipelineName = string | PipelineExpr;

/** The types {@link PipelineExpr.cast} casts to: pgorm's closed set, its names reaching the SQL as written. */
export type CastType =
  | "smallint"
  | "integer"
  | "bigint"
  | "real"
  | "double"
  | "float8"
  | "numeric"
  | "text"
  | "boolean"
  | "date"
  | "timestamp"
  | "timestamptz"
  | "interval"
  | "uuid"
  | "json"
  | "jsonb";

/**
 * A pipeline expression; every method returns a new one. It is not the
 * statement builders' `Expr`, and the two do not mix.
 */
export declare class PipelineExpr {
  private constructor();
  eq(other: PipelineOperand): PipelineExpr;
  ne(other: PipelineOperand): PipelineExpr;
  gt(other: PipelineOperand): PipelineExpr;
  gte(other: PipelineOperand): PipelineExpr;
  lt(other: PipelineOperand): PipelineExpr;
  lte(other: PipelineOperand): PipelineExpr;
  and(other: PipelineOperand): PipelineExpr;
  or(other: PipelineOperand): PipelineExpr;
  coalesce(other: PipelineOperand): PipelineExpr;
  add(other: PipelineOperand): PipelineExpr;
  sub(other: PipelineOperand): PipelineExpr;
  mul(other: PipelineOperand): PipelineExpr;
  /** Division as a float, as PRQL's `/` is. */
  div(other: PipelineOperand): PipelineExpr;
  rem(other: PipelineOperand): PipelineExpr;
  not(): PipelineExpr;
  neg(): PipelineExpr;
  isNull(): PipelineExpr;
  isNotNull(): PipelineExpr;
  /** A sort key in ascending order, the default. */
  asc(): PipelineExpr;
  desc(): PipelineExpr;
  /** Membership in an explicit list, each item an operand. */
  inArray(values: readonly PipelineOperand[]): PipelineExpr;
  cast(type: CastType): PipelineExpr;
  /**
   * Compared as the type `type` names, as a registered column of that type
   * compares: every comparison, `inArray`, `coalesce`, `min` and `max` cast
   * it to the type, and each value a comparison, `inArray` or `coalesce` sets
   * beside it is cast too, which PostgreSQL needs of a column whose domain
   * is over an enum, or of a `money` column compared with a `Decimal`, where
   * no registration types it.
   */
  comparesAs(type: string | TypeName): PipelineExpr;
  /** The name a projected expression takes: an identifier, or an {@link alias} a later stage reads it by. */
  as(name: PipelineName): PipelineExpr;
}

/**
 * A declared model's column in a pipeline. A value its comparisons,
 * `coalesce` and `inArray` take converts through the field's declaration,
 * `I`, as the model column's own comparisons convert one, and binds cast as
 * an entity column of its kind casts one: an enum's label to its enum. A
 * pipeline expression is taken as written; `null` is a `ConstructionError`,
 * since a comparison with it is never true. What an operator makes of the
 * column is a plain {@link PipelineExpr}.
 */
export interface PipelineColumn<I extends BindValue = BindValue> extends PipelineExpr {
  eq(other: I | PipelineExpr): PipelineExpr;
  ne(other: I | PipelineExpr): PipelineExpr;
  gt(other: I | PipelineExpr): PipelineExpr;
  gte(other: I | PipelineExpr): PipelineExpr;
  lt(other: I | PipelineExpr): PipelineExpr;
  lte(other: I | PipelineExpr): PipelineExpr;
  coalesce(other: I | PipelineExpr): PipelineExpr;
  inArray(values: readonly (I | PipelineExpr)[]): PipelineExpr;
}

/** What field `F` of columns `C` is compared with in a pipeline: a value of its kind, never a statement builder's expression. */
export type PipelineFieldOperand<C extends Columns, F extends keyof C> = Exclude<FieldOperand<C[F]>, Expr>;

/**
 * Field `field` of a declared model: the column its declaration names,
 * qualified by the model's alias or its table's name, compared as its
 * declared kind.
 */
export declare function col<C extends Columns, F extends FieldOf<C>>(
  model: Model<C>,
  field: F,
): PipelineColumn<PipelineFieldOperand<C, F>>;
/**
 * A column of the relation `table` names: prqlc has no catalog, so every
 * column is qualified. A registered entity's column is the entity's own,
 * compared as the entity types it; a name only qualifies.
 */
export declare function col(table: PipelineName, column: PipelineName | EntityColumn<any>): PipelineExpr;
/** A name a stage introduces, which reads back unqualified wherever it is used. */
export declare function alias(name: string): PipelineExpr;
/**
 * A value written into the SQL as pgorm's pipeline writes a literal: `null`,
 * a boolean, an integer within `bigint`, a finite number or a string.
 */
export declare function literal(value: null | boolean | number | bigint | string): PipelineExpr;
/** The column of the pipeline built so far, in a join condition: PRQL's `this`. */
export declare function thisColumn(column: PipelineName): PipelineExpr;
/** The column of the relation joined, in a join condition: PRQL's `that`. */
export declare function thatColumn(column: PipelineName): PipelineExpr;
/** `CASE`: the first arm whose condition holds, else `otherwise`, which pgorm's pipeline requires. */
export declare function caseWhen(
  arms: readonly (readonly [PipelineOperand, PipelineOperand])[],
  otherwise: PipelineOperand,
): PipelineExpr;
/** `SUM`, which PRQL writes as `COALESCE(SUM(x), 0)`. */
export declare function sum(value: PipelineOperand): PipelineExpr;
export declare function min(value: PipelineOperand): PipelineExpr;
export declare function max(value: PipelineOperand): PipelineExpr;
export declare function average(value: PipelineOperand): PipelineExpr;
export declare function stddev(value: PipelineOperand): PipelineExpr;
export declare function count(value: PipelineOperand): PipelineExpr;
export declare function countDistinct(value: PipelineOperand): PipelineExpr;
/** `COUNT(*)`. */
export declare function countRows(): PipelineExpr;
export declare function rowNumber(): PipelineExpr;
/** `RANK()`, over the column it ranks, as PRQL's signature takes it. */
export declare function rank(value: PipelineOperand): PipelineExpr;
export declare function rankDense(value: PipelineOperand): PipelineExpr;
export declare function first(value: PipelineOperand): PipelineExpr;
export declare function last(value: PipelineOperand): PipelineExpr;
export declare function lag(offset: number, value: PipelineOperand): PipelineExpr;
export declare function lead(offset: number, value: PipelineOperand): PipelineExpr;

/**
 * Mints one stage's placeholders. It binds only while its `With` function
 * runs; a placeholder it mints belongs to the expressions that function
 * returns to its stage, and is a `LifecycleError` anywhere else.
 */
export declare class Binder {
  private constructor();
  /** One placeholder for one value, reusable within the stage. */
  bind(value: BindValue): PipelineExpr;
}

/** What a `With` form's function returns: one expression for a filter or join condition. */
export type BinderScalar = (binder: Binder) => PipelineOperand;
/** What a list stage's `With` function returns: an expression, or an array of at most 32. */
export type BinderList = (binder: Binder) => PipelineExpr | readonly PipelineOperand[];

/** A pipeline grouped and not yet aggregated, which no terminal runs. */
export declare class Grouped {
  private constructor();
  /** The groups' keys, then these aggregates. */
  aggregate(...aggregates: PipelineOperand[]): Pipeline;
  aggregateWith(fn: BinderList): Pipeline;
}

/** A relation read under a name of its own, which is how a relation meets itself. */
export declare class Source {
  private constructor();
  named(name: string): Source;
}

/** A window: partition, ordering and frame. Its keys take no value, bound or not. */
export declare class Over {
  private constructor();
  /** `PARTITION BY` these keys. */
  by(...keys: PipelineExpr[]): Over;
  /** `ORDER BY` these keys within the window, which without a partition also orders the output. */
  sortBy(...keys: PipelineExpr[]): Over;
  /** A `ROWS` frame relative to the current row: 0 is the row, negative precedes, `null` is unbounded. */
  rows(start: number | bigint | null, end: number | bigint | null): Over;
  /** A `RANGE` frame, its bounds read as `rows` reads them. */
  range(start: number | bigint | null, end: number | bigint | null): Over;
}

export type JoinKind = "inner" | "left" | "right" | "full";

/**
 * A pipeline: a relation built stage by stage; every stage returns a new one.
 * Its own terminals are Rust's `Pipeline::all`, `one` and `one_opt`; it also
 * runs through `execute`, `query`, `one`, `optional` and `stream` as any
 * statement does, with their cardinality. What prqlc cannot compile is a
 * `ConstructionError` there or from `inspect()`.
 */
export declare class Pipeline extends Builder {
  private constructor();
  /** The SQL and values a terminal sends, `one` and `optional` with the `take 1` Rust appends. */
  inspect(terminal?: "all" | "one" | "optional"): Compiled;
  /** Every row, by `Pipeline::all`. */
  all<O extends QueryOptions = QueryOptions>(db: Queryable, options?: O): Promise<RowFor<O>[]>;
  /** The first row, by `Pipeline::one`, which appends `take 1`; none is a `DecodeError`. */
  one<O extends QueryOptions = QueryOptions>(db: Queryable, options?: O): Promise<RowFor<O>>;
  /** The first row, or `null`, by `Pipeline::one_opt`, which appends `take 1`. */
  optional<O extends QueryOptions = QueryOptions>(db: Queryable, options?: O): Promise<RowFor<O> | null>;
  /** Rows the predicate holds for: a `WHERE`, or a `HAVING` after an aggregate. */
  filter(predicate: PipelineOperand): Pipeline;
  filterWith(fn: BinderScalar): Pipeline;
  /** Computed columns beside the existing ones. */
  derive(...columns: PipelineOperand[]): Pipeline;
  deriveWith(fn: BinderList): Pipeline;
  /** The columns replaced by these. */
  select(...columns: PipelineOperand[]): Pipeline;
  selectWith(fn: BinderList): Pipeline;
  sort(...keys: PipelineOperand[]): Pipeline;
  sortWith(fn: BinderList): Pipeline;
  /** A grouping by these keys, a relation again only once aggregated. */
  group(...keys: PipelineOperand[]): Grouped;
  groupWith(fn: BinderList): Grouped;
  window(over: Over, ...columns: PipelineOperand[]): Pipeline;
  windowWith(over: Over, fn: BinderList): Pipeline;
  /** At most `count` rows: an integer, never an expression, as PRQL takes it. */
  take(count: number | bigint): Pipeline;
  /** Rows `start` through `end`, counted from 1, both included. */
  takeRange(start: number | bigint, end: number | bigint): Pipeline;
  join(source: SourceInput, on: PipelineOperand, options?: { readonly kind?: JoinKind }): Pipeline;
  joinWith(source: SourceInput, fn: BinderScalar, options?: { readonly kind?: JoinKind }): Pipeline;
  /** `UNION ALL` with another relation of the same columns. */
  append(source: SourceInput): Pipeline;
  intersect(source: SourceInput): Pipeline;
  /** The rows the other relation does not have: `EXCEPT`. */
  remove(source: SourceInput): Pipeline;
  distinct(): Pipeline;
  /**
   * The last stage: a registered source tuple's sources, each read under the
   * qualifier given for it or its table's name, decoded into their models
   * by the terminals of what this returns.
   */
  selectSources<Row extends readonly unknown[]>(
    selection: SourceSelection<Row>,
    options?: { readonly qualifiers?: readonly string[] },
  ): SelectedSources<Row>;
  /**
   * The last stage: declared models, each restating a relation the pipeline
   * read — a model for one read under its table's name, an aliased model for
   * one read under that name — every row a tuple of one record per model,
   * `null` where a join left that model's side absent. A pipeline reshaped
   * before the selection is a `ConstructionError`.
   */
  selectSources<const M extends readonly [Model<any>, ...Model<any>[]]>(
    ...models: M
  ): PipelineSelector<{ -readonly [K in keyof M]: RowOf<M[K]> | null }>;
  /** The rows read as a declared model's records, each field from the result column of its SQL name, as Rust's `into_model::<M>` reads them. */
  intoModel<C extends Columns>(model: Model<C>): PipelineSelector<Row<C>>;
  /** The rows read as what `fn` makes of each row's record. */
  intoModel<T>(fn: (row: Record<string, unknown>) => T): PipelineSelector<T>;
  /** The rows read by position, each an array of its values in column order, as Rust's `into_tuple` reads them. */
  intoTuple(): PipelineSelector<unknown[]>;
}

/**
 * A pipeline's rows staged for one shape of read, by `selectSources` over
 * models, `intoModel` or `intoTuple`. `one` and `optional` take one row
 * before the read's projection, as the pipeline's own do.
 */
export declare class PipelineSelector<R> {
  private constructor();
  inspect(terminal?: "all" | "one" | "optional"): Compiled;
  all(db: Queryable, options?: OperationOptions): Promise<R[]>;
  /** Exactly one row; none is a `DecodeError`. */
  one(db: Queryable, options?: OperationOptions): Promise<R>;
  optional(db: Queryable, options?: OperationOptions): Promise<R | null>;
}

/** The names `sources` takes: any string, until a generated module lists them. */
export type SourceTupleName = [keyof SourceRegistrations] extends [never] ? string : keyof SourceRegistrations & string;

/** The row of the source tuple registered as `N`: a tuple of each source's record or `null`. */
export type SourceRowOf<N> = N extends keyof SourceRegistrations ? SourceRegistrations[N] : (EntityRecord | null)[];

/** The names of the source tuples this module registers. */
export declare function sourceTuples(): string[];

/** The source tuple this module registers as `name`; another name is a `ConstructionError`. */
export declare function sources<N extends SourceTupleName>(name: N): SourceSelection<SourceRowOf<N>>;

/** A registered tuple of one to six entity types a pipeline's rows decode as. */
export declare class SourceSelection<Row extends readonly unknown[] = (EntityRecord | null)[]> {
  private constructor();
  readonly name: string;
  describe(): { readonly name: string; readonly rustShape: string; readonly entities: readonly string[] };
  /** The row type, for the types alone. */
  private readonly row?: Row;
}

/**
 * A pipeline whose last stage selects a registered tuple's sources: each row
 * a tuple of the sources' records, a source the row does not carry `null`.
 * A pipeline reshaped before the selection is a `ConstructionError`.
 */
export declare class SelectedSources<Row extends readonly unknown[] = (EntityRecord | null)[]> {
  private constructor();
  inspect(terminal?: "all" | "one" | "optional"): Compiled;
  all(db: Queryable, options?: OperationOptions): Promise<Row[]>;
  /** Exactly one row; none is a `DecodeError`. */
  one(db: Queryable, options?: OperationOptions): Promise<Row>;
  optional(db: Queryable, options?: OperationOptions): Promise<Row | null>;
}

/** A pipeline reading `source`; a pipeline source is embedded whole, its bound values with it. */
export declare function from(source: SourceInput): Pipeline;
/** A relation as a source, to read under a name of its own with `named`. */
export declare function source(relation: SourceInput): Source;
/** A window over the whole relation, until `by` partitions it. */
export declare function over(): Over;
