// @ts-check

// A registered entity's writes that read each written row's two versions,
// reached as the statement terminals they are in Rust, no
// `ActiveModelBehavior` hook running around them.
// [spec:pgorm:req:bindings.entity-versions+3]

import { arg } from "./builder.js";
import { Condition } from "./conditions.js";
import { Conflict, ConflictUpdate } from "./conflicts.js";
import { runJob } from "./connections.js";
import { ownActive, predicateOf, recordOf } from "./entities.js";
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
function versionsOf(entity, [columns, rows]) {
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


/**
 * @param {(object | null)[]} pair
 */
function upserted([old, now]) {
  return Object.freeze(old === null ? { kind: "inserted", new: now } : { kind: "updated", old, new: now });
}

/**
 * The names of `columns` an ActiveModel holds a value for, set or unchanged:
 * the ones an insert of it writes, as `Insert` reads an ActiveModel.
 *
 * @param {unknown} active
 * @param {readonly string[]} columns
 */
function present(active, columns) {
  return columns.map((column) => native.entityActiveGet(active, column)[0] !== "notSet");
}

// [spec:pgorm:req:bindings.entity-inserts]
/**
 * Refuse a batch whose ActiveModels do not all set the same columns, which
 * Rust's `Insert` records as they are added and refuses before anything is
 * sent; here as they are given.
 *
 * @param {readonly unknown[]} actives
 * @param {readonly string[]} columns
 */
function uniform(actives, columns) {
  const [first, ...rest] = actives.map((active) => present(active, columns));
  const later = rest.find((set) => set.some((held, index) => held !== first?.[index]));
  if (first === undefined || later === undefined) return;
  /** @param {boolean[]} these @param {boolean[]} others */
  const named = (these, others) =>
    columns.filter((_, index) => these[index] && !others[index]).map((name) => JSON.stringify(name)).join(", ");
  const sides = [
    [named(first, later), "set in the first ActiveModel but not in a later one"],
    [named(later, first), "set in a later ActiveModel but not in the first"],
  ].filter(([names]) => names !== "").map(([names, side]) => `${names} ${side}`);
  throw new ConstructionError(`the ActiveModels of one insert set the same columns: ${sides.join("; ")}`);
}

/** An insert's ActiveModels, the entity's columns and the conflict clause it takes. */
class Inserted {
  /** @type {unknown} */
  #entity;
  /** @type {string} */
  #name;
  /** @type {readonly string[]} */
  #columns;
  /** @type {readonly unknown[]} */
  #actives;
  /** @type {Conflict | ConflictUpdate | null} */
  #conflict;

  /**
   * @param {unknown} entity
   * @param {string} name
   * @param {readonly string[]} columns
   * @param {readonly unknown[]} actives
   * @param {Conflict | ConflictUpdate | null} conflict
   */
  constructor(entity, name, columns, actives, conflict) {
    uniform(actives, columns);
    this.#entity = entity;
    this.#name = name;
    this.#columns = columns;
    this.#actives = Object.freeze([...actives]);
    this.#conflict = conflict;
  }

  /**
   * The same insert with `action` as its conflict clause: the statement
   * builders' `Conflict`, naming SQL columns.
   *
   * @param {unknown} action
   */
  withConflict(action) {
    if (!(action instanceof Conflict) && !(action instanceof ConflictUpdate)) {
      throw new TypeError("a conflict's action is Conflict.doNothing(), or a target's doNothing() or update(..)");
    }
    return new Inserted(this.#entity, this.#name, this.#columns, this.#actives, action);
  }

  /**
   * The batch of its ActiveModels and then `actives`, each of its own
   * entity, its conflict clause kept, as Rust's `add` and `add_many` make
   * one.
   *
   * @param {unknown} actives
   */
  added(actives) {
    if (!Array.isArray(actives)) throw new TypeError("addMany takes an array of ActiveModels");
    const owned = actives.map((active) => ownActive(active, this.#name));
    const batch = new Inserted(this.#entity, this.#name, this.#columns, [...this.#actives, ...owned], this.#conflict);
    return new EntityInsertMany(batch, VERSIONS);
  }

  /**
   * The insert run through the terminal `answer` names, of one row when
   * `one`, as the addon gives its outcome.
   *
   * @param {boolean} one
   * @param {"count" | "keys" | "models" | "upserts"} answer
   * @param {unknown} db
   * @param {unknown} options
   */
  async run(one, answer, db, options) {
    const conflict = this.#conflict === null ? null : arg(this.#conflict);
    const job = native.entityInsertJob(this.#entity, this.#actives, conflict, one, answer);
    return await runJob(db, job, options);
  }

  /**
   * Each written row's key, as Rust's `PrimaryKey::ValueType` holds it: the
   * value of a key of one column, a frozen array of a composite key's.
   *
   * @param {boolean} one
   * @param {unknown} db
   * @param {unknown} options
   */
  async keys(one, db, options) {
    const keys = /** @type {unknown[][]} */ (await this.run(one, "keys", db, options));
    return keys.map((parts) => (parts.length === 1 ? parts[0] : Object.freeze(parts)));
  }

  /**
   * Each written row's record.
   *
   * @param {boolean} one
   * @param {unknown} db
   * @param {unknown} options
   */
  async models(one, db, options) {
    const [columns, rows] = /** @type {[string[], [unknown[], unknown][]]} */ (await this.run(one, "models", db, options));
    return rows.map((row) => recordOf(this.#name, columns, row));
  }

  /**
   * What the insert did with each row it wrote.
   *
   * @param {boolean} one
   * @param {unknown} db
   * @param {unknown} options
   */
  async upserts(one, db, options) {
    const outcome = /** @type {any} */ (await this.run(one, "upserts", db, options));
    return versionsOf(this.#name, outcome).map(upserted);
  }
}

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
/** An insert of one ActiveModel, `Insert::one`'s, with the conflict clause it takes. */
export class EntityInsert {
  /** @type {Inserted} */
  #inserted;

  /**
   * @param {Inserted} inserted
   * @param {symbol} token
   */
  constructor(inserted, token) {
    if (token !== VERSIONS) throw new TypeError("an EntityInsert comes from an entity's insert()");
    this.#inserted = inserted;
  }

  /** @param {unknown} action */
  onConflict(action) {
    return new EntityInsert(this.#inserted.withConflict(action), VERSIONS);
  }

  /**
   * A batch of its ActiveModel and `active`, as `Insert::add` makes one.
   *
   * @param {unknown} active
   */
  add(active) {
    return this.#inserted.added([active]);
  }

  /**
   * A batch of its ActiveModel and `actives`, as `Insert::add_many` makes one.
   *
   * @param {readonly unknown[]} actives
   */
  addMany(actives) {
    return this.#inserted.added(actives);
  }

  /**
   * How many rows the insert wrote, by `Insert::exec`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    return /** @type {number} */ (await this.#inserted.run(true, "count", db, options));
  }

  /**
   * The written row's key, by `Insert::exec_returning_pk`; a row the conflict
   * clause held back is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningPk(db, options) {
    return (await this.#inserted.keys(true, db, options))[0];
  }

  /**
   * The written row's record, by `Insert::exec_returning_model`; a row the
   * conflict clause held back is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningModel(db, options) {
    return (await this.#inserted.models(true, db, options))[0];
  }

  /**
   * What the insert did with its row, by `Insert::exec_returning_upsert`:
   * inserted it, updated the row its conflict found, or — null — wrote
   * nothing, its conflict clause holding it back.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningUpsert(db, options) {
    return (await this.#inserted.upserts(true, db, options))[0] ?? null;
  }
}

// [spec:pgorm:req:bindings.entity-versions+3]
// [spec:pgorm:req:bindings.entity-inserts]
/** An insert of ActiveModels, `Insert::many`'s, with the conflict clause it takes. */
export class EntityInsertMany {
  /** @type {Inserted} */
  #inserted;

  /**
   * @param {Inserted} inserted
   * @param {symbol} token
   */
  constructor(inserted, token) {
    if (token !== VERSIONS) throw new TypeError("an EntityInsertMany comes from an entity's insertMany()");
    this.#inserted = inserted;
  }

  /** @param {unknown} action */
  onConflict(action) {
    return new EntityInsertMany(this.#inserted.withConflict(action), VERSIONS);
  }

  /**
   * The batch with `active` after its own, as `Insert::add` adds one.
   *
   * @param {unknown} active
   */
  add(active) {
    return this.#inserted.added([active]);
  }

  /**
   * The batch with `actives` after its own, as `Insert::add_many` adds them.
   *
   * @param {readonly unknown[]} actives
   */
  addMany(actives) {
    return this.#inserted.added(actives);
  }

  /**
   * How many rows the insert wrote, by `Insert::exec`; a batch of none sends
   * nothing.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    return /** @type {number} */ (await this.#inserted.run(false, "count", db, options));
  }

  /**
   * Each written row's key, in the order the database wrote them, by
   * `Insert::exec_returning_pks`; a row the conflict clause held back is
   * left out.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningPks(db, options) {
    return await this.#inserted.keys(false, db, options);
  }

  /**
   * Each written row's record, in the order the database wrote them, by
   * `Insert::exec_returning_models`; a row the conflict clause held back is
   * left out.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningModels(db, options) {
    return await this.#inserted.models(false, db, options);
  }

  /**
   * What the insert did with each row it wrote, by
   * `Insert::exec_returning_upserts`; a row its conflict clause held back is
   * left out, and a batch of none writes nothing.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningUpserts(db, options) {
    return await this.#inserted.upserts(false, db, options);
  }
}

/**
 * An insert of `actives`, the ActiveModels of the entity `entity` names
 * `name`, whose columns are `columns`, with no conflict clause yet.
 *
 * @param {unknown} entity
 * @param {string} name
 * @param {readonly string[]} columns
 * @param {readonly unknown[]} actives
 */
export function inserted(entity, name, columns, actives) {
  return new Inserted(entity, name, columns, actives, null);
}
