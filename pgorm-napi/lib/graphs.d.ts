/**
 * Graphs over models, keyset cursors and pages.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.graphs]

import type { Compiled, OrderBy, Predicate } from "./expressions.d.ts";
import type { Select } from "./select.d.ts";
import type { Connection, OperationOptions, Pool, Queryable, RowStream } from "./index.d.ts";
import type { ColumnOf, Columns, ColumnsOf, FieldOf, FieldOperand, Model, Relation, RowOf } from "./models.d.ts";

/** What a cursor's boundary gives for each of its order fields `F` of columns `C`. */
export type KeyInputs<C extends Columns, F extends readonly FieldOf<C>[]> = {
  [I in keyof F]: Exclude<FieldOperand<C[F[I]]>, import("./expressions.d.ts").Expr>;
};

/** What one row of a graph decodes as: the root's record alone, or a tuple of every source's. */
export type GraphItem<I extends readonly unknown[]> = I extends readonly [infer Only] ? Only : I;

type Indexes<S extends readonly unknown[]> = { [K in keyof S]: K extends `${infer N extends number}` ? N : never }[number];

/** How a graph joins a relation's far end. */
export interface GraphJoinOptions {
  /** The name the joined table answers to; a second table of one name needs one. */
  readonly alias?: string;
  /**
   * The decoded source the relation starts at — 0 the root, i the i-th slot
   * — when it is not the first table of its model the graph reads.
   */
  readonly from?: number;
}

/**
 * A model's rows with the rows its relations reach, as pgorm's `SelectGraph`
 * reads them: `S` the decoded sources' models and `I` the records each
 * contributes. The slot kind is the join type is the decode shape:
 * `joinOne` an INNER JOIN decoded as a record, `joinMaybe` a LEFT JOIN
 * decoded as a record or `null` where it matched nothing, and `via` a LEFT
 * JOIN of a table no record reads. A present slot whose record does not
 * decode is a {@link DecodeError}, never an absent one.
 */
export declare class Graph<S extends readonly Model<any>[], I extends readonly unknown[]> {
  private constructor();
  /** A slot that must match. */
  joinOne<T extends Model<any>>(relation: Relation<any, T, any>, options?: GraphJoinOptions): Graph<[...S, T], [...I, RowOf<T>]>;
  /** A slot that may be absent. */
  joinMaybe<T extends Model<any>>(
    relation: Relation<any, T, any>,
    options?: GraphJoinOptions,
  ): Graph<[...S, T], [...I, RowOf<T> | null]>;
  /** A hop no record reads: a junction table, a step of a chain. */
  via(relation: Relation<any, any, any>, options?: GraphJoinOptions): Graph<S, I>;
  /** A column of decoded source `source` — 0 the root, i the i-th slot — qualified as it is named. */
  col<N extends Indexes<S>, F extends FieldOf<ColumnsOf<S[N]>>>(source: N, field: F): ColumnOf<ColumnsOf<S[N]>, F>;
  where(predicate: Predicate): Graph<S, I>;
  orderBy(...orderings: OrderBy[]): Graph<S, I>;
  limit(count: number | bigint | null): Graph<S, I>;
  offset(count: number | bigint | null): Graph<S, I>;
  /** The SELECT it runs, every source projected under its prefix: `s0_` the root, `s{i}_` the i-th slot. */
  readonly statement: Select;
  /** The SQL and values it runs, or the ones `count` or `allGrouped` runs. */
  inspect(terminal?: "count" | "allGrouped"): Compiled;
  /** How many rows it reads, limit and offset aside. */
  count(db: Queryable, options?: OperationOptions): Promise<number>;
  all(db: Queryable, options?: OperationOptions): Promise<GraphItem<I>[]>;
  one(db: Queryable, options?: OperationOptions): Promise<GraphItem<I>>;
  optional(db: Queryable, options?: OperationOptions): Promise<GraphItem<I> | null>;
  stream(db: Pool | Connection, options?: OperationOptions): RowStream<GraphItem<I>>;
  /**
   * Each root with the records its one slot holds for it, the root's
   * primary key ordered behind the graph's ordering and the rows grouped by
   * the decoded root's key.
   */
  allGrouped(
    db: Queryable,
    options?: OperationOptions,
  ): Promise<I extends readonly [infer R, infer T] ? [R, Exclude<T, null>[]][] : never>;
  /**
   * A keyset cursor ordered by `fields` of the root, the root's primary key
   * and each slot's following as tiebreaks.
   */
  cursor<const F extends readonly [FieldOf<ColumnsOf<S[0]>>, ...FieldOf<ColumnsOf<S[0]>>[]]>(
    ...fields: F
  ): Cursor<GraphItem<I>, KeyInputs<ColumnsOf<S[0]>, F>>;
  /** Pages of `pageSize` rows, a boundary falling between rows rather than roots. */
  paginate(pageSize: number): Paginator<GraphItem<I>>;
}

/**
 * A keyset cursor over a model query's or a graph's rows `R`, its order
 * fields' boundary values `K`. Every method returns a new cursor.
 */
export declare class Cursor<R, K extends readonly unknown[] = readonly unknown[]> {
  private constructor();
  /**
   * The cursor ordered by `field` of `model` after its key and the entries
   * before it: a column of the queried model or a table it joined, `model`
   * named as the query names it. `after` and `before` still take the key
   * alone; `afterWith` and `beforeWith` take the whole ordering.
   */
  addSecondaryOrderBy<M extends Model<any>>(model: M, field: FieldOf<ColumnsOf<M>>): Cursor<R, K>;
  /** Rows past the one whose order-field values these are. */
  after(...values: K): Cursor<R, K>;
  /** Rows short of the one whose order-field values these are. */
  before(...values: K): Cursor<R, K>;
  /**
   * Rows past the one whose whole key this is — the order fields, then each
   * tiebreak — so a page that ended inside a run of equal order values
   * resumes inside it.
   */
  afterWith(...values: readonly unknown[]): Cursor<R, K>;
  beforeWith(...values: readonly unknown[]): Cursor<R, K>;
  /** The first `rows` rows in the cursor's order, replacing any window. */
  first(rows: number): Cursor<R, K>;
  /** The last `rows` rows in the cursor's order, read backwards and returned forwards. */
  last(rows: number): Cursor<R, K>;
  asc(): Cursor<R, K>;
  desc(): Cursor<R, K>;
  /** The SELECT it runs: its order replacing the query's, its window and boundaries applied. */
  readonly statement: Select;
  inspect(): Compiled;
  all(db: Queryable, options?: OperationOptions): Promise<R[]>;
}

/**
 * Pages of a model query's or a graph's rows `R`, `pageSize` at a time,
 * numbered from 0: each a limit and offset over the query, which should be
 * ordered for its pages to be stable.
 */
export declare class Paginator<R> {
  private constructor();
  readonly pageSize: number;
  /** The SQL and values reading page `page`, or with `"count"` the ones counting its rows. */
  inspect(page: number | "count"): Compiled;
  fetchPage(db: Queryable, page: number, options?: OperationOptions): Promise<R[]>;
  /** How many rows the pages hold, counted as pgorm's paginator counts. */
  numItems(db: Queryable, options?: OperationOptions): Promise<number>;
  numPages(db: Queryable, options?: OperationOptions): Promise<number>;
  numItemsAndPages(db: Queryable, options?: OperationOptions): Promise<{ readonly items: number; readonly pages: number }>;
  /** Each page in turn from the first, ending at the first empty one. */
  pages(db: Queryable, options?: OperationOptions): AsyncGenerator<R[], void, undefined>;
}
