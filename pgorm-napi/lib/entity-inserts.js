// @ts-check

// A registered entity's insert, typed by its row count as Rust's is, with
// that count's own terminals, and the try-insert its `onConflictDoNothing()`
// or `onEmptyDoNothing()` makes. They are statement terminals, as in Rust, so
// no `ActiveModelBehavior` hook runs around them.
// [spec:pgorm:req:bindings.entity-inserts]

import { arg } from "./builder.js";
import { Conflict, ConflictUpdate } from "./conflicts.js";
import { runJob } from "./runs.js";
import { ownActive, recordOf } from "./entities.js";
import { VERSIONS, versionsOf } from "./entity-versions.js";
import { EntityTryInsert, EntityTryInsertMany } from "./entity-try-inserts.js";
import { ConstructionError } from "./errors.js";
import { CONFLICTED, EMPTY, inserted as insertedResult } from "./model-try-inserts.js";
import { native } from "./operations.js";

/**
 * A try-insert's conflict clause from `onConflictDoNothing()`: `DO NOTHING`
 * on the primary key, which the addon writes through Rust's
 * `on_conflict_do_nothing`.
 */
const PRIMARY_KEY = Symbol("the primary key");

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

/**
 * An insert's ActiveModels, the entity's columns, the conflict clause it
 * takes, and whether `onConflictDoNothing()` or `onEmptyDoNothing()` made it
 * Rust's `TryInsert`.
 */
export class Inserted {
  /** @type {unknown} */
  #entity;
  /** @type {string} */
  #name;
  /** @type {readonly string[]} */
  #columns;
  /** @type {readonly unknown[]} */
  #actives;
  /** @type {Conflict | ConflictUpdate | typeof PRIMARY_KEY | null} */
  #conflict;
  /** @type {boolean} */
  #tried;

  /**
   * @param {unknown} entity
   * @param {string} name
   * @param {readonly string[]} columns
   * @param {readonly unknown[]} actives
   * @param {Conflict | ConflictUpdate | typeof PRIMARY_KEY | null} conflict
   * @param {boolean} tried
   */
  constructor(entity, name, columns, actives, conflict, tried) {
    uniform(actives, columns);
    this.#entity = entity;
    this.#name = name;
    this.#columns = columns;
    this.#actives = Object.freeze([...actives]);
    this.#conflict = conflict;
    this.#tried = tried;
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
    return new Inserted(this.#entity, this.#name, this.#columns, this.#actives, action, this.#tried);
  }

  /**
   * The insert as a try-insert: its conflict clause `DO NOTHING` on the key
   * with `key`, as `on_conflict_do_nothing` makes it, or kept, as
   * `on_empty_do_nothing` does.
   *
   * @param {boolean} key
   */
  attempted(key) {
    const conflict = key ? PRIMARY_KEY : this.#conflict;
    return new Inserted(this.#entity, this.#name, this.#columns, this.#actives, conflict, true);
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
    const all = [...this.#actives, ...owned];
    return new Inserted(this.#entity, this.#name, this.#columns, all, this.#conflict, this.#tried);
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
    const conflict = this.#conflict === null ? null : this.#conflict === PRIMARY_KEY ? "primaryKey" : arg(this.#conflict);
    const job = native.entityInsertJob(this.#entity, this.#actives, conflict, one, answer, this.#tried);
    return await runJob(db, job, options);
  }

  /**
   * Each written row's key, as Rust's `PrimaryKey::ValueType` holds it: the
   * value of a key of one column, a frozen array of a composite key's.
   *
   * @param {unknown} outcome
   */
  keysOf(outcome) {
    return /** @type {unknown[][]} */ (outcome).map((parts) => (parts.length === 1 ? parts[0] : Object.freeze(parts)));
  }

  /**
   * Each written row's record.
   *
   * @param {unknown} outcome
   */
  modelsOf(outcome) {
    const [columns, rows] = /** @type {[string[], [unknown[], unknown][]]} */ (outcome);
    return rows.map((row) => recordOf(this.#name, columns, row));
  }

  /**
   * @param {boolean} one
   * @param {unknown} db
   * @param {unknown} options
   */
  async keys(one, db, options) {
    return this.keysOf(await this.run(one, "keys", db, options));
  }

  /**
   * @param {boolean} one
   * @param {unknown} db
   * @param {unknown} options
   */
  async models(one, db, options) {
    return this.modelsOf(await this.run(one, "models", db, options));
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

  /**
   * The try-insert run through the terminal `answer` names, answering as
   * pgorm's `TryInsertResult` does, what it wrote read by `read`.
   *
   * @param {boolean} one
   * @param {"count" | "keys" | "models"} answer
   * @param {(outcome: unknown) => unknown} read
   * @param {unknown} db
   * @param {unknown} options
   */
  async attempt(one, answer, read, db, options) {
    const [kind, outcome] = /** @type {[string, unknown]} */ (await this.run(one, answer, db, options));
    if (kind === "conflicted") return CONFLICTED;
    if (kind === "empty") return EMPTY;
    return insertedResult(read(outcome));
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

  // [spec:pgorm:req:bindings.entity-try-inserts]
  /**
   * An `EntityTryInsert` doing nothing on the primary key's columns, or on
   * any key for an entity with none, as `Insert::on_conflict_do_nothing`
   * makes it.
   */
  onConflictDoNothing() {
    return new EntityTryInsert(this.#inserted.attempted(true), VERSIONS);
  }

  /**
   * The insert as an `EntityTryInsert`, its statement unchanged, as
   * `Insert::on_empty_do_nothing` makes it.
   */
  onEmptyDoNothing() {
    return new EntityTryInsert(this.#inserted.attempted(false), VERSIONS);
  }

  /**
   * A batch of its ActiveModel and `active`, as `Insert::add` makes one.
   *
   * @param {unknown} active
   */
  add(active) {
    return new EntityInsertMany(this.#inserted.added([active]), VERSIONS);
  }

  /**
   * A batch of its ActiveModel and `actives`, as `Insert::add_many` makes one.
   *
   * @param {readonly unknown[]} actives
   */
  addMany(actives) {
    return new EntityInsertMany(this.#inserted.added(actives), VERSIONS);
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

  // [spec:pgorm:req:bindings.entity-try-inserts]
  /**
   * An `EntityTryInsertMany` doing nothing on the primary key's columns, or
   * on any key for an entity with none, as `Insert::on_conflict_do_nothing`
   * makes it.
   */
  onConflictDoNothing() {
    return new EntityTryInsertMany(this.#inserted.attempted(true), VERSIONS);
  }

  /**
   * The batch as an `EntityTryInsertMany`, its statement unchanged, as
   * `Insert::on_empty_do_nothing` makes it.
   */
  onEmptyDoNothing() {
    return new EntityTryInsertMany(this.#inserted.attempted(false), VERSIONS);
  }

  /**
   * The batch with `active` after its own, as `Insert::add` adds one.
   *
   * @param {unknown} active
   */
  add(active) {
    return new EntityInsertMany(this.#inserted.added([active]), VERSIONS);
  }

  /**
   * The batch with `actives` after its own, as `Insert::add_many` adds them.
   *
   * @param {readonly unknown[]} actives
   */
  addMany(actives) {
    return new EntityInsertMany(this.#inserted.added(actives), VERSIONS);
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
  return new Inserted(entity, name, columns, actives, null, false);
}
