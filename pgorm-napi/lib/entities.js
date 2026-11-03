// @ts-check

// Rust entities the application's own module registers, reached by name.
// Every operation is the entity's real Rust API — `Select<E>`'s terminals,
// `ModelTrait::set`, `ActiveModelTrait`'s writes with their
// `ActiveModelBehavior` hooks — run on a pool's connection, a connection or
// a transaction. A record is a frozen plain object keyed by SQL column name,
// behind which the module keeps the Rust model it was read as, so that
// `intoActive` converts the real model rather than rebuilding one.
// [spec:pgorm:req:bindings.entities+4]

import { Handle, TRUSTED } from "./builder.js";
import { Condition } from "./conditions.js";
import { runJob } from "./connections.js";
import { ConstructionError } from "./errors.js";
import { Expr } from "./expressions.js";
import { EntityColumn, EntityQuery, QUERIES } from "./entity-queries.js";
import { EntityInsert, EntityUpdate, EntityUpdateMany, VERSIONS } from "./entity-versions.js";
import { native } from "./operations.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi entity");



/**
 * The Rust model behind each record a registered entity read or wrote, and
 * the registration it belongs to.
 *
 * @type {WeakMap<object, { handle: unknown, entity: string }>}
 */
const models = new WeakMap();

/**
 * A record of `columns` holding `values`, the model `handle` behind it.
 *
 * @param {string} entity
 * @param {string[]} columns
 * @param {[unknown[], unknown]} pair
 */
export function recordOf(entity, columns, [values, handle]) {
  const record = Object.freeze(Object.fromEntries(columns.map((column, index) => [column, values[index]])));
  models.set(record, { handle, entity });
  return record;
}

/**
 * The records of a job's outcome, `[columns, [values, model][]]`.
 *
 * @param {string} entity
 * @param {[string[], [unknown[], unknown][]]} outcome
 */
export function recordsOf(entity, [columns, rows]) {
  return rows.map((pair) => recordOf(entity, columns, pair));
}

/**
 * The model behind `record`, which `entity` must have read or written.
 *
 * @param {unknown} record
 * @param {string} entity
 */
function modelOf(record, entity) {
  const model = typeof record === "object" && record !== null ? models.get(record) : undefined;
  if (!model) throw new TypeError("expected a record a registered entity read or wrote");
  if (model.entity !== entity) throw new ConstructionError(`the record is ${model.entity}'s, not ${entity}'s`);
  return model.handle;
}

/**
 * @param {unknown} predicate
 */
export function predicateOf(predicate) {
  if (!(predicate instanceof Expr) && !(predicate instanceof Condition)) {
    throw new TypeError("a condition is an expression or a Condition");
  }
  return predicate;
}

/** The names of the entities this module registers. */
export function entities() {
  return /** @type {string[]} */ (native.entityNames(native.registry));
}

/**
 * The entity this module registers as `name`.
 *
 * @param {string} name
 */
export function entity(name) {
  if (typeof name !== "string") throw new TypeError("an entity's name is a string");
  return new Entity(native.entityGet(native.registry, name), name, MADE);
}

/**
 * A registered entity: its queries, ActiveModels and the writes that return
 * a row's two versions.
 */
export class Entity extends Handle {
  /** @type {unknown} */
  #native;
  /** @type {string} */
  #name;

  /**
   * @param {unknown} handle
   * @param {string} name
   * @param {symbol} token
   */
  constructor(handle, name, token) {
    if (token !== MADE) throw new TypeError("an Entity comes from entity(name)");
    super(handle);
    this.#native = handle;
    this.#name = name;
    Object.freeze(this);
  }

  /** The name the entity is registered under. */
  get name() {
    return this.#name;
  }

  /** The SQL column names its records are keyed by, in column order. */
  get columns() {
    return /** @type {string[]} */ (native.entityColumns(this.#native));
  }

  /** The registration as data: table, columns, key, relations, Rust types. */
  describe() {
    return JSON.parse(native.entityDescribe(this.#native));
  }

  /**
   * A column, as the entity's `ColumnTrait` names it, whose comparisons
   * convert a value through its declared kind and `save_as`.
   *
   * @param {string} column
   */
  col(column) {
    return new EntityColumn(native.entityCol(this.#native, column), this.#native, column, TRUSTED);
  }

  /** `E::find()`. */
  find() {
    return new EntityQuery(native.entityFind(this.#native), this.#name, QUERIES);
  }

  /** A new ActiveModel from `ActiveModelBehavior::new`, its defaults included. */
  active() {
    return new ActiveModel(native.entityActive(this.#native), this.#name, MADE);
  }

  /**
   * The record's ActiveModel, by the real `IntoActiveModel`: every column
   * `unchanged`.
   *
   * @param {unknown} record
   */
  intoActive(record) {
    return new ActiveModel(native.entityModelActive(modelOf(record, this.#name)), this.#name, MADE);
  }

  /**
   * A copy of the record with `ModelTrait::set` applied to one column;
   * nothing is written.
   *
   * @param {unknown} record
   * @param {string} column
   * @param {unknown} value
   */
  withValue(record, column, value) {
    const pair = native.entityModelSet(modelOf(record, this.#name), column, value);
    return recordOf(this.#name, this.columns, pair);
  }

  /**
   * One column of a record as a `Value`, its kind as the entity declares it.
   *
   * @param {unknown} record
   * @param {string} column
   */
  tagged(record, column) {
    return native.entityModelTagged(modelOf(record, this.#name), column);
  }

  /**
   * An update of the ActiveModel's row by its key, whose terminal returns
   * the row before and after.
   *
   * @param {ActiveModel} active
   */
  update(active) {
    return new EntityUpdate(this.#native, this.#name, ownActive(active, this.#name), VERSIONS);
  }

  /** An update of every row its condition admits, whose terminal returns each row's two versions. */
  updateMany() {
    return new EntityUpdateMany(this.#native, this.#name, [], null, VERSIONS);
  }

  /**
   * An insert of one ActiveModel, whose terminal says what it did with the row.
   *
   * @param {ActiveModel} active
   */
  insert(active) {
    return new EntityInsert(this.#native, this.#name, [ownActive(active, this.#name)], null, true, VERSIONS);
  }

  /**
   * An insert of ActiveModels, whose terminal says what it did with each row.
   *
   * @param {readonly ActiveModel[]} actives
   */
  insertMany(actives) {
    if (!Array.isArray(actives)) throw new TypeError("insertMany takes an array of ActiveModels");
    return new EntityInsert(
      this.#native,
      this.#name,
      actives.map((active) => ownActive(active, this.#name)),
      null,
      false,
      VERSIONS,
    );
  }
}

/** @type {(active: ActiveModel) => unknown} */
let activeHandle;

/**
 * The native half of `active`, an ActiveModel of whichever registration.
 *
 * @param {unknown} active
 */
export function activeHandleOf(active) {
  if (!(active instanceof ActiveModel)) throw new TypeError("expected an ActiveModel");
  return activeHandle(active);
}

/**
 * @param {unknown} active
 * @param {string} entity
 */
function ownActive(active, entity) {
  if (!(active instanceof ActiveModel)) throw new TypeError("expected an ActiveModel");
  if (active.entityName !== entity) throw new ConstructionError(`the ActiveModel is ${active.entityName}'s, not ${entity}'s`);
  return activeHandle(active);
}

/**
 * A registered entity's ActiveModel: each column `notSet`, `set` or
 * `unchanged`. Every change returns a new ActiveModel; the writes run
 * `ActiveModelTrait`'s, its `ActiveModelBehavior` hooks around them.
 */
export class ActiveModel {
  /** @type {unknown} */
  #native;
  /** @type {string} */
  #entity;

  /**
   * @param {unknown} handle
   * @param {string} entity
   * @param {symbol} token
   */
  constructor(handle, entity, token) {
    if (token !== MADE) throw new TypeError("an ActiveModel comes from an entity's active() or intoActive()");
    this.#native = handle;
    this.#entity = entity;
    Object.freeze(this);
  }

  static {
    activeHandle = (active) => active.#native;
  }

  /** The name of the entity it is an ActiveModel of. */
  get entityName() {
    return this.#entity;
  }

  /**
   * A column's state, and its value unless it is `notSet`.
   *
   * @param {string} column
   */
  get(column) {
    const [state, value] = native.entityActiveGet(this.#native, column);
    return Object.freeze(state === "notSet" ? { state } : { state, value });
  }

  /**
   * The column `set` to `value`, converted to its declared kind.
   *
   * @param {string} column
   * @param {unknown} value
   */
  set(column, value) {
    return new ActiveModel(native.entityActiveChange(this.#native, "set", column, value), this.#entity, MADE);
  }

  /** @param {string} column */
  notSet(column) {
    return new ActiveModel(native.entityActiveChange(this.#native, "notSet", column), this.#entity, MADE);
  }

  /**
   * The column back to `unchanged` if it held a value.
   *
   * @param {string} column
   */
  reset(column) {
    return new ActiveModel(native.entityActiveChange(this.#native, "reset", column), this.#entity, MADE);
  }

  /**
   * @param {"insert" | "update" | "delete"} write
   * @param {unknown} db
   * @param {unknown} options
   */
  async #write(write, db, options) {
    return await runJob(db, native.entityActiveJob(this.#native, write), options);
  }

  /**
   * `ActiveModelTrait::insert`, resolving with the inserted record.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async insert(db, options) {
    return recordsOf(this.#entity, await this.#write("insert", db, options))[0];
  }

  /**
   * `ActiveModelTrait::update`, resolving with the updated record.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async update(db, options) {
    return recordsOf(this.#entity, await this.#write("update", db, options))[0];
  }

  /**
   * `ActiveModelTrait::delete`, resolving with the rows it deleted.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async delete(db, options) {
    return /** @type {number} */ (await this.#write("delete", db, options));
  }
}
