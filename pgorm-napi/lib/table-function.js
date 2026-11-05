// @ts-check

// Set-returning functions read as a FROM item: one call, or several zipped
// side by side under ROWS FROM, the shorter ones padded with NULLs — built as
// pgorm-query's TableFunction is, and finished by `alias`.
// [spec:pgorm:req:bindings.select+4]

import { arg, Handle, trusted, TRUSTED } from "./builder.js";
import { native } from "./operations.js";
import { FromItem } from "./select.js";

/**
 * @typedef {import("./expressions.js").Expr} Expr
 */

/** A function in FROM before its alias, which `alias` gives it. */
export class TableFunction extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "TableFunction", "tableFunction() or TableFunction.rowsFrom()");
    super(handle);
  }

  /**
   * `ROWS FROM (f(..))`, to which `and` adds calls. Over one call it differs
   * from `tableFunction(call)` only for a function returning a composite,
   * whose columns `ROWS FROM` expands.
   *
   * @param {Expr} call
   */
  static rowsFrom(call) {
    return new TableFunction(native.tableFunctionNew(arg(call), true), TRUSTED);
  }

  /**
   * Another call read beside the others; several calls are `ROWS FROM`
   * whichever factory began it.
   *
   * @param {Expr} call
   */
  and(call) {
    return new TableFunction(native.tableFunctionAnd(arg(this), arg(call)), TRUSTED);
  }

  /**
   * A column of the `record` the call added last returns, its name and type:
   * a lone call writes the list in its alias, and each call of a `ROWS FROM`
   * carries its own.
   *
   * @param {string} name
   * @param {import("./schema.d.ts").ColumnType} type
   */
  columnDefinition(name, type) {
    return new TableFunction(native.tableFunctionColumnDefinition(arg(this), name, arg(type)), TRUSTED);
  }

  /** `WITH ORDINALITY`: a final `bigint` column numbering the rows from 1. */
  withOrdinality() {
    return new TableFunction(native.tableFunctionWithOrdinality(arg(this)), TRUSTED);
  }

  /**
   * The relation's columns named in its alias, `AS "t"("a", "b")`, the
   * ordinal column last, replacing any names given before.
   *
   * @param {...string} names
   */
  columns(...names) {
    return new TableFunction(native.tableFunctionColumns(arg(this), names), TRUSTED);
  }

  /**
   * The relation under `name`, the FROM item `from` and the joins take.
   *
   * @param {string} name
   */
  alias(name) {
    return new FromItem(native.tableFunctionAlias(arg(this), name), TRUSTED);
  }
}

/**
 * One set-returning call, `f(..)`, read as a relation: a call `call()` made,
 * `generate_series` and `unnest` among them, or one returning `record` with
 * the columns `columnDefinition` gives it.
 *
 * @param {Expr} call
 */
export function tableFunction(call) {
  return new TableFunction(native.tableFunctionNew(arg(call), false), TRUSTED);
}
