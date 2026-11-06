// @ts-check

// The sink `copyInRows(statement)` gives with no rows: a binary COPY FROM a
// program writes a row at a time, each checked against the columns the
// server described before the COPY started, a row refused leaving nothing of
// it written and the sink still taking rows.
// [spec:pgorm:req:bindings.copy-rows+2]

import { describedColumns } from "./copy-columns.js";
import { write } from "./copy-feed.js";
import { ConstructionError, LifecycleError } from "./errors.js";
import { Expr } from "./expressions.js";
import { columnValue } from "./model-columns.js";
import { shapeOf } from "./model-shape.js";
import { native, TRUSTED as HANDLE } from "./operations.js";

/**
 * The row a record keyed by column name gives a COPY's columns: each column
 * the record's property of its name.
 *
 * @param {readonly import("./copy-columns.js").CopyColumn[]} columns
 * @param {Record<string, unknown>} record
 */
function byColumn(columns, record) {
  return columns.map(({ name }) => {
    if (!Object.hasOwn(record, name)) {
      throw new ConstructionError(`the record has no ${JSON.stringify(name)}, a column of the COPY`);
    }
    return record[name];
  });
}

/**
 * The row a model's record gives a COPY's columns: each column the record's
 * field whose column it is, read in the field's kind as a model's insert
 * reads it.
 *
 * @param {readonly import("./copy-columns.js").CopyColumn[]} columns
 * @param {Record<string, unknown>} record
 * @param {import("./model-shape.js").Shape} shape
 */
function byField(columns, record, shape) {
  return columns.map(({ name }) => {
    const field = shape.fields.find((candidate) => candidate.sql === name);
    if (field === undefined) {
      throw new ConstructionError(`no field of ${shape.name} reads the COPY's column ${JSON.stringify(name)}`);
    }
    if (!Object.hasOwn(record, field.field)) {
      throw new ConstructionError(`the record has no ${field.field}, the COPY's column ${JSON.stringify(name)}`);
    }
    const value = columnValue(field.column, field.field, record[field.field]);
    if (value instanceof Expr) throw new ConstructionError(`${field.field} is an expression, and a COPY writes values`);
    return value;
  });
}

/**
 * A binary `COPY … FROM STDIN` a program writes a row at a time, holding
 * what it runs on until it ends. `send(row)` writes one row, an array of a
 * value for each of `columns` in order, each read as a parameter is and
 * written against its column's type as a parameter is bound; `sendModel`
 * writes a record. A row the columns refuse is a `ConstructionError` before
 * anything of it is queued, and the sink goes on taking rows. Rows are sent
 * in the order they are given, the sender held back while the connection is
 * busy. `finish()` completes the COPY and resolves with the rows written;
 * `close()`, `await using` and the signal the COPY was opened with abandon an
 * unfinished one, so nothing is written.
 */
export class CopyRowSink {
  /** @type {unknown} */
  #native;
  /**
   * What the COPY runs on, kept reachable while the sink is: a transaction
   * collected under its COPY would end it.
   *
   * @type {unknown}
   */
  #owner;
  /** @type {AbortSignal | undefined} */
  #signal;
  /** @type {(() => void) | undefined} */
  #onAbort;
  /** @type {readonly import("./copy-columns.js").CopyColumn[]} */
  #columns;
  /**
   * The last row queued, settled either way, which the next waits behind so
   * that rows reach the COPY in the order they were sent.
   *
   * @type {Promise<void>}
   */
  #queued = Promise.resolve();
  #ended = false;

  /**
   * @param {unknown} handle
   * @param {unknown} owner
   * @param {AbortSignal | undefined} signal
   * @param {unknown} token
   * @param {symbol} trusted
   */
  constructor(handle, owner, signal, token, trusted) {
    if (trusted !== HANDLE) throw new TypeError("a CopyRowSink comes from copyInRows(statement)");
    this.#native = handle;
    this.#owner = owner;
    this.#signal = signal;
    this.#columns = describedColumns(native.copySinkColumns(handle));
    if (signal !== undefined) {
      this.#onAbort = () => native.abort(token);
      signal.addEventListener("abort", this.#onAbort, { once: true });
    }
  }

  /** The columns each row supplies, in order, as the server describes them. */
  get columns() {
    return this.#columns;
  }

  /** @returns {boolean} */
  get closed() {
    return this.#ended || native.copyInClosed(this.#native);
  }

  /**
   * Write one row, a value for each column in order.
   *
   * @param {unknown} row
   * @returns {Promise<void>}
   */
  async send(row) {
    if (!Array.isArray(row)) throw new TypeError("a COPY row is an array of its columns' values");
    if (this.#ended) throw new LifecycleError("the COPY has ended");
    const encoded = native.copyRowsEncode(this.#native, [row]);
    const queued = this.#queued.then(() => write(this.#native, encoded));
    this.#queued = queued.then(() => {}, () => {});
    try {
      await queued;
    } catch (error) {
      throw this.#reason(error);
    }
  }

  /**
   * Write a record as one row: each of the COPY's columns from the record's
   * field whose column it is, a field of `model`, or, with no model, from its
   * property of the column's name, as a row object a query gives is keyed. A
   * column no field reads is a `ConstructionError`.
   *
   * @param {unknown} record
   * @param {unknown} [model]
   * @returns {Promise<void>}
   */
  async sendModel(record, model) {
    if (typeof record !== "object" || record === null) throw new TypeError("a record is an object of its fields");
    const fields = /** @type {Record<string, unknown>} */ (record);
    const row = model === undefined ? byColumn(this.#columns, fields) : byField(this.#columns, fields, shapeOf(model));
    await this.send(row);
  }

  /**
   * Write the format's trailer, complete the COPY once every row sent has
   * been queued, and resolve with the rows it wrote, once the connection is
   * free again. A row the server refused is reported here.
   *
   * @returns {Promise<number>}
   */
  async finish() {
    if (this.#ended) throw new LifecycleError("the COPY has ended");
    this.#ended = true;
    try {
      await this.#queued;
      return await native.copyInFinish(this.#native);
    } catch (error) {
      throw this.#reason(error);
    } finally {
      this.#release();
    }
  }

  /**
   * Abandon the COPY unless it has finished, so that nothing is written, and
   * resolve once what it ran on is free again: inside a transaction, the
   * transaction is aborted, as by any failed statement.
   *
   * @returns {Promise<void>}
   */
  async close() {
    if (this.#ended) return;
    this.#ended = true;
    try {
      await native.copyInAbort(this.#native);
    } finally {
      this.#release();
    }
  }

  [Symbol.asyncDispose]() {
    return this.close();
  }

  /**
   * What a failure is reported as: the signal's reason once it has fired,
   * which abandoned the COPY.
   *
   * @param {unknown} error
   */
  #reason(error) {
    return this.#signal?.aborted ? this.#signal.reason : error;
  }

  #release() {
    if (this.#onAbort !== undefined) this.#signal?.removeEventListener("abort", this.#onAbort);
    this.#onAbort = undefined;
    this.#owner = undefined;
  }
}
