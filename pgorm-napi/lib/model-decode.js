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
// [spec:pgorm:req:bindings.model-records+2]

import { runStatement } from "./runs.js";
import { DecodeError } from "./errors.js";
import { columnKey, readsText, readValue } from "./model-columns.js";
import { native } from "./operations.js";
import { CreatedMultirange } from "./values.js";
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
 * How a decoder reads its result: whether every result column is read by
 * exactly one field, as it is unless the result holds columns a source
 * leaves unread — one kind's row of a disjoint read, whose other kinds'
 * columns that row leaves unread, or a COPY's row, whose columns no field of
 * the model names — and whether a field read from its text was projected
 * cast to text (`projected`), as a statement projects it and a COPY, whose
 * binary rows carry each column in its own type, does not.
 *
 * @typedef {{ exhaustive?: boolean, castsText?: boolean }} Reading
 */

/**
 * Where each source's fields are in a result, found by name, each column's
 * kind held to its field's.
 *
 * @param {readonly string[]} names
 * @param {string[]} kinds
 * @param {readonly Source[]} sources
 * @param {Required<Reading>} reading
 * @returns {number[][]}
 */
function plan(names, kinds, sources, { exhaustive, castsText }) {
  const at = new Map(names.map((name, index) => [name, index]));
  let read = 0;
  const plans = sources.map((source) =>
    source.fields.map((field, position) => {
      const name = /** @type {string} */ (source.names[position]);
      const index = at.get(name);
      if (index === undefined) {
        throw new DecodeError(`the result has no column ${JSON.stringify(name)} for ${field.field}`);
      }
      const key = castsText && readsText(field.column) ? (field.column.array ? `${TEXT}[]` : TEXT) : columnKey(field.column);
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
 * Whether `field`'s value is read from the text a result cast it to, as a
 * created multirange's is.
 *
 * @param {import("./model-shape.js").Field} field
 * @param {boolean} castsText
 */
function fromText(field, castsText) {
  return castsText && readsText(field.column) && field.column.kind instanceof CreatedMultirange;
}

/**
 * One field's value, held to its declaration's nullability and values, and
 * read from its text where `textual` says the result was cast to it.
 *
 * @param {import("./model-shape.js").Field} field
 * @param {unknown} value
 * @param {boolean} textual
 */
function checked(field, value, textual) {
  const { column } = field;
  if (value === null) {
    if (!column.nullable) throw new DecodeError(`${field.field} is not nullable, and the row holds NULL`);
    return value;
  }
  if (textual) {
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
  return readValue(column, value);
}

/**
 * How a decoder reads one source: its fields, a template defining each, and
 * whether each is read from its text.
 *
 * @typedef {{ source: Source, template: Record<string, null>, textual: boolean[] }} Reader
 */

/**
 * One source's record from row `at` of a result's columns, or null when the
 * source is optional and every column it reads is NULL: the row of a LEFT
 * JOIN that matched nothing, as pgorm's absence witness reads it. Each record
 * is a copy of the reader's template, so that a field named `__proto__` is a
 * property rather than the record's prototype.
 *
 * @param {Reader} reader
 * @param {number[]} indexes
 * @param {readonly unknown[][]} columns
 * @param {number} at
 */
function record({ source, template, textual }, indexes, columns, at) {
  if (source.optional && indexes.every((index) => /** @type {unknown[]} */ (columns[index])[at] === null)) return null;
  /** @type {Record<string, unknown>} */
  const made = { ...template };
  const { fields } = source;
  for (let position = 0; position < fields.length; position++) {
    const field = /** @type {import("./model-shape.js").Field} */ (fields[position]);
    const column = /** @type {unknown[]} */ (columns[/** @type {number} */ (indexes[position])]);
    made[field.field] = checked(field, column[at], /** @type {boolean} */ (textual[position]));
  }
  return made;
}

/**
 * A decoder for a result of `sources`, given its columns of values and a
 * row: one source's record, or a tuple of each source's. It finds its columns
 * on the first row it meets, read as `reading` says, every column read by one
 * of them and fields read from their text cast to it unless it says
 * otherwise.
 *
 * @param {readonly Source[]} sources
 * @param {Reading} [reading]
 * @returns {(names: readonly string[], columns: readonly unknown[][], kinds: string[], at: number) => any}
 */
export function decoder(sources, { exhaustive = true, castsText = true } = {}) {
  /** @type {number[][] | null} */
  let plans = null;
  /** @type {Reader[]} */
  const readers = sources.map((source) => ({
    source,
    template: Object.fromEntries(source.fields.map((field) => [field.field, null])),
    textual: source.fields.map((field) => fromText(field, castsText)),
  }));
  return (names, columns, kinds, at) => {
    plans ??= plan(names, kinds, sources, { exhaustive, castsText });
    const found = plans;
    if (readers.length === 1) return record(/** @type {Reader} */ (readers[0]), /** @type {number[]} */ (found[0]), columns, at);
    return readers.map((reader, index) => record(reader, /** @type {number[]} */ (found[index]), columns, at));
  };
}

/**
 * A decoder as `decoder` makes one, given one row's values rather than a
 * result's columns: a streamed row, a COPY's or a disjoint read's.
 *
 * @param {readonly Source[]} sources
 * @param {Reading} [reading]
 * @returns {(names: readonly string[], values: readonly unknown[], kinds: string[]) => any}
 */
export function rowDecoder(sources, reading) {
  const decode = decoder(sources, reading);
  return (names, values, kinds) => decode(names, values.map((value) => [value]), kinds, 0);
}

/**
 * Every row of `statement`, decoded.
 *
 * @param {unknown} db
 * @param {unknown} statement A builder's, or SQL text a raw read holds.
 * @param {readonly Source[]} sources
 * @param {unknown} options
 * @param {Reading} [reading] How the result is read: by default every column a field's, cast as a built statement casts it.
 * @returns {Promise<any[]>}
 */
export async function decodeAll(db, statement, sources, options, reading) {
  const [names, columns, kinds, count] = await runStatement(db, "all", statement, options);
  const decode = decoder(sources, reading);
  const decoded = [];
  for (let at = 0; at < count; at++) decoded.push(decode(names, columns, kinds, at));
  return decoded;
}

/**
 * Exactly one row of `statement`, decoded; any other count is a
 * `DecodeError`.
 *
 * @param {unknown} db
 * @param {unknown} statement
 * @param {readonly Source[]} sources
 * @param {unknown} options
 * @param {Reading} [reading]
 */
export async function decodeOne(db, statement, sources, options, reading) {
  const [names, columns, kinds] = await runStatement(db, "one", statement, options);
  return decoder(sources, reading)(names, columns, kinds, 0);
}

/**
 * At most one row of `statement`, decoded, or null.
 *
 * @param {unknown} db
 * @param {unknown} statement
 * @param {readonly Source[]} sources
 * @param {unknown} options
 * @param {Reading} [reading]
 */
export async function decodeOptional(db, statement, sources, options, reading) {
  const [names, columns, kinds, count] = await runStatement(db, "optional", statement, options);
  return count === 0 ? null : decoder(sources, reading)(names, columns, kinds, 0);
}

/**
 * The rows of `statement` as a stream of decoded items, over a pool's or a
 * connection's connection, or in a transaction, each made what `read` makes it where one is given.
 *
 * @param {unknown} db
 * @param {unknown} statement
 * @param {readonly Source[]} sources
 * @param {{ signal?: AbortSignal }} [options]
 * @param {Reading} [reading]
 * @param {((record: any) => unknown) | null} [read]
 */
export function decodeStream(db, statement, sources, options = {}, reading = undefined, read = null) {
  const stream = /** @type {{ stream?: unknown }} */ (db)?.stream;
  if (typeof stream !== "function") {
    throw new TypeError("a model's rows stream from a Pool, a Connection or a Transaction");
  }
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  const decode = rowDecoder(sources, reading);
  const made = read === null ? decode : (/** @type {string[]} */ names, /** @type {unknown[]} */ values, /** @type {string[]} */ kinds) =>
    read(decode(names, values, kinds));
  return decodedWith(stream.call(db, statement, { signal: options.signal }), made);
}

/**
 * A count a statement read as `num_items`, as an exact number.
 *
 * @param {unknown} db
 * @param {unknown} statement
 * @param {unknown} options
 */
export async function decodeCount(db, statement, options) {
  const [, columns] = await runStatement(db, "one", statement, options);
  const count = /** @type {bigint} */ (columns[0][0]);
  if (count > BigInt(Number.MAX_SAFE_INTEGER)) {
    throw new DecodeError(`${count} rows is past the integers a number holds exactly`);
  }
  return Number(count);
}
