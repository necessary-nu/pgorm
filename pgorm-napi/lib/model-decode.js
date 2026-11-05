// @ts-check

// How a model reads a statement's result. Each source the statement
// projected — a model's own columns, a graph's slots, a write's two
// versions — finds its fields by the result-column names it was projected
// under; each column's kind is held to its field's declaration and each
// value to its nullability and listed values, a mismatch being a
// DecodeError rather than a value of the wrong type; and each row becomes a
// record keyed by field, a tuple of a graph's records, or null for an
// optional source its row does not carry. A field read from its text, a
// created multirange's, is held to the text its projection casts it to and
// read as the value that text spells.
// [spec:pgorm:req:bindings.model-records+1]

import { runStatement } from "./connections.js";
import { DecodeError } from "./errors.js";
import { columnKey, readsText } from "./model-columns.js";
import { native } from "./operations.js";
import { decodedWith } from "./streams.js";

/** The kind a field read from its text is projected as. */
const TEXT = "text";

/**
 * A source of a result: the fields it decodes, the result column each is
 * read from, and whether its row may be absent, as a LEFT JOIN's is.
 *
 * @typedef {object} Source
 * @property {readonly import("./model-shape.js").Field[]} fields
 * @property {readonly string[]} names
 * @property {boolean} optional
 */

/**
 * Where each source's fields are in a result, found by name, each column's
 * kind held to its field's: every result column read by exactly one field,
 * unless `exhaustive` is false, as it is for one kind's row of a disjoint
 * read, whose other kinds' columns that row leaves unread.
 *
 * @param {string[]} names
 * @param {string[]} kinds
 * @param {readonly Source[]} sources
 * @param {boolean} [exhaustive]
 * @returns {number[][]}
 */
function plan(names, kinds, sources, exhaustive = true) {
  const at = new Map(names.map((name, index) => [name, index]));
  let read = 0;
  const plans = sources.map((source) =>
    source.fields.map((field, position) => {
      const name = /** @type {string} */ (source.names[position]);
      const index = at.get(name);
      if (index === undefined) {
        throw new DecodeError(`the result has no column ${JSON.stringify(name)} for ${field.field}`);
      }
      const key = readsText(field.column) ? (field.column.array ? `${TEXT}[]` : TEXT) : columnKey(field.column);
      if (kinds[index] !== key) {
        throw new DecodeError(`column ${JSON.stringify(name)} is ${kinds[index]}, and ${field.field} declares ${key}`);
      }
      read += 1;
      return index;
    })
  );
  if (exhaustive && read !== names.length) {
    const unread = names.filter((name) => !sources.some((source) => source.names.includes(name)));
    throw new DecodeError(`the result has columns no field reads: ${unread.join(", ")}`);
  }
  return plans;
}

/**
 * One field's value, held to its declaration's nullability and values, and
 * read from its text where the field is.
 *
 * @param {import("./model-shape.js").Field} field
 * @param {unknown} value
 */
function checked(field, value) {
  const { column } = field;
  if (value === null) {
    if (!column.nullable) throw new DecodeError(`${field.field} is not nullable, and the row holds NULL`);
    return value;
  }
  if (readsText(column)) {
    if (!column.array) return native.modelReadText(column.kind, value);
    return /** @type {(string | null)[]} */ (value).map((item) => item === null ? null : native.modelReadText(column.kind, item));
  }
  if (column.values !== null) {
    const labels = column.values;
    const items = column.array ? /** @type {unknown[]} */ (value) : [value];
    for (const item of items) {
      if (item !== null && !labels.includes(/** @type {string} */ (item))) {
        throw new DecodeError(`${JSON.stringify(item)} is not one of ${field.field}'s values`);
      }
    }
  }
  return value;
}

/**
 * One source's record from a row, or null when the source is optional and
 * every column it reads is NULL: the row of a LEFT JOIN that matched
 * nothing, as pgorm's absence witness reads it.
 *
 * @param {Source} source
 * @param {number[]} indexes
 * @param {unknown[]} values
 */
function record(source, indexes, values) {
  if (source.optional && indexes.every((index) => values[index] === null)) return null;
  return Object.fromEntries(
    source.fields.map((field, position) => [field.field, checked(field, values[/** @type {number} */ (indexes[position])])]),
  );
}

/**
 * A decoder for a result of `sources`: one source's record, or a tuple of
 * each source's. It finds its columns on the first row it meets, every
 * column read by one of them unless `exhaustive` is false.
 *
 * @param {readonly Source[]} sources
 * @param {boolean} [exhaustive]
 * @returns {(names: string[], values: unknown[], kinds: string[]) => any}
 */
export function decoder(sources, exhaustive = true) {
  /** @type {number[][] | null} */
  let plans = null;
  return (names, values, kinds) => {
    plans ??= plan(names, kinds, sources, exhaustive);
    const found = plans;
    const records = sources.map((source, index) => record(source, /** @type {number[]} */ (found[index]), values));
    return records.length === 1 ? records[0] : records;
  };
}

/**
 * Every row of `statement`, decoded.
 *
 * @param {unknown} db
 * @param {import("./builder.js").Handle} statement
 * @param {readonly Source[]} sources
 * @param {unknown} options
 * @returns {Promise<any[]>}
 */
export async function decodeAll(db, statement, sources, options) {
  const [names, rows, kinds] = await runStatement(db, "all", statement, options);
  const decode = decoder(sources);
  return rows.map((/** @type {unknown[]} */ values) => decode(names, values, kinds));
}

/**
 * Exactly one row of `statement`, decoded; any other count is a
 * `DecodeError`.
 *
 * @param {unknown} db
 * @param {import("./builder.js").Handle} statement
 * @param {readonly Source[]} sources
 * @param {unknown} options
 */
export async function decodeOne(db, statement, sources, options) {
  const [names, rows, kinds] = await runStatement(db, "one", statement, options);
  return decoder(sources)(names, rows[0], kinds);
}

/**
 * At most one row of `statement`, decoded, or null.
 *
 * @param {unknown} db
 * @param {import("./builder.js").Handle} statement
 * @param {readonly Source[]} sources
 * @param {unknown} options
 */
export async function decodeOptional(db, statement, sources, options) {
  const [names, rows, kinds] = await runStatement(db, "optional", statement, options);
  return rows.length === 0 ? null : decoder(sources)(names, rows[0], kinds);
}

/**
 * The rows of `statement` as a stream of decoded items, over a pool's or a
 * connection's connection.
 *
 * @param {unknown} db
 * @param {import("./builder.js").Handle} statement
 * @param {readonly Source[]} sources
 * @param {{ signal?: AbortSignal }} [options]
 */
export function decodeStream(db, statement, sources, options = {}) {
  const stream = /** @type {{ stream?: unknown }} */ (db)?.stream;
  if (typeof stream !== "function") throw new TypeError("a model's rows stream from a Pool or a Connection");
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  return decodedWith(stream.call(db, statement, { signal: options.signal }), decoder(sources));
}

/**
 * A count a statement read as `num_items`, as an exact number.
 *
 * @param {unknown} db
 * @param {import("./builder.js").Handle} statement
 * @param {unknown} options
 */
export async function decodeCount(db, statement, options) {
  const [, rows] = await runStatement(db, "one", statement, options);
  const count = /** @type {bigint} */ (rows[0][0]);
  if (count > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new DecodeError(`${count} rows is past the integers a number holds exactly`);
  }
  return Number(count);
}
