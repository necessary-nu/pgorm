// @ts-check

// A registered entity's writes that read each written row's two versions,
// reached as the statement terminals they are in Rust, no
// `ActiveModelBehavior` hook running around them.
// [spec:pgorm:req:bindings.entity-versions+3]

import { arg } from "./builder.js";
import { Condition } from "./conditions.js";
import { runJob } from "./runs.js";
import { predicateOf, recordOf } from "./entities.js";
import { ConstructionError } from "./errors.js";
import { Expr } from "./expressions.js";
import { native } from "./operations.js";

/** Marks a construction from a registered entity. */
export const VERSIONS = Symbol("pgorm-napi entity versions");

/** What `allRows()` sets in place of a condition: every row, said outright. */
const ALL_ROWS = Symbol("every row");


/**
 * The versions of a job's rows: `[columns, [old | null, new][]]`.
 *
 * @param {string} entity
 * @param {[string[], [[unknown[], unknown] | null, [unknown[], unknown]][]]} outcome
 */
export function versionsOf(entity, [columns, rows]) {
  return rows.map(([old, now]) => [old === null ? null : recordOf(entity, columns, old), recordOf(entity, columns, now)]);
}

/** An update of one ActiveModel's row by its key. */
export class EntityUpdate {
  /** @type {unknown} */
  #entity;
  /** @type {string} */
  #name;
  /** @type {unknown} */
  #active;

  /**
   * @param {unknown} entity
   * @param {string} name
   * @param {unknown} active
   * @param {symbol} token
   */
  constructor(entity, name, active, token) {
    if (token !== VERSIONS) throw new TypeError("an EntityUpdate comes from an entity's update()");
    this.#entity = entity;
    this.#name = name;
    this.#active = active;
  }

  /**
   * The row before and after, by `UpdateOne::exec_returning_change`, no
   * hook running around it; a key matching no row is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningChange(db, options) {
    const outcome = await runJob(db, native.entityChangeJob(this.#entity, this.#active), options);
    const [[old, now]] = versionsOf(this.#name, outcome);
    return Object.freeze({ old, new: now });
  }
}

/** An update of every row a condition admits, its columns set by `UpdateMany::col_expr`. */
export class EntityUpdateMany {
  /** @type {unknown} */
  #entity;
  /** @type {string} */
  #name;
  /** @type {readonly [string, unknown][]} */
  #assignments;
  /** @type {Expr | Condition | typeof ALL_ROWS | null} */
  #predicate;

  /**
   * @param {unknown} entity
   * @param {string} name
   * @param {readonly [string, unknown][]} assignments
   * @param {Expr | Condition | typeof ALL_ROWS | null} predicate
   * @param {symbol} token
   */
  constructor(entity, name, assignments, predicate, token) {
    if (token !== VERSIONS) throw new TypeError("an EntityUpdateMany comes from an entity's updateMany()");
    this.#entity = entity;
    this.#name = name;
    this.#assignments = Object.freeze(assignments);
    this.#predicate = predicate;
  }

  /**
   * The column set to a value, converted to its declared kind and written
   * through its `save_as`, or to an expression as written.
   *
   * @param {string} column
   * @param {unknown} value
   */
  set(column, value) {
    if (typeof column !== "string") throw new TypeError("a column is named by a string");
    return new EntityUpdateMany(this.#entity, this.#name, [...this.#assignments, [column, value]], this.#predicate, VERSIONS);
  }

  /**
   * Rows that also satisfy `predicate`.
   *
   * @param {unknown} predicate
   */
  where(predicate) {
    const next = predicateOf(predicate);
    const combined = this.#predicate === null || this.#predicate === ALL_ROWS ? next : Condition.all(this.#predicate, next);
    return new EntityUpdateMany(this.#entity, this.#name, this.#assignments, combined, VERSIONS);
  }

  /** Every row, said outright. */
  allRows() {
    return new EntityUpdateMany(this.#entity, this.#name, this.#assignments, ALL_ROWS, VERSIONS);
  }

  // [spec:pgorm:req:bindings.entity-versions+3]
  /**
   * Each row the update wrote, before and after, by
   * `UpdateMany::exec_returning_changes`; setting nothing is a
   * `ConstructionError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningChanges(db, options) {
    if (this.#predicate === null) {
      throw new ConstructionError("an update of many rows needs where(..) or allRows() before it runs");
    }
    const assignments = this.#assignments.map(([column, value]) => [column, arg(value)]);
    const predicate = this.#predicate === ALL_ROWS ? Condition.all() : this.#predicate;
    const job = native.entityChangesJob(this.#entity, assignments, arg(predicate));
    return versionsOf(this.#name, await runJob(db, job, options)).map(([old, now]) => Object.freeze({ old, new: now }));
  }
}
