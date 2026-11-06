// @ts-check

// A registered entity's try-insert, Rust's `TryInsert`, made by an insert's
// `onConflictDoNothing()` or `onEmptyDoNothing()`, whose terminals answer as
// `TryInsertResult` does. They are statement terminals, as in Rust, so no
// `ActiveModelBehavior` hook runs around them.

import { VERSIONS } from "./entity-versions.js";

/** @typedef {import("./entity-inserts.js").Inserted} Inserted */

// [spec:pgorm:req:bindings.entity-try-inserts]
/**
 * An entity's insert of one row that may write nothing, made by its
 * insert's `onConflictDoNothing()` or `onEmptyDoNothing()`: Rust's
 * `TryInsert<A, OneRow>`, whose terminals answer as `TryInsertResult` does.
 */
export class EntityTryInsert {
  /** @type {Inserted} */
  #inserted;

  /**
   * @param {Inserted} inserted
   * @param {symbol} token
   */
  constructor(inserted, token) {
    if (token !== VERSIONS) throw new TypeError("an EntityTryInsert comes from an insert's onConflictDoNothing() or onEmptyDoNothing()");
    this.#inserted = inserted;
  }

  /**
   * The conflict clause in place of the one it had, as `TryInsert::on_conflict` sets it.
   *
   * @param {unknown} action
   */
  onConflict(action) {
    return new EntityTryInsert(this.#inserted.withConflict(action), VERSIONS);
  }

  /**
   * The try-insert of its ActiveModel and then `active`, its conflict clause
   * kept, as `TryInsert::add` makes it.
   *
   * @param {unknown} active
   */
  add(active) {
    return new EntityTryInsertMany(this.#inserted.added([active]), VERSIONS);
  }

  /**
   * The try-insert of its ActiveModel and then `actives`, its conflict clause
   * kept, as `TryInsert::add_many` makes it.
   *
   * @param {readonly unknown[]} actives
   */
  addMany(actives) {
    return new EntityTryInsertMany(this.#inserted.added(actives), VERSIONS);
  }

  /**
   * The rows written, by `TryInsert::exec`: `conflicted` when its conflict
   * clause wrote none, `empty` when its ActiveModel sets nothing.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    return await this.#inserted.attempt(true, "count", (count) => count, db, options);
  }

  /**
   * The written row's key, by `TryInsert::exec_returning_pk`, or
   * `conflicted` or `empty`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningPk(db, options) {
    return await this.#inserted.attempt(true, "keys", (keys) => this.#inserted.keysOf(keys)[0], db, options);
  }

  /**
   * The written row's record, by `TryInsert::exec_returning_model`, or
   * `conflicted` or `empty`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningModel(db, options) {
    return await this.#inserted.attempt(true, "models", (rows) => this.#inserted.modelsOf(rows)[0], db, options);
  }
}

// [spec:pgorm:req:bindings.entity-try-inserts]
/**
 * An entity's insert of a batch that may write nothing, made by a batch's
 * `onConflictDoNothing()` or `onEmptyDoNothing()`, or a try-insert's `add`:
 * Rust's `TryInsert<A, ManyRows>`, whose terminals answer as
 * `TryInsertResult` does.
 */
export class EntityTryInsertMany {
  /** @type {Inserted} */
  #inserted;

  /**
   * @param {Inserted} inserted
   * @param {symbol} token
   */
  constructor(inserted, token) {
    if (token !== VERSIONS) throw new TypeError("an EntityTryInsertMany comes from an insert's onConflictDoNothing() or onEmptyDoNothing()");
    this.#inserted = inserted;
  }

  /**
   * The conflict clause in place of the one it had, as `TryInsert::on_conflict` sets it.
   *
   * @param {unknown} action
   */
  onConflict(action) {
    return new EntityTryInsertMany(this.#inserted.withConflict(action), VERSIONS);
  }

  /**
   * The try-insert with `active` after its own rows, its conflict clause
   * kept, as `TryInsert::add` adds it.
   *
   * @param {unknown} active
   */
  add(active) {
    return new EntityTryInsertMany(this.#inserted.added([active]), VERSIONS);
  }

  /**
   * The try-insert with `actives` after its own rows, its conflict clause
   * kept, as `TryInsert::add_many` adds them.
   *
   * @param {readonly unknown[]} actives
   */
  addMany(actives) {
    return new EntityTryInsertMany(this.#inserted.added(actives), VERSIONS);
  }

  /**
   * The rows written, by `TryInsert::exec`: `conflicted` when its conflict
   * clause wrote none of them, `empty` for a batch of none.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    return await this.#inserted.attempt(false, "count", (count) => count, db, options);
  }

  /**
   * Each written row's key in the order the database wrote them, by
   * `TryInsert::exec_returning_pks`: `conflicted` when its conflict clause
   * held every row back, `empty` for a batch of none.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningPks(db, options) {
    return await this.#inserted.attempt(false, "keys", (keys) => this.#inserted.keysOf(keys), db, options);
  }

  /**
   * Each written row's record in the order the database wrote them, by
   * `TryInsert::exec_returning_models`: `conflicted` when its conflict
   * clause held every row back, `empty` for a batch of none.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningModels(db, options) {
    return await this.#inserted.attempt(false, "models", (rows) => this.#inserted.modelsOf(rows), db, options);
  }
}
