// @ts-check

// Rust entities the application's own module registers, reached by name.
// Every operation is the entity's real Rust API — `Select<E>`'s terminals,
// `pgorm::Model::set`, `pgorm::ActiveModel`'s writes with their
// `ActiveModelBehavior` hooks — run on a pool's connection, a connection or
// a transaction. A record is a frozen plain object keyed by SQL column name,
// behind which the module keeps the Rust model it was read as, so that
// `intoActive` converts the real model rather than rebuilding one.
// [spec:pgorm:req:bindings.entities+6]

import { Handle, TRUSTED } from "./builder.js";
import { Condition } from "./conditions.js";
import { runJob } from "./runs.js";
import { Expr } from "./expressions.js";
import { EntityColumn, EntityQuery, QUERIES } from "./entity-queries.js";
import { EntityRelation, READS } from "./entity-reads.js";
import { EntityInsert, EntityInsertMany, inserted } from "./entity-inserts.js";
import { EntityUpdate, EntityUpdateMany, VERSIONS } from "./entity-versions.js";
import { native } from "./operations.js";
import { heldOf } from "./scopes.js";

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
 * The rows of a read of several sources, `[columns per source, ([values,
 * model] | null)[][]]`: a row of one source as its record, and otherwise a
 * tuple of each source's, null for an absent one.
 *
 * @param {readonly string[]} entities
 * @param {[string[][], ([unknown[], unknown] | null)[][]]} outcome
 */
export function rowsOf(entities, [columns, rows]) {
  return rows.map((row) => {
    const records = row.map((pair, index) =>
      pair === null ? null : recordOf(/** @type {string} */ (entities[index]), /** @type {string[]} */ (columns[index]), pair)
    );
    return records.length === 1 ? records[0] : records;
  });
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
  if (model.entity !== entity) throw new TypeError(`the record is ${model.entity}'s, not ${entity}'s`);
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
   * A column, as the entity's `pgorm::Column` names it, whose comparisons
   * convert a value through its declared kind and `save_as`.
   *
   * @param {string} column
   */
  col(column) {
    return new EntityColumn(native.entityCol(this.#native, column), this.#native, column, TRUSTED);
  }

  /** The scopes the entity declares, by name: none for an unscoped one. */
  get scopes() {
    return Object.freeze(/** @type {string[]} */ (native.entityScopes(this.#native)));
  }

  /**
   * The Rust scope values a statement under `held` runs under, refused under
   * `null` where the entity is scoped.
   *
   * @param {import("./model-shape.js").Held} held
   */
  #scoping(held) {
    return native.entityScoping(this.#native, held);
  }

  /** `E::find()`. */
  find() {
    return new EntityQuery(native.entityFind(this.#native, this.#scoping(null)), this.#name, QUERIES);
  }

  /**
   * `E::find_for(cx)`: the rows `cx`'s scopes hold.
   *
   * @param {unknown} cx
   */
  findFor(cx) {
    return new EntityQuery(native.entityFind(this.#native, this.#scoping(heldOf(cx))), this.#name, QUERIES);
  }

  /**
   * The relation the entity's `Relation` enum declares under `name`, for a
   * query to join.
   *
   * @param {string} name
   */
  relation(name) {
    if (typeof name !== "string") throw new TypeError("a relation's name is a string");
    native.entityRelation(this.#native, name);
    return new EntityRelation(this.#native, this.#name, name, READS);
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
   * A copy of the record with `pgorm::Model::set` applied to one column;
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
    return new EntityUpdate(this.#native, this.#name, ownActive(active, this.#name), this.#scoping(null), VERSIONS);
  }

  /**
   * `Update::one_for(cx, active)`: the update by key under `cx`, a row
   * outside its scopes no row.
   *
   * @param {unknown} cx
   * @param {ActiveModel} active
   */
  updateFor(cx, active) {
    return new EntityUpdate(this.#native, this.#name, ownActive(active, this.#name), this.#scoping(heldOf(cx)), VERSIONS);
  }

  /** An update of every row its condition admits, whose terminal returns each row's two versions. */
  updateMany() {
    return new EntityUpdateMany(this.#native, this.#name, [], null, this.#scoping(null), VERSIONS);
  }

  /**
   * `Update::many_for(cx, ..)`: `cx`'s scopes are its first condition, so
   * even `allRows()` reaches no row outside them.
   *
   * @param {unknown} cx
   */
  updateManyFor(cx) {
    return new EntityUpdateMany(this.#native, this.#name, [], null, this.#scoping(heldOf(cx)), VERSIONS);
  }

  /**
   * An insert of one ActiveModel, `Insert::one`, whose terminal says what it
   * did with the row.
   *
   * @param {ActiveModel} active
   */
  insert(active) {
    return this.#insert(null, [ownActive(active, this.#name)]);
  }

  /**
   * `Insert::one_for(cx, active)`: a scope column the ActiveModel leaves
   * unset is written from `cx`, one set to another value refused.
   *
   * @param {unknown} cx
   * @param {ActiveModel} active
   */
  insertFor(cx, active) {
    return this.#insert(heldOf(cx), [ownActive(active, this.#name)]);
  }

  /**
   * An insert of ActiveModels, `Insert::many`, whose terminal says what it
   * did with each row.
   *
   * @param {readonly ActiveModel[]} actives
   */
  insertMany(actives) {
    return this.#insertMany(null, actives);
  }

  /**
   * `Insert::many_for(cx, actives)`: each row as {@link insertFor} writes
   * one; under `every(..)` each sets that scope's column itself.
   *
   * @param {unknown} cx
   * @param {readonly ActiveModel[]} actives
   */
  insertManyFor(cx, actives) {
    return this.#insertMany(heldOf(cx), actives);
  }

  /**
   * @param {import("./model-shape.js").Held} held
   * @param {readonly unknown[]} owned
   */
  #insert(held, owned) {
    return new EntityInsert(inserted(this.#native, this.#name, this.columns, owned, this.#scoping(held)), VERSIONS);
  }

  /**
   * @param {import("./model-shape.js").Held} held
   * @param {unknown} actives
   */
  #insertMany(held, actives) {
    if (!Array.isArray(actives)) throw new TypeError("insertMany takes an array of ActiveModels");
    const owned = actives.map((active) => ownActive(active, this.#name));
    return new EntityInsertMany(inserted(this.#native, this.#name, this.columns, owned, this.#scoping(held)), VERSIONS);
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
 * The native half of `active`, refused unless it is an ActiveModel of the
 * registration `entity` names: another's is an argument of the wrong type.
 * [spec:pgorm:req:bindings.entity-writes+2]
 *
 * @param {unknown} active
 * @param {string} entity
 */
export function ownActive(active, entity) {
  if (!(active instanceof ActiveModel)) throw new TypeError("expected an ActiveModel");
  if (active.entityName !== entity) throw new TypeError(`the ActiveModel is ${active.entityName}'s, not ${entity}'s`);
  return activeHandle(active);
}

/**
 * A registered entity's ActiveModel: each column `notSet`, `set` or
 * `unchanged`. Every change returns a new ActiveModel; the writes run
 * `pgorm::ActiveModel`'s, its `ActiveModelBehavior` hooks around them.
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

  /**
   * The column `set` to `value` unless it holds that value unchanged, as
   * pgorm's `set_if_not_equals` sets it, so `isChanged()` then says whether
   * there is anything to write.
   *
   * @param {string} column
   * @param {unknown} value
   */
  setIfNotEquals(column, value) {
    return new ActiveModel(native.entityActiveChange(this.#native, "setIfNotEquals", column, value), this.#entity, MADE);
  }

  /** Whether any column is set, `pgorm::ActiveModel::is_changed`. */
  isChanged() {
    return /** @type {boolean} */ (native.entityActiveChanged(this.#native));
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
   * @param {import("./model-shape.js").Held} [held] The context it is written under, none by default.
   */
  async #write(write, db, options, held = null) {
    const scoping = native.entityScoping(this.#native, held);
    return await runJob(db, native.entityActiveJob(this.#native, write, scoping), options);
  }

  /**
   * `pgorm::ActiveModel::insert`, resolving with the inserted record.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async insert(db, options) {
    return recordsOf(this.#entity, await this.#write("insert", db, options))[0];
  }

  /**
   * `pgorm::ActiveModel::update`, resolving with the updated record.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async update(db, options) {
    return recordsOf(this.#entity, await this.#write("update", db, options))[0];
  }

  /**
   * `pgorm::ActiveModel::delete`, resolving with the rows it deleted.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async delete(db, options) {
    return /** @type {number} */ (await this.#write("delete", db, options));
  }

  /**
   * `pgorm::ActiveModel::insert_for`: the row's scope columns filled from
   * `cx`.
   *
   * @param {unknown} cx
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async insertFor(cx, db, options) {
    return recordsOf(this.#entity, await this.#write("insert", db, options, heldOf(cx)))[0];
  }

  /**
   * `pgorm::ActiveModel::update_for`: a row outside `cx`'s scopes is no row,
   * a `DecodeError`.
   *
   * @param {unknown} cx
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async updateFor(cx, db, options) {
    return recordsOf(this.#entity, await this.#write("update", db, options, heldOf(cx)))[0];
  }

  /**
   * `pgorm::ActiveModel::delete_for`: a row outside `cx`'s scopes is not
   * deleted.
   *
   * @param {unknown} cx
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async deleteFor(cx, db, options) {
    return /** @type {number} */ (await this.#write("delete", db, options, heldOf(cx)));
  }
}
