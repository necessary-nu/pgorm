// @ts-check

// What a model's writes read back: the RETURNING list of a write's fields,
// whose terminals decode each returned row into a record, and the list of a
// written row's two versions, as pgorm's `exec_returning_change(s)` and
// `exec_returning_upsert(s)` read them.
// [spec:pgorm:req:bindings.model-writes+3]

import { ConstructionError, DecodeError } from "./errors.js";
import { col } from "./expressions.js";
import { decodeAll, decodeOne, decodeOptional } from "./model-decode.js";
import { fieldOf, projected } from "./model-shape.js";
import { native } from "./operations.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi model rows");

/**
 * The names a RETURNING list reads a written row's versions by: always
 * renamed, as pgorm renames them, so a table called `old` or `new` cannot
 * take the keyword.
 */
export const OLD = "pgorm_old";
export const NEW = "pgorm_new";

/**
 * The RETURNING items reading `fields` as the records decode them, qualified
 * by `table` where the write reads another table whose columns may share
 * their names.
 *
 * @param {readonly import("./model-shape.js").Field[]} fields
 * @param {string} [table]
 */
export function returned(fields, table) {
  return fields.map((field) => projected(field, col(field.sql, table === undefined ? undefined : { table }), field.sql));
}

/**
 * The RETURNING list of both versions of every field, the old under `o_` and
 * the new under `n_`, and the sources they decode as.
 *
 * @param {import("./model-shape.js").Shape} shape
 */
export function versions(shape) {
  /** @type {import("./model-decode.js").Source[]} */
  const sources = [];
  const items = [[OLD, "o_"], [NEW, "n_"]].flatMap(([row, prefix]) => {
    const names = shape.fields.map((field) => native.modelResultName(prefix, field.sql));
    sources.push({ fields: shape.fields, names, optional: row === OLD });
    return shape.fields.map((field, index) => projected(field, col(field.sql, { table: row }), /** @type {string} */ (names[index])));
  });
  return { items, sources };
}

/**
 * @param {unknown} pair
 */
export function change(pair) {
  const [old, now] = /** @type {[unknown, unknown]} */ (pair);
  if (old === null) throw new DecodeError("an updated row returned no old version");
  return Object.freeze({ old, new: now });
}

/**
 * @param {unknown} pair
 */
export function upserted(pair) {
  const [old, now] = /** @type {[unknown, unknown]} */ (pair);
  return Object.freeze(old === null ? { kind: "inserted", new: now } : { kind: "updated", old, new: now });
}

/**
 * The fields `names` picks out of `shape`, every field when it names none.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {readonly unknown[]} names
 */
export function picked(shape, names) {
  const fields = names.length === 0 ? shape.fields : names.map((name) => fieldOf(shape, name));
  if (new Set(fields).size !== fields.length) throw new ConstructionError("a field is selected once");
  return fields;
}

/**
 * A write with a RETURNING list of a model's fields: its terminals decode
 * each returned row into a record.
 */
export class ModelRows {
  /** @type {import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | null} */
  #statement;
  /** @type {import("./model-decode.js").Source[]} */
  #sources;

  /**
   * @param {import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | null} statement
   * @param {readonly import("./model-shape.js").Field[]} fields
   * @param {symbol} token
   */
  constructor(statement, fields, token) {
    if (token !== MADE) throw new TypeError("ModelRows come from a model write's returning()");
    this.#statement = statement;
    this.#sources = [{ fields, names: fields.map((field) => field.sql), optional: false }];
  }

  get statement() {
    return written(this.#statement);
  }

  inspect() {
    return written(this.#statement).inspect();
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    if (this.#statement === null) return [];
    return await decodeAll(db, this.#statement, this.#sources, options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    if (this.#statement === null) throw new DecodeError("expected exactly one row, and the insert wrote none");
    return await decodeOne(db, this.#statement, this.#sources, options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    if (this.#statement === null) return null;
    return await decodeOptional(db, this.#statement, this.#sources, options);
  }
}

/**
 * @template T
 * @param {T | null} statement
 * @returns {T}
 */
export function written(statement) {
  if (statement === null) throw new ConstructionError("an insert of no rows writes nothing and has no statement");
  return statement;
}

/**
 * The rows `statement`'s RETURNING list of `fields` reads.
 *
 * @param {import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | null} statement
 * @param {readonly import("./model-shape.js").Field[]} fields
 */
export function rowsOf(statement, fields) {
  return new ModelRows(statement, fields, MADE);
}
