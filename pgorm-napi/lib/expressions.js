// @ts-check

// Expressions over pgorm-query's builders. An operand that is not an
// expression is a value, bound as a parameter: nothing a caller passes becomes
// SQL text, and an identifier is quoted wherever it appears.
// [spec:pgorm:req:bindings.expressions+2]
// [spec:pgorm:req:bindings.quantifiers+1]

import { arg, args, Builder, Handle, made, trusted, TRUSTED } from "./builder.js";
import { native } from "./operations.js";
import { Predicate } from "./predicates.js";
import { Windowed } from "./windows.js";

/**
 * @param {Expr} expr
 * @param {"asc" | "desc"} direction
 * @param {{ nulls?: "first" | "last" }} options
 */
function ordering(expr, direction, { nulls } = {}) {
  return new OrderBy(native.exprOrder(arg(expr), direction, nulls), TRUSTED);
}

/** An SQL expression. Every method returns a new expression. */
export class Expr extends Predicate {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "SQL expression", "col(), bind(), literal(), call() and the other expression functions");
    super(handle);
  }

  /**
   * @param {string | import("./values.js").TypeName} type
   * @param {{ array?: boolean }} [options]
   */
  cast(type, { array = false } = {}) {
    return new Expr(native.exprCast(arg(this), type, array === true), TRUSTED);
  }

  /**
   * @param {string} collation
   * @param {{ schema?: string }} [options]
   */
  collate(collation, { schema } = {}) {
    return new Expr(native.exprCollate(arg(this), collation, schema), TRUSTED);
  }

  /** @param {unknown} index */
  at(index) {
    return new Expr(native.exprSubscript(arg(this), arg(index)), TRUSTED);
  }

  /**
   * @param {unknown} lower
   * @param {unknown} upper
   */
  slice(lower, upper) {
    return new Expr(native.exprSubscript(arg(this), arg(lower ?? null), arg(upper ?? null), true), TRUSTED);
  }

  /**
   * This call over a `Window` or an `ExtendedWindow` written inline, or over
   * the window the statement names; only a function call or `jsonArrayagg` /
   * `jsonObjectagg` takes one.
   *
   * @param {import("./windows.js").Window | import("./windows.js").ExtendedWindow | string} window
   */
  over(window) {
    return new Windowed(native.exprOver(arg(this), arg(window)), TRUSTED);
  }

  /** @param {string} alias */
  as(alias) {
    return new Aliased(native.exprAlias(arg(this), alias), TRUSTED);
  }

  /** @param {{ nulls?: "first" | "last" }} [options] */
  asc(options) {
    return ordering(this, "asc", options);
  }

  /** @param {{ nulls?: "first" | "last" }} [options] */
  desc(options) {
    return ordering(this, "desc", options);
  }
}

made.expr = (handle) => new Expr(handle, TRUSTED);

/** An expression and the name of the output column it makes. */
export class Aliased extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "aliased projection", "expr.as(name)");
    super(handle);
  }
}

/** An ORDER BY item. */
export class OrderBy extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "ordering", "expr.asc() or expr.desc()");
    super(handle);
  }
}

/**
 * A column, qualified by a table and its schema when they are given.
 *
 * @param {string} name
 * @param {{ table?: string, schema?: string }} [options]
 */
export function col(name, { table, schema } = {}) {
  return new Expr(native.exprCol(name, table, schema), TRUSTED);
}

/**
 * A value bound as a parameter, inferred as a parameter is or declared with
 * `Value`.
 *
 * @param {unknown} value
 */
export function bind(value) {
  return new Expr(native.exprBind(value), TRUSTED);
}

// [spec:pgorm:req:bindings.operands]
/**
 * A value written into the SQL text, as pgorm-query renders a constant, read
 * as a parameter is read. Every other operand is bound.
 *
 * @param {unknown} value
 */
export function literal(value) {
  return new Expr(native.exprLiteral(value), TRUSTED);
}

/**
 * A call to one of the functions pgorm-query constructs.
 *
 * @param {string} name
 * @param {...unknown} operands
 */
export function call(name, ...operands) {
  return new Expr(native.exprCall(name, args(operands)), TRUSTED);
}

/** @param {...unknown} items */
export function tuple(...items) {
  return new Expr(native.exprTuple(args(items)), TRUSTED);
}

/**
 * `EXISTS (select)`.
 *
 * @param {import("./select.js").Select} select
 */
export function exists(select) {
  return new Expr(native.exprSubquery("exists", arg(select)), TRUSTED);
}

/**
 * `(select)`, a subquery yielding one value.
 *
 * @param {import("./select.js").Select} select
 */
export function scalar(select) {
  return new Expr(native.exprSubquery("scalar", arg(select)), TRUSTED);
}

/**
 * `ARRAY(select)`: the subquery's one column collected into one array value,
 * which can be cast and compared as a value where a subquery's rows cannot.
 *
 * @param {import("./select.js").Select} select
 */
export function array(select) {
  return new Expr(native.exprSubquery("array", arg(select)), TRUSTED);
}

/**
 * The quantifier and the values `value` ranges over when it is `any(..)` or
 * `all(..)` over a list holding no expression: what a registered column
 * converts to its declared kind and compares through its `ColumnTrait`.
 *
 * @type {(value: unknown) => readonly ["any" | "all", readonly unknown[]] | undefined}
 */
export let listedValues;

/**
 * `ANY (..)` or `ALL (..)`, the right operand of an operator PostgreSQL reads
 * one after and nothing else: `col("id").eq(any(ids))`,
 * `col("name").like(any(patterns))`. It has no SQL of its own to inspect.
 */
export class Quantified extends Handle {
  /** @type {readonly ["any" | "all", readonly unknown[]] | undefined} */
  #listed;

  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   * @param {"any" | "all"} [quantifier]
   * @param {unknown} [source]
   */
  constructor(handle, token, quantifier, source) {
    trusted(token, "quantified operand", "any() or all()");
    super(handle);
    if (quantifier !== undefined && Array.isArray(source) && !source.some((item) => item instanceof Handle)) {
      this.#listed = Object.freeze([quantifier, Object.freeze([...source])]);
    }
  }

  static {
    listedValues = (value) => value instanceof Quantified ? value.#listed : undefined;
  }
}

/**
 * `ANY (..)`: true when the operator holds for some element of an array —
 * an expression, or a value bound as one array parameter — or some row of a
 * subquery. Over an empty array or no rows it is false.
 *
 * @param {import("./select.js").Select | unknown} source
 */
export function any(source) {
  return new Quantified(native.exprQuantified("any", arg(source)), TRUSTED, "any", source);
}

/**
 * `ALL (..)`: true when the operator holds for every element of an array,
 * or every row of a subquery. Over an empty array or no rows it is true.
 *
 * @param {import("./select.js").Select | unknown} source
 */
export function all(source) {
  return new Quantified(native.exprQuantified("all", arg(source)), TRUSTED, "all", source);
}
