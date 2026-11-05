// @ts-check

// Relations between models: columns of one model's table equal to columns
// of another's, which a query joins by, a graph decodes along, and `find`
// and `load` read the far end of.
// [spec:pgorm:req:bindings.relations]

import { Condition } from "./conditions.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { tuple } from "./expressions.js";
import { columnValue } from "./model-columns.js";
import { query } from "./model-queries.js";
import { fieldOf, qualified, shapeOf } from "./model-shape.js";
import { registerRelation } from "./relation-parts.js";
import { Decimal, Interval, Uuid } from "./values.js";

export { relationOn, shapesOf } from "./relation-parts.js";

/** Marks a construction from a model's relation methods. */
const MADE = Symbol("pgorm-napi relation");

/**
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} names
 * @param {string} side
 */
function keyFields(shape, names, side) {
  const list = typeof names === "string" ? [names] : names;
  if (!Array.isArray(list)) throw new TypeError(`a relation's ${side} is a field or a list of fields`);
  if (list.length === 0) throw new ConstructionError(`a relation's ${side} names at least one field`);
  const fields = list.map((name) => fieldOf(shape, name));
  if (new Set(fields).size !== fields.length) throw new ConstructionError(`a relation's ${side} names each field once`);
  return fields;
}

/**
 * A relation from one model to another: its kind, and the fields of each
 * that are equal in a related pair of rows, matched in order. A
 * `belongsTo` or `hasOne` relation reads at most one row at its far end, a
 * `hasMany` any number.
 */
export class Relation {
  /** @type {"belongsTo" | "hasOne" | "hasMany"} */
  #kind;
  /** @type {import("./model-shape.js").Shape} */
  #from;
  /** @type {import("./model-shape.js").Shape} */
  #to;
  /** @type {import("./model-shape.js").Field[]} */
  #fromFields;
  /** @type {import("./model-shape.js").Field[]} */
  #toFields;

  /**
   * @param {"belongsTo" | "hasOne" | "hasMany"} kind
   * @param {unknown} from
   * @param {unknown} to
   * @param {unknown} keys
   * @param {symbol} token
   */
  constructor(kind, from, to, keys, token) {
    if (token !== MADE) throw new TypeError("a Relation comes from a model's belongsTo(), hasOne() or hasMany()");
    if (typeof keys !== "object" || keys === null) throw new TypeError("a relation's keys are { from, to }");
    const { from: fromNames, to: toNames } = /** @type {{ from?: unknown, to?: unknown }} */ (keys);
    this.#kind = kind;
    this.#from = shapeOf(from);
    this.#to = shapeOf(to);
    this.#fromFields = keyFields(this.#from, fromNames, "from");
    this.#toFields = keyFields(this.#to, toNames, "to");
    if (this.#fromFields.length !== this.#toFields.length) {
      throw new ConstructionError("a relation pairs as many fields at each end");
    }
    registerRelation(this, { from: this.#from, to: this.#to, fromFields: this.#fromFields, toFields: this.#toFields });
    Object.freeze(this);
  }

  get kind() {
    return this.#kind;
  }

  /** The model the relation starts at. */
  get from() {
    return this.#from.model;
  }

  /** The model at its far end. */
  get to() {
    return this.#to.model;
  }

  get fromFields() {
    return Object.freeze(this.#fromFields.map((field) => field.field));
  }

  get toFields() {
    return Object.freeze(this.#toFields.map((field) => field.field));
  }

  /** Whether a row reads at most one row at the far end, or any number. */
  get cardinality() {
    return this.#kind === "hasMany" ? "many" : "one";
  }

  /**
   * The rows at the far end related to `row`, a record of the model the
   * relation starts at: none when a key field of it is null.
   *
   * @param {unknown} row
   */
  find(row) {
    const values = keyValues(this.#fromFields, row);
    const found = query(this.#to, this.#to.fields);
    if (values === null) return found.where(Condition.any());
    return found.where(Condition.all(...this.#toFields.map((field, index) =>
      qualified(field, this.#to.qualifier).eq(values[index])
    )));
  }

  /**
   * The rows at the far end of each of `rows`, read in one query: for a
   * `hasMany` relation a list per row; otherwise a record per row, or null
   * where the row's key holds a NULL. A key no row at the far end matches,
   * or a `hasOne` key two match, is a `DecodeError`, never a missing row.
   *
   * @param {unknown} db
   * @param {readonly unknown[]} rows
   * @param {{ signal?: AbortSignal }} [options]
   */
  async load(db, rows, options) {
    if (!Array.isArray(rows)) throw new TypeError("load takes an array of records");
    const keys = rows.map((row) => keyValues(this.#fromFields, row));
    /** @type {Map<string, unknown[]>} */
    const wanted = new Map();
    for (const key of keys) if (key !== null) wanted.set(keyText(key), key);
    /** @type {Map<string, Record<string, unknown>[]>} */
    const found = new Map();
    if (wanted.size > 0) {
      for (const record of await query(this.#to, this.#to.fields).where(this.#among([...wanted.values()])).all(db, options)) {
        const text = keyText(this.#toFields.map((field) => record[field.field]));
        const list = found.get(text);
        if (list) list.push(record);
        else found.set(text, [record]);
      }
    }
    return keys.map((key) => {
      if (this.#kind === "hasMany") return key === null ? [] : found.get(keyText(key)) ?? [];
      if (key === null) return null;
      const matched = found.get(keyText(key)) ?? [];
      if (matched.length === 0) throw new DecodeError(`no ${this.#to.name} row has the key ${keyText(key)}`);
      if (matched.length > 1) throw new DecodeError(`${matched.length} ${this.#to.name} rows have the key ${keyText(key)}`);
      return matched[0];
    });
  }

  /**
   * The far end's key among `keys`: an IN list of one column, or of a row of
   * the key's columns.
   *
   * @param {unknown[][]} keys
   */
  #among(keys) {
    const columns = this.#toFields.map((field) => qualified(field, this.#to.qualifier));
    const bound = keys.map((key) => key.map((value, index) => {
      const field = /** @type {import("./model-shape.js").Field} */ (this.#toFields[index]);
      return columnValue(field.column, field.field, value);
    }));
    if (columns.length === 1) return /** @type {import("./model-columns.js").ModelColumn} */ (columns[0]).isIn(bound.map((key) => key[0]));
    return tuple(...columns).isIn(bound.map((key) => tuple(...key)));
  }
}

/**
 * The values of `fields` in `row`, or null when one of them is null.
 *
 * @param {readonly import("./model-shape.js").Field[]} fields
 * @param {unknown} row
 * @returns {unknown[] | null}
 */
function keyValues(fields, row) {
  if (typeof row !== "object" || row === null) throw new TypeError("a relation reads a record of the model it starts at");
  const values = fields.map((field) => {
    if (!Object.hasOwn(row, field.field)) throw new ConstructionError(`the record has no ${field.field}`);
    return /** @type {Record<string, unknown>} */ (row)[field.field];
  });
  return values.some((value) => value === null) ? null : values;
}

/**
 * A key as text two equal keys share: integers whatever their JavaScript
 * type, and the module's values and Temporal's by their text.
 *
 * @param {readonly unknown[]} key
 */
export function keyText(key) {
  return JSON.stringify(key.map((value) => {
    if (typeof value === "bigint" || typeof value === "number") return `n:${value}`;
    if (typeof value === "string") return `s:${value}`;
    if (value instanceof Uint8Array) return `b:${Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
    if (value instanceof Decimal || value instanceof Uuid || value instanceof Interval) return `v:${value}`;
    return `o:${String(value)}`;
  }));
}

/**
 * A relation of `kind` from `from` to `to`.
 *
 * @param {"belongsTo" | "hasOne" | "hasMany"} kind
 * @param {object} from
 * @param {unknown} to
 * @param {unknown} keys
 */
export function relation(kind, from, to, keys) {
  return new Relation(kind, from, to, keys, MADE);
}
