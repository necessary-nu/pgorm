// @ts-check

// Set-returning functions read as a FROM item: one call, or several zipped
// side by side under ROWS FROM, the shorter ones padded with NULLs.
// [spec:pgorm:req:bindings.select+3]

import { arg, Handle, TRUSTED } from "./builder.js";
import { native } from "./operations.js";
import { FromItem } from "./select.js";

/**
 * @typedef {import("./expressions.js").Expr} Expr
 * @typedef {readonly [string, import("./data-type.js").DataType | string]} ColumnDefinition
 * @typedef {{ readonly call: Expr, readonly columnDefinitions: readonly ColumnDefinition[] }} RecordCall
 */

/**
 * `f(..) AS alias`, or `ROWS FROM (f(..), g(..)) AS alias` over several
 * calls or when `rowsFrom` says; `ordinality` adds a final `bigint` column
 * numbering the rows from 1, and `columns` names the columns in the alias,
 * the ordinal last. Each call is one `call()` made — `generate_series` and
 * `unnest` among them — or, for a function returning `record`, `{ call,
 * columnDefinitions }`, its columns' names and types. The alias is required,
 * as every FROM item that is not a table has one.
 *
 * @param {Expr | RecordCall | readonly (Expr | RecordCall)[]} calls
 * @param {{ alias: string, ordinality?: boolean, columns?: readonly string[], rowsFrom?: boolean }} options
 */
export function tableFunction(
  calls,
  { alias, ordinality = false, columns, rowsFrom = false } = /** @type {any} */ ({}),
) {
  const each = Array.isArray(calls) ? calls : [calls];
  const list = each.map((call) => arg(recordCall(call) ? call.call : call));
  const definitions = each.map((call) =>
    recordCall(call) ? Array.from(call.columnDefinitions, (column) => Array.from(column, arg)) : null
  );
  const handle = native.tableFunction(list, alias, ordinality === true, columns, rowsFrom === true, definitions);
  return new FromItem(handle, TRUSTED);
}

/**
 * Whether `call` is a call with its column definition list rather than a
 * call alone.
 *
 * @param {unknown} call
 * @returns {call is RecordCall}
 */
function recordCall(call) {
  return typeof call === "object" && call !== null && !(call instanceof Handle) && "call" in call;
}
