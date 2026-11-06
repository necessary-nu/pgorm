// @ts-check

// A model's try-inserts: an INSERT that may write nothing, as pgorm's
// `TryInsert` is, whose terminals say what it did as `TryInsertResult` does:
// `inserted` with what it wrote, `conflicted` when its conflict clause skipped
// every row, or `empty` when it had nothing to write and sent nothing.

import { DecodeError } from "./errors.js";
import { attempt } from "./model-writes.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi model try-insert");

/** @typedef {import("./model-writes.js").ModelInsert} ModelInsert */
/** @typedef {import("./model-writes.js").ModelInsertMany} ModelInsertMany */
/** @typedef {import("./model-rows.js").ModelRows} ModelRows */

/** The answers of pgorm's `TryInsertResult`. */
const EMPTY = Object.freeze({ kind: "empty" });
const CONFLICTED = Object.freeze({ kind: "conflicted" });

/** @param {unknown} value */
function inserted(value) {
  return Object.freeze({ kind: "inserted", value });
}

// [spec:pgorm:req:bindings.model-try-inserts]
/**
 * What a model's try-insert of one row and of a batch share: the insert it
 * runs, and the terminals both kinds have, each answering as pgorm's
 * `TryInsertResult` does.
 *
 * @template {ModelInsert | ModelInsertMany} I
 */
class Trying {
  /** @type {I} */
  #insert;

  /**
   * @param {I} insert
   * @param {symbol} token
   */
  constructor(insert, token) {
    if (token !== MADE) throw new TypeError(`a ${new.target.name} comes from an insert's onConflictDoNothing() or onEmptyDoNothing()`);
    this.#insert = insert;
  }

  /** The insert it runs. */
  get insert() {
    return this.#insert;
  }

  get statement() {
    return this.#insert.statement;
  }

  inspect() {
    return this.#insert.inspect();
  }

  /**
   * @param {unknown} action
   * @returns {this}
   */
  onConflict(action) {
    const Kind = /** @type {new (insert: I, token: symbol) => this} */ (this.constructor);
    return new Kind(/** @type {I} */ (this.#insert.onConflict(action)), MADE);
  }

  /** @param {unknown} values */
  add(values) {
    return new ModelTryInsertMany(this.#insert.add(values), MADE);
  }

  /** @param {unknown} rows */
  addMany(rows) {
    return new ModelTryInsertMany(this.#insert.addMany(rows), MADE);
  }

  /**
   * The rows written, as pgorm's `TryInsert::exec` answers: `conflicted`
   * when a conflict clause wrote none of them, `empty` for nothing to write.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    const { empty, conflict } = attempt(this.#insert);
    if (empty) return EMPTY;
    const count = await this.#insert.execute(db, options);
    return count === 0 && conflict ? CONFLICTED : inserted(count);
  }
}

/**
 * The RETURNING list of `fields` over `trying`'s insert, and what its
 * terminal needs to answer as `TryInsertResult` does.
 *
 * @param {Trying<ModelInsert | ModelInsertMany>} trying
 * @param {readonly string[]} fields
 * @returns {Returning}
 */
function returningOf(trying, fields) {
  const { insert } = trying;
  return { rows: insert.returning(...fields), ...attempt(insert) };
}

/**
 * A model's INSERT of one row that may write nothing: Rust's
 * `TryInsert<A, OneRow>`.
 *
 * @extends {Trying<ModelInsert>}
 */
export class ModelTryInsert extends Trying {
  /**
   * A RETURNING list of `fields`, every field when it names none, whose
   * `one` answers with the row written, as pgorm's `exec_returning_model`
   * does.
   *
   * @param {...string} fields
   */
  returning(...fields) {
    return new ModelTryRow(returningOf(this, fields), MADE);
  }
}

/**
 * A model's INSERT of a batch that may write nothing: Rust's
 * `TryInsert<A, ManyRows>`.
 *
 * @extends {Trying<ModelInsertMany>}
 */
export class ModelTryInsertMany extends Trying {
  /**
   * A RETURNING list of `fields`, every field when it names none, whose
   * `all` answers with the rows written, as pgorm's `exec_returning_models`
   * does.
   *
   * @param {...string} fields
   */
  returning(...fields) {
    return new ModelTryRows(returningOf(this, fields), MADE);
  }
}

/**
 * @typedef {{ rows: ModelRows, empty: boolean, conflict: boolean }} Returning
 */

/** A try-insert of one row with a RETURNING list. */
export class ModelTryRow {
  /** @type {Returning} */
  #returning;

  /**
   * @param {Returning} returning
   * @param {symbol} token
   */
  constructor(returning, token) {
    if (token !== MADE) throw new TypeError("a ModelTryRow comes from a try-insert's returning()");
    this.#returning = returning;
  }

  inspect() {
    return this.#returning.rows.inspect();
  }

  /**
   * The row written as a record, `conflicted` when the conflict clause held
   * it back, or `empty` when it sets no field; with no conflict clause, no
   * row is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    const { rows, empty: none, conflict } = this.#returning;
    if (none) return EMPTY;
    const record = await rows.optional(db, options);
    if (record !== null) return inserted(record);
    if (conflict) return CONFLICTED;
    throw new DecodeError("expected the inserted row, and the insert returned none");
  }
}

/** A try-insert of a batch with a RETURNING list. */
export class ModelTryRows {
  /** @type {Returning} */
  #returning;

  /**
   * @param {Returning} returning
   * @param {symbol} token
   */
  constructor(returning, token) {
    if (token !== MADE) throw new TypeError("a ModelTryRows comes from a try-insert's returning()");
    this.#returning = returning;
  }

  inspect() {
    return this.#returning.rows.inspect();
  }

  /**
   * The rows written as records, in the order the database wrote them,
   * `conflicted` when the conflict clause held every one back, or `empty`
   * for a batch of none.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    const { rows, empty: none, conflict } = this.#returning;
    if (none) return EMPTY;
    const records = await rows.all(db, options);
    return records.length === 0 && conflict ? CONFLICTED : inserted(records);
  }
}

/**
 * The try-insert of `insert`'s one row.
 *
 * @param {ModelInsert} insert
 */
export function tryOne(insert) {
  return new ModelTryInsert(insert, MADE);
}

/**
 * The try-insert of `insert`'s batch.
 *
 * @param {ModelInsertMany} insert
 */
export function tryMany(insert) {
  return new ModelTryInsertMany(insert, MADE);
}
