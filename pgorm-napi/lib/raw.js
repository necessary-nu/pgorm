// @ts-check

// What SQL text reads beyond its terminals: rows decoded into a declared
// model's records, a statement checked against a model before any row
// exists, and pages and counts over a SELECT written by hand. Each is
// pgorm's own: `SelectorRaw`, `VerifyStatement::describe` and `raw_select`.

import { paginatorOver } from "./cursors.js";
import { DecodeError } from "./errors.js";
import { columnKey, readsText } from "./model-columns.js";
import { decodeAll, decodeCount, decodeOne, decodeOptional, decodeStream } from "./model-decode.js";
import { shapeOf } from "./model-shape.js";
import { native } from "./operations.js";
import { Held } from "./raw-sql.js";

/** Marks a construction the module made. */
const MADE = Symbol("pgorm-napi raw selector");

/**
 * A statement of SQL text over the wrapped statement `sql` builds, its
 * parameters in the order the wrapper reads them: each a position among the
 * statement's own values, or past them one of the values the wrapper binds.
 *
 * @param {Held} statement
 * @param {[string, number[], unknown[]]} built
 */
function rebound(statement, [sql, order, added]) {
  const values = statement.values;
  return new Held(sql, order.map((position) => position < values.length ? values[position] : added[position - values.length]));
}

/**
 * The statements a raw statement's pages are read and counted with,
 * `statement` read as a subquery, refused unless it is one row-returning
 * `SELECT`, as Rust's `SelectorRaw::paginate` refuses it.
 * [spec:pgorm:req:bindings.raw-pages]
 *
 * @param {Held} statement
 */
function pageable(statement) {
  const counted = rebound(statement, native.rawCount(statement.sql, statement.values.length));
  return {
    page: (/** @type {number} */ limit, /** @type {number} */ offset) =>
      rebound(statement, native.rawPage(statement.sql, statement.values.length, limit, offset / limit)),
    counted,
  };
}

/**
 * The source a raw statement's rows decode as: every field of `model`, each
 * read from the column of its SQL name, other columns left unread, as
 * Rust's `FromQueryResult` reads a row by column name; or with no model none,
 * the rows read as plain objects.
 *
 * @param {object | null} model
 */
function source(model) {
  if (model === null) return null;
  const shape = shapeOf(model);
  return [{ fields: shape.fields, names: shape.fields.map((field) => field.sql), optional: false }];
}

/**
 * A raw statement whose rows decode into a declared model's records, as
 * Rust's `SelectorRaw` decodes them, or are read as plain objects. pgorm adds
 * nothing to the SQL, so `one` and `optional` add no `LIMIT`.
 * [spec:pgorm:req:bindings.raw-models]
 */
export class SelectorRaw {
  /** @type {Held} */
  #statement;
  /** @type {object | null} */
  #model;

  /**
   * @param {Held} statement
   * @param {object | null} model
   * @param {symbol} token
   */
  constructor(statement, model, token) {
    if (token !== MADE) throw new TypeError("a SelectorRaw is made by a model's findByStatement()");
    this.#statement = statement;
    this.#model = model;
    Object.freeze(this);
  }

  /** The SQL and values running it sends. */
  inspect() {
    return this.#statement.inspect();
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    const sources = source(this.#model);
    if (sources === null) return await queryable(db).query(this.#statement, options);
    return await decodeAll(db, this.#statement, sources, options, false);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    const sources = source(this.#model);
    if (sources === null) return await queryable(db).one(this.#statement, options);
    return await decodeOne(db, this.#statement, sources, options, false);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    const sources = source(this.#model);
    if (sources === null) return await queryable(db).optional(this.#statement, options);
    return await decodeOptional(db, this.#statement, sources, options, false);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    const sources = source(this.#model);
    if (sources === null) return queryable(db).stream(this.#statement, options);
    return decodeStream(db, this.#statement, sources, options, false);
  }

  /**
   * Pages of the statement's records, the statement read as a subquery,
   * refused unless it is one row-returning `SELECT`.
   * [spec:pgorm:req:bindings.raw-pages]
   *
   * @param {unknown} pageSize
   */
  paginate(pageSize) {
    return paginatorOver({ ...pageable(this.#statement), sources: source(this.#model), exhaustive: false }, pageSize);
  }

  /**
   * How many rows the statement returns.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    return await decodeCount(db, pageable(this.#statement).counted, options);
  }
}

/**
 * What runs a raw statement's rows as plain objects.
 *
 * @param {unknown} db
 * @returns {{ query: Function, one: Function, optional: Function, stream: Function }}
 */
function queryable(db) {
  const runs = /** @type {{ query?: unknown, stream?: unknown }} */ (db);
  if (typeof runs?.query !== "function" || typeof runs?.stream !== "function") {
    throw new TypeError("a raw statement runs on a Pool, a Connection or a Transaction");
  }
  return /** @type {any} */ (db);
}

/**
 * `sql` and its `params` as a raw statement whose rows are plain objects,
 * which pages and counts, as Rust's `SelectorRaw::from_statement` holds one.
 *
 * @param {unknown} sql
 * @param {unknown} [params]
 */
export function fromStatement(sql, params) {
  return new SelectorRaw(new Held(sql, params ?? []), null, MADE);
}

/**
 * `statement`, SQL text, and its `params`, the rows decoded into `model`'s
 * records.
 *
 * @param {object} model
 * @param {unknown} statement
 * @param {unknown} [params]
 */
export function findByStatement(model, statement, params) {
  shapeOf(model);
  return new SelectorRaw(new Held(statement, params ?? []), model, MADE);
}

/**
 * Check `model` against the result the server describes for `statement`,
 * without running it, as Rust's `verify` does: every field's column
 * returned, of a type the field's kind reads, or a `DecodeError` naming the
 * field. The statement's values are neither bound nor checked.
 * [spec:pgorm:req:bindings.verify]
 *
 * @param {(job: unknown, options: unknown) => Promise<any>} run
 * @param {unknown} statement
 * @param {unknown} model
 * @param {unknown} options
 */
export async function verify(run, statement, model, options) {
  const sql = new Held(statement).sql;
  const shape = shapeOf(model);
  /** @type {[string, string, string | null][]} */
  const described = await run(native.describeJob(sql), options ?? {});
  /** @type {Map<string, [string, string | null]>} */
  const columns = new Map();
  for (const [name, type, kind] of described) {
    if (!columns.has(name)) columns.set(name, [type, kind]);
  }
  for (const field of shape.fields) {
    const found = columns.get(field.sql);
    if (found === undefined) {
      const returned = described.map(([name]) => name).join(", ");
      throw new DecodeError(
        `\`${shape.name}\` reads column \`${field.sql}\`, which the statement does not return; it returns: ${returned}`,
      );
    }
    const [type, kind] = found;
    const key = columnKey(field.column);
    const accepted = kind === key || (readsText(field.column) && kind === (field.column.array ? "text[]" : "text"));
    if (!accepted) {
      throw new DecodeError(
        `\`${shape.name}\` decodes column \`${field.sql}\` as \`${key}\`, which cannot read PostgreSQL type \`${type}\``,
      );
    }
  }
}
