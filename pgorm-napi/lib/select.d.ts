/**
 * SELECT, the tables and other items it reads, and common table expressions.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.select+3]
// [spec:pgorm:req:bindings.from-subqueries]
// [spec:pgorm:req:bindings.row-locks]
// [spec:pgorm:req:bindings.with+1]
// [spec:pgorm:req:bindings.cte-options]

import type { Aliased, Builder, Expr, OrderBy, Predicate } from "./expressions.d.ts";
import type { Merge } from "./merge.d.ts";
import type { ExtendedWindow, Window, Windowed } from "./windows.d.ts";
import type { BuiltinType, DataType } from "./json.d.ts";
import type { Delete, Insert, Update } from "./writes.d.ts";

/** A table, schema-qualified and aliased when the options say. Its names are identifiers, quoted. */
export declare class Table extends Builder {
  constructor(name: string, options?: { readonly schema?: string; readonly alias?: string });
  as(alias: string): Table;
  /** A column, qualified by the table's alias if it has one, else by its name. */
  col(name: string): Expr;
  /** Every column, `"table".*`. */
  star(): Expr;
  /**
   * The table read through `TABLESAMPLE`: each row (`"bernoulli"`) or page
   * (`"system"`) drawn with probability `percentage` out of 100, and the same
   * sample drawn again for the same `repeatable` seed over unchanged data.
   * Both are bound; a percentage outside 0 to 100 is PostgreSQL's refusal
   * (`2202H`). The item answers to the table's alias, or its name.
   */
  tablesample(method: SampleMethod, percentage: number, options?: { readonly repeatable?: number }): FromItem;
}

/** PostgreSQL's two built-in `TABLESAMPLE` methods: by row, or by page. */
export type SampleMethod = "bernoulli" | "system";

/**
 * A FROM item that is not a named table, always aliased: a subquery,
 * `JSON_TABLE`, a table function, or a sampled table.
 */
export declare class FromItem extends Builder {
  private constructor();
  col(name: string): Expr;
  star(): Expr;
}

/** A source a statement reads. */
export type Source = Table | FromItem;

export type Projection = Expr | Aliased | Windowed;

/** A `SELECT`; every method returns a new statement. */
export declare class Select extends Builder {
  private constructor();
  /** The projection replaced by at least one item. */
  select(first: Projection, ...rest: Projection[]): Select;
  /** One more FROM item, comma-joined. */
  from(item: Source): Select;
  /**
   * A join on `on`. `lateral` lets a subquery read the columns of the items
   * before it.
   */
  join(
    item: Source,
    on: Predicate,
    options?: { readonly kind?: "inner" | "left" | "right" | "full"; readonly lateral?: boolean },
  ): Select;
  crossJoin(item: Source): Select;
  /** A predicate ANDed to the WHERE clause. */
  where(predicate: Predicate): Select;
  /** Items added to the GROUP BY list in order: expressions and grouping elements. */
  groupBy(...items: (Expr | GroupingElement)[]): Select;
  /**
   * `GROUP BY DISTINCT`: a grouping set the list reaches more than once —
   * `ROLLUP (a, b)` beside `ROLLUP (a)` reaches `(a)` three ways — grouped
   * once.
   */
  groupByDistinct(): Select;
  having(predicate: Predicate): Select;
  orderBy(...orderings: OrderBy[]): Select;
  /**
   * LIMIT, or none with `null`: a non-negative integer within PostgreSQL's
   * `bigint`. It replaces a tie limit, and `null` removes either.
   */
  limit(count: number | bigint | null): Select;
  /**
   * `FETCH FIRST count ROWS WITH TIES` in LIMIT's place: the first `count`
   * rows and every row tied with the last on the ORDER BY. Inspecting or
   * running the statement throws a {@link ConstructionError} when it has no
   * ORDER BY or locks SKIP LOCKED, which PostgreSQL's grammar refuses; the
   * order may be given after the limit.
   */
  limitWithTies(count: number | bigint): Select;
  offset(count: number | bigint | null): Select;
  distinct(): Select;
  /**
   * `UNION`, with `other` as a parenthesised arm. A chain of set operations
   * combines in call order: each takes everything before it as its left
   * operand, so `a.union(b).intersect(c)` is `(a UNION b) INTERSECT c`, the
   * union parenthesised because PostgreSQL binds `INTERSECT` tighter. The
   * receiver's ORDER BY and limits order and bound the combined rows.
   */
  union(other: Select): Select;
  /** `UNION ALL`, combining in call order as {@link Select.union} does. */
  unionAll(other: Select): Select;
  /** `INTERSECT`, combining in call order as {@link Select.union} does. */
  intersect(other: Select): Select;
  /** `INTERSECT ALL`, combining in call order as {@link Select.union} does. */
  intersectAll(other: Select): Select;
  /** `EXCEPT`, combining in call order as {@link Select.union} does. */
  except(other: Select): Select;
  /** `EXCEPT ALL`, combining in call order as {@link Select.union} does. */
  exceptAll(other: Select): Select;
  /** Lock the rows read: `FOR UPDATE` and the weaker strengths, `OF` some items, without waiting. */
  lock(
    strength: "update" | "noKeyUpdate" | "share" | "keyShare",
    options?: { readonly of?: readonly Source[]; readonly wait?: "nowait" | "skipLocked" },
  ): Select;
  /** The statement's WITH clause, replacing any it had. */
  with(clause: With): Select;
  /**
   * An entry appended to the WINDOW clause, which `over(name)` reads: a
   * `Window`, or an `ExtendedWindow` built on an entry appended before it.
   * A clause PostgreSQL refuses is refused when the statement is inspected,
   * run or read into another.
   */
  window(name: string, window: Window | ExtendedWindow): Select;
  /** This query as a FROM item, `(SELECT ..) AS "alias"`. */
  as(alias: string): FromItem;
}

/** `SELECT items`, or `SELECT *` when there are none. */
export declare function select(...items: Projection[]): Select;

/**
 * An item of a GROUP BY list beyond a bare expression. Each stands for a list
 * of grouping sets, and the list groups by every combination of its items'.
 */
export declare class GroupingElement {
  #private;
  private constructor();
  /** One grouping set, `(a, b)`; one expression renders bare, and none `()`. */
  static set(...expressions: Expr[]): GroupingElement;
  /** The empty grouping set, `()`: the whole input as one group. */
  static empty(): GroupingElement;
  /** `ROLLUP (a, b)`: the sets `(a, b)`, `(a)` and `()`. A `tuple` item is one unit. */
  static rollup(...expressions: Expr[]): GroupingElement;
  /** `CUBE (a, b)`: every subset of the list, `()` included. */
  static cube(...expressions: Expr[]): GroupingElement;
  /** `GROUPING SETS (..)`: exactly its items' sets, an expression the set of itself; never empty. */
  static sets(first: Expr | GroupingElement, ...rest: (Expr | GroupingElement)[]): GroupingElement;
}

/** How a {@link tableFunction} is read. */
export interface TableFunctionOptions {
  /** The item's alias, which every FROM item that is not a table has. */
  readonly alias: string;
  /** `WITH ORDINALITY`: a final `bigint` column numbering the rows from 1. */
  readonly ordinality?: boolean;
  /** The columns' names in the alias, `AS "t"("a", "b")`, the ordinal last. */
  readonly columns?: readonly string[];
  /** `ROWS FROM (..)` over a single call; several calls are always read so. */
  readonly rowsFrom?: boolean;
}

/**
 * A call to a function returning `record`, `json_to_record` and its kin among
 * them, with the column definition list that gives its columns: each a name
 * and a type, at least one.
 */
export interface RecordCall {
  readonly call: Expr;
  readonly columnDefinitions: readonly [ColumnDefinition, ...ColumnDefinition[]];
}

/** A column of a {@link RecordCall}: its name and its type. */
export type ColumnDefinition = readonly [name: string, type: DataType | BuiltinType];

/**
 * Set-returning calls as a FROM item: `f(..) AS alias`, or `ROWS FROM (f(..),
 * g(..))` over several, which reads them side by side and pads the shorter
 * ones with NULLs. Each call is one {@link call} made, `generate_series` and
 * `unnest` among them, or a {@link RecordCall}; any other expression is a
 * {@link ConstructionError}. A lone record call writes its list in the alias,
 * `AS "t"("a" integer)`, and beside `ordinality` or `columns`, or among
 * several calls, each call carries its own inside `ROWS FROM (..)`.
 */
export declare function tableFunction(
  calls: Expr | RecordCall | readonly [Expr | RecordCall, ...(Expr | RecordCall)[]],
  options: TableFunctionOptions,
): FromItem;

/** A common table expression's column names and materialization. */
export interface CteOptions {
  readonly columns?: readonly string[];
  readonly materialized?: boolean;
}

/** What a common table expression's rows come from: a query, or a write's RETURNING list. */
export type CteBody = Select | Insert | Update | Delete | Merge;

/** A WITH clause: common table expressions a statement reads by name. */
export declare class With extends Builder {
  constructor(name: string, body: CteBody, options?: CteOptions);
  /** One more common table expression, which may read the ones before it. */
  cte(name: string, body: CteBody, options?: CteOptions): With;
  /**
   * `WITH RECURSIVE`, of exactly one common table expression, whose body may
   * read itself, with SEARCH and CYCLE when given.
   */
  static recursive(
    name: string,
    body: Select,
    options?: CteOptions & {
      readonly search?: { readonly order: "breadth" | "depth"; readonly by: Expr; readonly set: string };
      readonly cycle?: { readonly by: Expr; readonly set: string; readonly using: string };
    },
  ): With;
}
