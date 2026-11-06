// @ts-check

// A declared model's record writes and the hooks around them, the dialect of
// Rust's `ActiveModelTrait` and `ActiveModelBehavior`: `Model.active(values)`
// holds the fields a write sets, and its `insert`, `update` and `delete`
// write one record through the model's own statement builders, each between
// the model's `before*` and `after*` hooks.
// [spec:pgorm:req:bindings.model-hooks]

import { ConstructionError, DecodeError } from "./errors.js";
import { fieldOf, shapeOf } from "./model-shape.js";

const MADE = Symbol("pgorm-napi model active");

/** The hooks a model takes, in the order `describe()` lists them. */
export const HOOKS = Object.freeze(["beforeSave", "afterSave", "beforeDelete", "afterDelete"]);

/**
 * `hooks` as a model holds them, refused unless an object of known hooks to
 * functions.
 *
 * @param {unknown} hooks
 * @returns {Readonly<Record<string, Function>>}
 */
export function hooksOf(hooks) {
  if (hooks === undefined || hooks === null) return Object.freeze({});
  if (typeof hooks !== "object" || Array.isArray(hooks)) throw new TypeError("a model's hooks are an object of hook functions");
  /** @type {Record<string, Function>} */
  const held = {};
  for (const [name, hook] of Object.entries(hooks)) {
    if (!HOOKS.includes(name)) throw new TypeError(`${JSON.stringify(name)} is no model hook: a model takes ${HOOKS.join(", ")}`);
    if (typeof hook !== "function") throw new TypeError(`a model's ${name} hook is a function`);
  }
  for (const name of HOOKS) {
    const hook = /** @type {Record<string, unknown>} */ (hooks)[name];
    if (typeof hook === "function") held[name] = hook;
  }
  return Object.freeze(held);
}

/**
 * The fields one record write sets, as a Rust `ActiveModel` holds them:
 * `insert`, `update` and `delete` write the record and run the model's hooks
 * around the write, as `ActiveModelTrait` does. A model's statement builders,
 * `insert`, `update` and `delete` on the model itself, run none, as Rust's
 * `Insert`, `Update` and `Delete` run none.
 */
export class ModelActive {
  /** @type {any} */
  #model;
  /** @type {Readonly<Record<string, unknown>>} */
  #values;

  /**
   * @param {object} model
   * @param {unknown} values
   * @param {symbol} token
   */
  constructor(model, values, token) {
    if (token !== MADE) throw new TypeError("a ModelActive comes from a model's active()");
    if (typeof values !== "object" || values === null || Array.isArray(values)) {
      throw new TypeError("an active model's values are an object of fields");
    }
    const shape = shapeOf(model);
    for (const field of Object.keys(values)) fieldOf(shape, field);
    this.#model = model;
    this.#values = Object.freeze({ ...values });
    Object.freeze(this);
  }

  /** The fields the write sets. */
  get values() {
    return this.#values;
  }

  /**
   * @param {string} name
   * @param {...unknown} rest
   * @returns {Promise<Record<string, unknown>>}
   */
  async #before(name, ...rest) {
    const hook = shapeOf(this.#model).hooks[name];
    if (hook === undefined) return { ...this.#values };
    const values = await hook({ ...this.#values }, ...rest);
    if (typeof values !== "object" || values === null || Array.isArray(values)) {
      throw new TypeError(`a model's ${name} hook gives the values to write`);
    }
    return { ...values };
  }

  /**
   * @param {Record<string, unknown>} record
   * @param {boolean} insert
   */
  async #saved(record, insert) {
    const hook = shapeOf(this.#model).hooks.afterSave;
    if (hook === undefined) return record;
    const saved = await hook(record, insert);
    if (typeof saved !== "object" || saved === null) throw new TypeError("a model's afterSave hook gives the record the write returns");
    return saved;
  }

  /**
   * The primary key the write names, from `values`.
   *
   * @param {Record<string, unknown>} values
   * @param {string} write
   */
  #key(values, write) {
    const shape = shapeOf(this.#model);
    if (shape.primaryKey.length === 0) throw new ConstructionError(`${shape.name} declares no primary key to ${write} a record by`);
    const missing = shape.primaryKey.filter((field) => !Object.hasOwn(values, field.field)).map((field) => field.field);
    if (missing.length > 0) throw new ConstructionError(`a record ${write} needs its key: ${missing.join(", ")} is not set`);
    return Object.fromEntries(shape.primaryKey.map((field) => [field.field, values[field.field]]));
  }

  /**
   * `ActiveModelTrait::insert`: `beforeSave(values, true)`, the row inserted
   * and read back, then `afterSave(record, true)`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async insert(db, options) {
    const values = await this.#before("beforeSave", true);
    const record = await this.#model.insert(values).returning().one(db, options);
    return await this.#saved(record, true);
  }

  /**
   * `ActiveModelTrait::update`: `beforeSave(values, false)`, the row the key
   * names updated with the other fields set and read back, then
   * `afterSave(record, false)`. A key no row has is a `DecodeError`, as
   * Rust's `RecordNotUpdated` is an error.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async update(db, options) {
    const values = await this.#before("beforeSave", false);
    const key = this.#key(values, "update");
    const changes = Object.fromEntries(Object.entries(values).filter(([field]) => !Object.hasOwn(key, field)));
    const rows = await this.#model.update(changes).where(this.#model.key(key)).returning().all(db, options);
    if (rows.length !== 1) throw new DecodeError(`no ${shapeOf(this.#model).name} row has the key ${JSON.stringify(key, (_, value) => typeof value === "bigint" ? `${value}` : value)} to update`);
    return await this.#saved(rows[0], false);
  }

  /**
   * `ActiveModelTrait::delete`: `beforeDelete(values)`, the row the key
   * names deleted, then `afterDelete(values)`; the rows deleted.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async delete(db, options) {
    const values = await this.#before("beforeDelete");
    const key = this.#key(values, "delete");
    const deleted = await this.#model.delete().where(this.#model.key(key)).execute(db, options);
    const hook = shapeOf(this.#model).hooks.afterDelete;
    if (hook !== undefined) await hook(Object.freeze(values));
    return deleted;
  }
}

/**
 * The active model of `model` setting `values`.
 *
 * @param {object} model
 * @param {unknown} values
 */
export function active(model, values) {
  return new ModelActive(model, values ?? {}, MADE);
}
