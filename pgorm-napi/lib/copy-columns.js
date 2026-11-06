// @ts-check

// The columns of a binary COPY and the format's two sides over them: a
// `CopyColumn` the server describes, of any type, or one a program names, of
// a built-in type; and pgorm's `CopyRowEncoder` and `CopyRowDecoder`, for a
// program that moves a COPY's bytes itself.
// [spec:pgorm:req:bindings.copy-rows+2]

import { native } from "./operations.js";
import { TypeName } from "./values.js";

/** Marks a column made from a handle a description gave. */
const DESCRIBED = Symbol("pgorm-napi described column");

/** @type {(column: CopyColumn) => unknown} */
let handleOf;

/**
 * One column of a COPY: its name, its type as a `TypeName` qualified by the
 * schema the server names it in — `pg_catalog` for a built-in type, an
 * array's being `_` and its element's name — and for a `money` column the
 * places its currency is written to.
 *
 * A program makes one of a built-in type, named as `pg_type` names it
 * (`"int4"`, `"varchar"`, `"_text"`) or as an element's name followed by
 * `[]` (`"text[]"`), or by a `TypeName` in `pg_catalog`. Any other type is a
 * `ConstructionError`: describe the COPY's columns to learn it. A `money`
 * column needs `moneyPlaces`, which no other column takes.
 */
export class CopyColumn {
  /** @type {unknown} */
  #native;
  /** @type {string} */
  #name;
  /** @type {TypeName} */
  #type;
  /** @type {number | null} */
  #moneyPlaces;

  /**
   * @param {unknown} name
   * @param {unknown} type
   * @param {unknown} [options]
   */
  constructor(name, type, options = {}) {
    if (name === DESCRIBED) {
      this.#native = type;
    } else {
      if (typeof name !== "string") throw new TypeError("a COPY column's name is a string");
      if (typeof type !== "string" && !(type instanceof TypeName)) {
        throw new TypeError("a COPY column's type is a built-in type's name or a TypeName");
      }
      this.#native = native.copyColumnNew(name, type, moneyPlaces(options));
    }
    const [described, typeName, places] = native.copyColumnParts(this.#native);
    this.#name = described;
    this.#type = typeName;
    this.#moneyPlaces = places;
  }

  static {
    handleOf = (column) => column.#native;
  }

  /** The column's name, unquoted, its case kept. */
  get name() {
    return this.#name;
  }

  /** @returns {TypeName} */
  get type() {
    return this.#type;
  }

  /** @returns {number | null} */
  get moneyPlaces() {
    return this.#moneyPlaces;
  }

  /**
   * Whether `other` is a column of the same name and type, counting `money`
   * in the same places.
   *
   * @param {unknown} other
   */
  equals(other) {
    return other instanceof CopyColumn && other.#name === this.#name && other.#type.name === this.#type.name &&
      other.#type.schema === this.#type.schema && other.#moneyPlaces === this.#moneyPlaces;
  }
}

/**
 * The places a `money` column's options give it, if any.
 *
 * @param {unknown} options
 * @returns {number | null}
 */
function moneyPlaces(options) {
  if (typeof options !== "object" || options === null) throw new TypeError("a COPY column's options are an object");
  for (const option of Object.keys(options)) {
    if (option !== "moneyPlaces") throw new TypeError(`${JSON.stringify(option)} is no COPY column option`);
  }
  const { moneyPlaces: places = null } = /** @type {{ moneyPlaces?: unknown }} */ (options);
  if (places === null) return null;
  if (typeof places !== "number" || !Number.isInteger(places) || places < 0 || places > 0xffff_ffff) {
    throw new TypeError("moneyPlaces is a whole number of places, from 0");
  }
  return places;
}

/**
 * The columns a description's handles are.
 *
 * @param {unknown[]} handles
 * @returns {readonly CopyColumn[]}
 */
export function describedColumns(handles) {
  return Object.freeze(handles.map((handle) => new CopyColumn(DESCRIBED, handle)));
}

/**
 * The handles of `columns`, checked to be a list of `CopyColumn`s.
 *
 * @param {unknown} columns
 * @param {string} what
 */
function columnHandles(columns, what) {
  if (!Array.isArray(columns) || !columns.every((column) => column instanceof CopyColumn)) {
    throw new TypeError(`a ${what} is made over a list of CopyColumns`);
  }
  return { columns: Object.freeze([...columns]), handles: columns.map(handleOf) };
}

/**
 * The binary format's writing side over a COPY's columns: the format's
 * `header()`, each row's `encode(row)` and the `trailer()`, each as bytes. A
 * row is an array of one value for each column, each read as a parameter is
 * and written against its column's type as a parameter is bound; a row of
 * another width, or a value its column's type does not take, is a
 * `ConstructionError` naming the column.
 */
export class CopyRowEncoder {
  /** @type {unknown} */
  #native;
  /** @type {readonly CopyColumn[]} */
  #columns;

  /** @param {unknown} columns */
  constructor(columns) {
    const checked = columnHandles(columns, "CopyRowEncoder");
    this.#columns = checked.columns;
    this.#native = native.copyEncoderNew(checked.handles);
  }

  /** The columns each row holds, in order. */
  get columns() {
    return this.#columns;
  }

  /**
   * The signature, flags and extension area that come once, before the first
   * row.
   *
   * @returns {Uint8Array}
   */
  static header() {
    return native.copyEncoderHeader();
  }

  /**
   * The field count of -1 that comes once, after the last row.
   *
   * @returns {Uint8Array}
   */
  static trailer() {
    return native.copyEncoderTrailer();
  }

  /**
   * One row in the binary format.
   *
   * @param {unknown} row
   * @returns {Uint8Array}
   */
  encode(row) {
    if (!Array.isArray(row)) throw new TypeError("a COPY row is an array of its columns' values");
    return native.copyEncoderEncode(this.#native, row);
  }
}

/**
 * The binary format's reading side over a COPY's columns: `push(chunk)` takes
 * the data in chunks split anywhere, `nextRow()` gives each row once all of
 * it has arrived, `null` until then and after the trailer, each field decoded
 * as `copyOutRows` decodes it, and `finish()` refuses data that ended part
 * way through a row or before the trailer. Data that is not the binary
 * format, and a field that does not decode, are a `DecodeError`.
 */
export class CopyRowDecoder {
  /** @type {unknown} */
  #native;
  /** @type {readonly CopyColumn[]} */
  #columns;

  /** @param {unknown} columns */
  constructor(columns) {
    const checked = columnHandles(columns, "CopyRowDecoder");
    this.#columns = checked.columns;
    this.#native = native.copyDecoderNew(checked.handles);
  }

  /** The columns each row holds, in order. */
  get columns() {
    return this.#columns;
  }

  /**
   * Take the next chunk of the data, copied.
   *
   * @param {unknown} chunk
   */
  push(chunk) {
    if (!(chunk instanceof Uint8Array)) throw new TypeError("a COPY's data is pushed as Uint8Array chunks");
    native.copyDecoderPush(this.#native, chunk);
  }

  /** @returns {unknown[] | null} */
  nextRow() {
    return native.copyDecoderNext(this.#native);
  }

  /** Check that the data, all of it pushed, ended with the trailer. */
  finish() {
    native.copyDecoderFinish(this.#native);
  }
}
