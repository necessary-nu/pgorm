// @ts-check

// SQL text and its values held together inside the module, where a reader of
// raw SQL keeps a statement it runs more than once; and the run-time checks
// that stand in for Rust's compile-time `sql!` and `prql!`. A program's own
// statements stay SQL text and an array, as every terminal takes them
// (napi.raw).

import { ConstructionError } from "./errors.js";
import { native } from "./operations.js";

/**
 * SQL text and the values its placeholders bind, as a `SelectorRaw`, a page
 * of one or its count holds them: never made by a program, which passes text
 * and an array.
 */
export class Held {
  /** @type {string} */
  #sql;
  /** @type {readonly unknown[]} */
  #values;

  /**
   * @param {unknown} sql
   * @param {unknown} values
   */
  constructor(sql, values = []) {
    if (typeof sql !== "string") throw new TypeError("the statement is a string");
    if (!Array.isArray(values)) throw new TypeError("a statement's parameters are an array");
    if (sql.includes("\0")) throw new ConstructionError("statement text cannot contain NUL");
    this.#sql = sql;
    this.#values = Object.freeze([...values]);
    Object.freeze(this);
  }

  get sql() {
    return this.#sql;
  }

  get values() {
    return this.#values;
  }

  /** The SQL and values running it sends. */
  inspect() {
    return Object.freeze({ sql: this.#sql, values: this.#values });
  }
}

/**
 * `text`, held to PostgreSQL 18's grammar and returned unchanged, as Rust's
 * `sql!` holds a literal while its crate compiles: a script is held to it
 * statement by statement. What the grammar rejects is a `ConstructionError`
 * whose message reprints the line with a caret under the token the parser
 * stopped at. Call it once, where a module defines its statements.
 * [spec:pgorm:req:bindings.sql-check]
 *
 * @template {string} T
 * @param {T} text
 * @returns {T}
 */
export function sql(text) {
  return /** @type {T} */ (native.sqlCheck(text));
}

/**
 * `text`, PRQL, compiled to PostgreSQL SQL held to the grammar, with `values`
 * for its placeholders in order, as Rust's `prql!` expands: one value per
 * distinct placeholder. The pair runs as SQL text and its array do.
 * [spec:pgorm:req:bindings.prql-text]
 *
 * @param {string} text
 * @param {readonly unknown[]} [values]
 */
export function prql(text, values = []) {
  if (!Array.isArray(values)) throw new TypeError("a query's values are an array");
  return Object.freeze({ sql: /** @type {string} */ (native.prqlSql(text, values.length)), values: Object.freeze([...values]) });
}
