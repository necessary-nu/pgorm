// @ts-check

// One-of references: a model's nullable fields, each holding the key of one
// target model, of which a row references exactly one, or at most one where
// the reference is optional. `oneOf()` declares one over models;
// `oneOfEntity(name)` is a downstream crate's `#[derive(OneOf)]` enum the
// module registers. Either is pgorm's `OneOfShape`, which decides which
// target a row references, which fields a write sets, the condition each
// target's rows are loaded by and the error for a key its target does not
// hold, and writes the `CHECK`. JavaScript only reads the records' fields
// and runs the reads.
// [spec:pgorm:req:bindings.one-of+1]

import { TRUSTED } from "./builder.js";
import { modelOf } from "./disjoint.js";
import { described } from "./model-shape.js";
import { ActiveModel, entity } from "./entities.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { Expr } from "./expressions.js";
import { columnValue } from "./model-columns.js";
import { fieldOf } from "./model-shape.js";
import { native } from "./operations.js";
import { AlterTable } from "./schema-alter.js";

/** Marks a construction from `oneOf()` or `oneOfEntity()`. */
const MADE = Symbol("pgorm-napi one-of");

/**
 * How a reference reads its records and its targets: the key of a record as
 * `Value`s, a field of one as a `Value`, the query a target's rows are read
 * by, and a target's key field.
 *
 * @typedef {object} Reader
 * @property {(record: any) => unknown[]} rowKey
 * @property {(record: any, field: string) => unknown} tagged
 * @property {(target: number) => any} query
 * @property {(target: number) => string} key
 */

/**
 * What a reference holds: its native shape, each target's field and value,
 * and its reader.
 *
 * @typedef {object} State
 * @property {unknown} handle
 * @property {readonly string[]} fields
 * @property {readonly string[]} values
 * @property {Reader} reader
 */

/** @type {WeakMap<object, State>} */
const states = new WeakMap();

/**
 * @param {unknown} reference
 * @returns {State}
 */
function stateOf(reference) {
  const state = typeof reference === "object" && reference !== null ? states.get(reference) : undefined;
  if (!state) throw new TypeError("expected a OneOf or a OneOfEntity");
  return state;
}

/**
 * The target `record` references, or `null` for none.
 *
 * @param {State} state
 * @param {unknown} record
 * @returns {number | null}
 */
function referenced(state, record) {
  if (typeof record !== "object" || record === null) throw new TypeError("a one-of reference reads a record");
  const set = state.fields.map((field) => /** @type {Record<string, unknown>} */ (record)[field] != null);
  return native.oneOfReferenced(state.handle, state.reader.rowKey(record), set);
}

/**
 * The fields a write sets for a reference to `value`'s target of key
 * `key`: its field to the key, every other to `null`.
 *
 * @param {State} state
 * @param {unknown} value
 * @param {unknown} key
 * @returns {Record<string, unknown>}
 */
function assigned(state, value, key) {
  if (value !== null && typeof value !== "string") {
    throw new TypeError("a one-of reference's target is its value, a string, or null");
  }
  if (value !== null && (key === null || key === undefined)) {
    throw new ConstructionError(`a reference to ${JSON.stringify(value)} holds its key: pass it, or null for no target`);
  }
  const target = value === null ? null : native.oneOfTarget(state.handle, value);
  const written = /** @type {boolean[]} */ (native.oneOfAssigned(state.handle, target));
  return Object.fromEntries(state.fields.map((field, index) => [field, written[index] ? key : null]));
}

/**
 * What a declaration and a registration share: their description, and each
 * record's reference read and its targets loaded.
 */
class OneOfReference {
  /**
   * @param {unknown} handle
   * @param {Reader} reader
   * @param {symbol} token
   */
  constructor(handle, reader, token) {
    if (token !== MADE) throw new TypeError("a one-of reference is made by oneOf() or oneOfEntity()");
    const targets = /** @type {{ value: string, field: string }[]} */ (JSON.parse(native.oneOfDescribe(handle)).targets);
    states.set(this, {
      handle,
      fields: Object.freeze(targets.map((target) => target.field)),
      values: Object.freeze(targets.map((target) => target.value)),
      reader,
    });
  }

  /** The reference's name. */
  get name() {
    return /** @type {string} */ (this.describe().name);
  }

  /** The reference as data: its name, model, targets and whether it is optional. */
  describe() {
    return JSON.parse(native.oneOfDescribe(stateOf(this).handle));
  }

  /**
   * The record's reference as `[value, key]`, its target's value and the key
   * its field holds, or `null` for a row referencing none where the
   * reference is optional; a row against the declaration is a
   * `DecodeError`.
   *
   * @param {unknown} record
   * @returns {[string, unknown] | null}
   */
  key(record) {
    const state = stateOf(this);
    const target = referenced(state, record);
    if (target === null) return null;
    const field = /** @type {string} */ (state.fields[target]);
    return [/** @type {string} */ (state.values[target]), /** @type {Record<string, unknown>} */ (record)[field]];
  }

  /**
   * Each record's target record, as `[value, record]`, in order, or `null`
   * for a record referencing none: each target's table read once, for the
   * keys the records hold for it. A key its table does not hold is a
   * `DecodeError`.
   *
   * @param {any} db
   * @param {unknown} records
   * @returns {Promise<([string, any] | null)[]>}
   */
  async load(db, records) {
    if (!Array.isArray(records)) throw new TypeError("load takes an array of records");
    const state = stateOf(this);
    const references = records.map((record) => referenced(state, record));
    /** @type {([string, any] | null)[]} */
    const loaded = records.map(() => null);
    for (const [target, field] of state.fields.entries()) {
      const rows = references.flatMap((reference, row) => (reference === target ? [row] : []));
      if (rows.length === 0) continue;
      const keys = rows.map((row) => state.reader.tagged(records[row], field));
      const condition = new Expr(native.oneOfLoadCondition(state.handle, target, keys), TRUSTED);
      const found = /** @type {any[]} */ (await state.reader.query(target).where(condition).all(db));
      const key = state.reader.key(target);
      for (const [place, row] of rows.entries()) {
        const held = records[row][field];
        const match = found.find((item) => item[key] === held);
        if (match === undefined) {
          throw new DecodeError(native.oneOfAbsent(state.handle, target, state.reader.rowKey(records[row]), keys[place]));
        }
        loaded[row] = [/** @type {string} */ (state.values[target]), match];
      }
    }
    return loaded;
  }
}

/**
 * A reference from a model's rows to exactly one of its targets, or to at
 * most one where it is optional, made by {@link oneOf}.
 */
export class OneOf extends OneOfReference {
  /**
   * @param {unknown} handle
   * @param {import("./model-shape.js").Shape} model
   * @param {readonly import("./model-shape.js").Shape[]} targets
   * @param {symbol} token
   */
  constructor(handle, model, targets, token) {
    super(handle, {
      rowKey: (record) => model.primaryKey.map((field) => columnValue(field.column, field.field, record[field.field])),
      tagged: (record, field) => {
        const declared = fieldOf(model, field);
        return columnValue(declared.column, declared.field, record[field]);
      },
      query: (target) => /** @type {any} */ (/** @type {import("./model-shape.js").Shape} */ (targets[target]).model).find(),
      key: (target) => {
        const key = /** @type {import("./model-shape.js").Shape} */ (targets[target]).primaryKey[0];
        return /** @type {import("./model-shape.js").Field} */ (key).field;
      },
    }, token);
    Object.freeze(this);
  }

  /**
   * The fields a write sets for a reference to `value`'s target of key
   * `key`: its field set to the key and every other to `null`, each `null`
   * for `set(null)` where the reference is optional, which a model's
   * `insert` and `update` take beside their other values.
   *
   * @param {unknown} value
   * @param {unknown} [key]
   */
  set(value, key = null) {
    return assigned(stateOf(this), value, key);
  }
}

/**
 * A downstream crate's registered `#[derive(OneOf)]` enum, made by
 * {@link oneOfEntity}: its targets named as its variants are, in snake_case,
 * and its records the entities' own.
 */
export class OneOfEntity extends OneOfReference {
  /**
   * @param {unknown} handle
   * @param {symbol} token
   */
  constructor(handle, token) {
    const description = JSON.parse(native.oneOfDescribe(handle));
    const registered = entity(description.model);
    const targets = /** @type {{ target: string }[]} */ (description.targets).map((target) => entity(target.target));
    super(handle, {
      rowKey: (record) => /** @type {string[]} */ (registered.describe().primaryKey).map((column) => registered.tagged(record, column)),
      tagged: (record, field) => registered.tagged(record, field),
      query: (target) => /** @type {import("./entities.js").Entity} */ (targets[target]).find(),
      key: (target) => /** @type {string} */ (/** @type {import("./entities.js").Entity} */ (targets[target]).describe().primaryKey[0]),
    }, token);
    Object.freeze(this);
  }

  /**
   * `active` with its reference set to `value`'s target of key `key`: that
   * target's column set to the key and every other to null, each null for
   * `set(active, null)` where the reference is optional.
   *
   * @param {unknown} active
   * @param {unknown} value
   * @param {unknown} [key]
   */
  set(active, value, key = null) {
    if (!(active instanceof ActiveModel)) throw new TypeError("set takes the entity's ActiveModel");
    let written = active;
    for (const [column, held] of Object.entries(assigned(stateOf(this), value, key))) written = written.set(column, held);
    return written;
  }
}

/**
 * Declare a reference from `model`'s rows to exactly one of `targets`, or to
 * at most one where `optional` says so: `targets` maps each target's value,
 * in order, to `[field, model]`, the field of `model` holding its key and
 * the target's model, keyed by one field.
 *
 * @param {unknown} name
 * @param {unknown} model
 * @param {unknown} targets
 * @param {unknown} [options]
 */
export function oneOf(name, model, targets, options = {}) {
  if (typeof name !== "string") throw new TypeError("a one-of reference's name is a string");
  const shape = modelOf(model, "a one-of reference's model");
  if (typeof targets !== "object" || targets === null || Array.isArray(targets)) {
    throw new TypeError("a one-of reference's targets map each value to [field, model]");
  }
  if (typeof options !== "object" || options === null) throw new TypeError("a one-of reference's options are an object");
  for (const key of Object.keys(options)) {
    if (key !== "optional") throw new TypeError(`${JSON.stringify(key)} is no one-of option`);
  }
  const { optional = false } = /** @type {{ optional?: unknown }} */ (options);
  if (typeof optional !== "boolean") throw new TypeError("a one-of reference's optional is a boolean");
  /** @type {import("./model-shape.js").Shape[]} */
  const shapes = [];
  const listed = Object.entries(targets).map(([value, target]) => {
    if (!Array.isArray(target) || target.length !== 2 || typeof target[0] !== "string") {
      throw new TypeError("a target is [field, model]");
    }
    const field = shape.byField.get(target[0]);
    if (!field) throw new ConstructionError(`${shape.name} has no field ${JSON.stringify(target[0])}`);
    const targetShape = modelOf(target[1], `the target ${JSON.stringify(value)}'s model`);
    shapes.push(targetShape);
    return [value, field.field, field.sql, [targetShape.name, described(targetShape)]];
  });
  const handle = native.oneOfModel(name, [shape.name, described(shape)], listed, optional);
  return new OneOf(handle, shape, Object.freeze(shapes), MADE);
}

/**
 * The `#[derive(OneOf)]` enum the module registers as `name`.
 *
 * @param {string} name
 */
export function oneOfEntity(name) {
  return new OneOfEntity(native.oneOfEntity(native.registry, name), MADE);
}

/** The names of the one-of enums the module registers. */
export function oneOfEntities() {
  return /** @type {string[]} */ (native.oneOfEntities(native.registry));
}

/**
 * The `ALTER TABLE` adding the `CHECK` that holds every row of a one-of
 * reference to one target, or to one at most where it is optional:
 * pgorm's `OneOfShape::check`, which `Schema::create_one_of` writes for a
 * registered enum.
 *
 * @param {unknown} reference
 */
export function createOneOf(reference) {
  if (!(reference instanceof OneOfReference)) throw new TypeError("createOneOf takes a OneOf or a OneOfEntity");
  return new AlterTable(native.oneOfStatement(stateOf(reference).handle), TRUSTED);
}
