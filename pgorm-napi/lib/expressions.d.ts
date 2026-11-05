/**
 * Expressions and conditions over pgorm-query's builders.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.expressions+2]
// [spec:pgorm:req:bindings.quantifiers+2]
// [spec:pgorm:req:bindings.case]

import type { Multirange, Param, Range, TypeName, Value } from "./index.d.ts";
import type { EntityColumn } from "./entities.d.ts";
import type { ModelColumn } from "./models.d.ts";
import type { Select } from "./select.d.ts";
import type { ExtendedWindow, Window, Windowed } from "./windows.d.ts";

/** The SQL and the bound values a builder makes, exactly as running it would. */
export interface Compiled {
  readonly sql: string;
  /** Each bound value with its kind, in placeholder order. */
  readonly values: readonly Value[];
}

/**
 * What every statement and expression builder is: immutable, each method
 * returning a new builder from a copy of its receiver's pgorm-query state.
 */
export declare abstract class Builder {
  /**
   * The SQL and values: a statement as it runs, an expression as the one item
   * of a `SELECT`, a condition as the `WHERE` of `SELECT TRUE`.
   */
  inspect(): Compiled;
}

/**
 * A value a built statement binds: a parameter, inferred or declared with
 * {@link Value}, but not `null`, which has no kind — test with `isNull()`, or
 * bind `Value.null(kind)`.
 */
export type BindValue = Exclude<Param, null>;

/** What an operand may be: an expression, or a value to bind. */
export type Operand = Expr | BindValue;

/**
 * What a comparison, a membership test, `between`, a distinctness test,
 * containment, overlap or a range position of the expression `S` takes as
 * its value: a model's column or a registered entity's column takes what its
 * declared kind converts, a `Range` or `Multirange` for a range or
 * multirange kind, and any other expression an {@link Operand}.
 *
 * `Expr`'s methods find it through their receiver, `this`, rather than each
 * column redeclaring them: a `Range` is no `Operand`, and a column whose
 * methods took one in place of an `Operand` would be no `Expr`.
 */
export type ComparisonOperand<S> = S extends ModelColumn<any, infer I> ? I
  : S extends EntityColumn<infer I> ? I
  : Operand;

export type Predicate = Expr | Condition;

/** Where NULLs sort. */
export type Nulls = "first" | "last";

/** An SQL expression; every method returns a new one. */
export declare class Expr extends Builder {
  protected constructor();
  /** `=`; `any(..)` or `all(..)` compares with an array's elements or a subquery's rows. */
  eq<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  ne<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  lt<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  lte<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  gt<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  gte<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  add(other: Operand): Expr;
  sub(other: Operand): Expr;
  mul(other: Operand): Expr;
  div(other: Operand): Expr;
  mod(other: Operand): Expr;
  /** `a || b`. */
  concat(other: Operand): Expr;
  /** `@>`: a range, multirange or array containing the other. */
  contains<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `<@`: contained by the other. */
  containedBy<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `&&`: sharing a value with the other. */
  overlaps<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `<<`: a range or multirange lying wholly left of the other. */
  strictlyLeftOf<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `>>`: a range or multirange lying wholly right of the other. */
  strictlyRightOf<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `&<`: extending no further right than the other. */
  notExtendRightOf<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `&>`: extending no further left than the other. */
  notExtendLeftOf<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `-|-`: adjacent to the other, sharing no value with nothing between. */
  adjacentTo<S>(this: S, other: ComparisonOperand<S> | Quantified): Expr;
  /** `~`: the text matches a POSIX regular expression, case sensitively; the pattern is bound unless it is an expression. */
  regex(pattern: Operand | Quantified): Expr;
  /** `!~`: the text does not match the regular expression. */
  notRegex(pattern: Operand | Quantified): Expr;
  /** `~*`: the text matches the regular expression, case ignored. */
  iregex(pattern: Operand | Quantified): Expr;
  /** `!~*`: the text does not match the regular expression, case ignored. */
  notIregex(pattern: Operand | Quantified): Expr;
  /**
   * `@@`: a `tsvector` against a `tsquery`, text against text as PostgreSQL's
   * full-text conversion reads it, or a `jsonb` against a `jsonpath`
   * predicate; a `jsonpath` is written cast, as a bound string is text.
   */
  matches(other: Operand | Quantified): Expr;
  /**
   * `@?`: a `jsonpath` returns an item for this `jsonb`; the path is bound as
   * text cast to `jsonpath`, or `any(..)` or `all(..)` ranges over `jsonpath`
   * values, which a bound string array is not.
   */
  pathExists(path: string | Quantified): Expr;
  isDistinctFrom<S>(this: S, other: ComparisonOperand<S>): Expr;
  isNotDistinctFrom<S>(this: S, other: ComparisonOperand<S>): Expr;
  and(other: Expr): Expr;
  or(other: Expr): Expr;
  not(): Expr;
  isNull(): Expr;
  isNotNull(): Expr;
  /**
   * Membership in a list of operands, or in a subquery's rows. An empty list
   * holds nothing: `isIn([])` is false and `isNotIn([])` true.
   */
  isIn<S>(this: S, source: readonly ComparisonOperand<S>[] | Select): Expr;
  isNotIn<S>(this: S, source: readonly ComparisonOperand<S>[] | Select): Expr;
  between<S>(this: S, lower: ComparisonOperand<S>, upper: ComparisonOperand<S>, options?: { readonly symmetric?: boolean }): Expr;
  notBetween<S>(this: S, lower: ComparisonOperand<S>, upper: ComparisonOperand<S>, options?: { readonly symmetric?: boolean }): Expr;
  /**
   * A pattern match; the pattern is bound, and `escape` is one character, or
   * `""` to turn escaping off. `any(..)` or `all(..)` over patterns takes no
   * escape.
   */
  like(pattern: string, options?: { readonly escape?: string }): Expr;
  like(pattern: Quantified): Expr;
  notLike(pattern: string, options?: { readonly escape?: string }): Expr;
  notLike(pattern: Quantified): Expr;
  ilike(pattern: string, options?: { readonly escape?: string }): Expr;
  ilike(pattern: Quantified): Expr;
  notIlike(pattern: string, options?: { readonly escape?: string }): Expr;
  notIlike(pattern: Quantified): Expr;
  /**
   * `SIMILAR TO`: whether the whole string matches a pattern in SQL's
   * regular-expression dialect — LIKE's `%` and `_` with `|`, `*`, `+`, `?`,
   * repetition, grouping and bracket classes. Bound as LIKE's pattern is, with
   * the same `escape`. A bound left operand needs its type, as
   * the server cannot infer both sides from the operator alone.
   */
  similarTo(pattern: string, options?: { readonly escape?: string }): Expr;
  notSimilarTo(pattern: string, options?: { readonly escape?: string }): Expr;
  /** Whether the text begins with `text`, read as text: `%` and `_` match themselves. */
  startsWith(text: Operand): Expr;
  endsWith(text: Operand): Expr;
  containsText(text: Operand): Expr;
  /** `CAST(expr AS type)`, the type named by identifier. */
  cast(type: string | TypeName, options?: { readonly array?: boolean }): Expr;
  /** `(expr COLLATE "collation")`. */
  collate(collation: string, options?: { readonly schema?: string }): Expr;
  /** An array element, `a[index]`; arrays count from 1. */
  at(index: Operand): Expr;
  /** An array slice, `a[lower:upper]`; `null` leaves that end open. */
  slice(lower: Operand | null, upper: Operand | null): Expr;
  /**
   * This call over a window, inline or named by the statement; an inline
   * `ExtendedWindow` builds on an entry the statement declares. Only a
   * function call or `jsonArrayagg` / `jsonObjectagg` takes one.
   */
  over(window: Window | ExtendedWindow | string): Windowed;
  /** A projection naming its output column. */
  as(alias: string): Aliased;
  asc(options?: { readonly nulls?: Nulls }): OrderBy;
  desc(options?: { readonly nulls?: Nulls }): OrderBy;
}

/**
 * `ANY (..)` or `ALL (..)`: the right operand of an operator PostgreSQL reads
 * one after — a comparison, `like`, `notLike`, `ilike` and `notIlike`, the
 * regular-expression matches, containment, the range positions, `matches`
 * and `pathExists` — and no expression of its own.
 */
export declare class Quantified {
  #private;
  private constructor();
}

/** An expression and the output column name it takes in a projection. */
export declare class Aliased extends Builder {
  #private;
  private constructor();
}

/** An ORDER BY item, from `expr.asc()` or `expr.desc()`. */
export declare class OrderBy extends Builder {
  #private;
  private constructor();
}

/** A boolean combination of predicates. */
export declare class Condition extends Builder {
  private constructor();
  /** Their AND; true when there are none. */
  static all(...items: Predicate[]): Condition;
  /** Their OR; false when there are none. */
  static any(...items: Predicate[]): Condition;
  add(item: Predicate): Condition;
  not(): Condition;
}

/** A searched `CASE` with at least one arm. */
export declare class SearchedCase extends Expr {
  private constructor();
  when(condition: Predicate, result: Operand): SearchedCase;
  else(result: Operand): Expr;
}

/** A simple `CASE` with at least one arm. */
export declare class SimpleCase extends Expr {
  private constructor();
  when(value: Operand, result: Operand): SimpleCase;
  else(result: Operand): Expr;
}

/** The operand of a simple `CASE`, which is no expression until it has an arm. */
export declare class CaseOperand extends Builder {
  private constructor();
  when(value: Operand, result: Operand): SimpleCase;
}

/** A column, qualified by a table and its schema when they are given. */
export declare function col(name: string, options?: { readonly table?: string; readonly schema?: string }): Expr;
/** A value bound as a parameter. */
export declare function bind(value: BindValue): Expr;
/**
 * A value written into the SQL text, as pgorm-query renders a constant, read
 * as a parameter is read; every other operand is bound.
 */
export declare function literal(value: BindValue): Expr;

/** The functions {@link call} reaches, each taking the arguments its pgorm-query constructor does. */
export type FunctionName =
  | "lower"
  | "upper"
  | "abs"
  | "char_length"
  | "count"
  | "count_distinct"
  | "sum"
  | "avg"
  | "min"
  | "max"
  | "round"
  | "coalesce"
  | "random"
  | "gen_random_uuid"
  | "uuidv4"
  | "uuidv7"
  | "uuid_extract_timestamp"
  | "uuid_extract_version"
  | "generate_series"
  | "unnest"
  | "json_to_record"
  | "jsonb_to_record"
  | "json_to_recordset"
  | "jsonb_to_recordset";

/**
 * A call to one of the functions pgorm-query has a constructor for; another
 * name or argument count is a {@link ConstructionError}. `uuidv7` takes no
 * argument, or the interval its time is shifted by.
 *
 * `generate_series` (start, stop and an optional step) and `unnest` (one
 * array) are set-returning, read as a relation by `tableFunction`. Their
 * overloads are chosen by argument type and a parameter has none, so a bound
 * argument needs its type: `bind(1).cast("int4")`, or an array cast with
 * `{ array: true }`; a column or a typed expression already has one.
 *
 * `json_to_record`, `jsonb_to_record`, `json_to_recordset` and
 * `jsonb_to_recordset` (one document each) return `record`, read by
 * `tableFunction` through the column definition list a `RecordCall` gives.
 */
export declare function call(name: FunctionName, ...operands: Operand[]): Expr;
/** `(a, b, ..)`, at least one item. */
export declare function tuple(first: Operand, ...rest: Operand[]): Expr;
/** `EXISTS (select)`. */
export declare function exists(select: Select): Expr;
/** `(select)`, a subquery yielding one value. */
export declare function scalar(select: Select): Expr;
/**
 * `ARRAY(select)`: the subquery's one column collected into one array value,
 * empty when the subquery yields no row, which can be cast and compared as a
 * value where a subquery's rows cannot. PostgreSQL refuses a subquery of more
 * than one column here (`42601`).
 */
export declare function array(select: Select): Expr;
/**
 * What `any(..)` or `all(..)` ranges over: a subquery's rows, an array
 * expression, a value bound as one array parameter, or a list of values,
 * ranges among them, which a registered column types where the list alone
 * says too little.
 */
export type QuantifiedSource = Select | Operand | readonly (BindValue | Range<any> | Multirange<any>)[];
/**
 * `ANY (..)`: the operator holds for some element of an array — an
 * expression, or a value bound as one array parameter — or for some row of a
 * subquery; over an empty array or no rows it is false.
 */
export declare function any(source: QuantifiedSource): Quantified;
/**
 * `ALL (..)`: the operator holds for every element of an array, or every
 * row of a subquery; over an empty array or no rows it is true.
 */
export declare function all(source: QuantifiedSource): Quantified;
/** A searched `CASE` with its first arm. */
export declare function caseWhen(condition: Predicate, result: Operand): SearchedCase;
/** A simple `CASE` over `operand`; its first `when` makes it an expression. */
export declare function caseOf(operand: Operand): CaseOperand;
