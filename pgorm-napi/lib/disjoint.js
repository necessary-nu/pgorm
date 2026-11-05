// @ts-check

// Disjoint models: a base model whose every row has a kind, and a model for
// each kind that carries data of its own, keyed by the base row's key.
// `disjoint()` declares one over models; `disjointEntity(name)` is a
// downstream crate's `#[derive(Disjoint)]` enum the module registers. Either
// is pgorm's `DisjointShape`, which builds every statement: the read and its
// narrowing by key and kind, its pages and cursor, the writes and the
// schema. JavaScript only converts the values written and views each row
// read: `[base record, kind, kind record or null]`.
// [spec:pgorm:req:bindings.disjoint+1]

import { Disjoined, DisjointWrite, handleOf, MADE, viewOf } from "./disjoint-query.js";
import { statementsOf } from "./disjoint-schema.js";
import { activeHandleOf, entity, recordOf } from "./entities.js";
import { ConstructionError } from "./errors.js";
import { Expr } from "./expressions.js";
import { columnValue } from "./model-columns.js";
import { decoder } from "./model-decode.js";
import { fieldOf, shapeOf } from "./model-shape.js";
import { Model } from "./models.js";
import { native } from "./operations.js";
import { TypeName } from "./values.js";

// [spec:pgorm:req:bindings.model-unique-keys]
/**
 * A model's table as the native shape is described from.
 *
 * @param {import("./model-shape.js").Shape} shape
 */
function described(shape) {
  return {
    schema: shape.schema,
    table: shape.name,
    columns: shape.fields.map(({ sql, column }) => ({
      name: sql,
      kind: column.kind,
      array: column.array,
      nullable: column.nullable,
      default: column.default,
      generated: column.generated,
      values: column.values,
    })),
    key: shape.primaryKey.map((field) => field.sql),
    uniqueKeys: shape.uniqueKeys.map((key) => ({
      name: key.name,
      columns: key.fields.map((field) => field.sql),
      nullsNotDistinct: key.nullsNotDistinct,
    })),
  };
}

/**
 * @param {unknown} model
 * @param {string} what
 */
function modelOf(model, what) {
  if (!(model instanceof Model)) throw new TypeError(`${what} is a Model`);
  const shape = shapeOf(model);
  if (shape.alias !== null) {
    throw new ConstructionError(`${what} reads its table by its name: a disjoint read names each table bare`);
  }
  return shape;
}

/**
 * `values` as the `[column, Value]` pairs a write gives the model's table,
 * each converted through its field's declared kind, in field order.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} values
 */
function pairs(shape, values) {
  if (typeof values !== "object" || values === null || Array.isArray(values)) {
    throw new TypeError("a write's values are an object of fields");
  }
  for (const field of Object.keys(values)) fieldOf(shape, field);
  return shape.fields
    .filter((field) => Object.hasOwn(values, field.field))
    .map((field) => {
      const value = columnValue(field.column, field.field, /** @type {Record<string, unknown>} */ (values)[field.field]);
      if (value instanceof Expr) throw new ConstructionError(`${field.field} is given an expression: a disjoint write takes values`);
      return [field.sql, value];
    });
}

/**
 * A model's key values in key order, every key field and no other given.
 *
 * @param {import("./model-shape.js").Shape} base
 * @param {unknown} key
 */
function modelKey(base, key) {
  if (typeof key !== "object" || key === null) throw new TypeError("a key is an object of the key's fields");
  const wanted = base.primaryKey.map((field) => field.field);
  const given = Object.keys(key);
  if (given.length !== wanted.length || !wanted.every((name) => Object.hasOwn(key, name))) {
    throw new TypeError(`a key of ${base.name} gives ${wanted.join(", ")}, and nothing else`);
  }
  return base.primaryKey.map((field) =>
    columnValue(field.column, field.field, /** @type {Record<string, unknown>} */ (key)[field.field])
  );
}

/**
 * A disjoint model over `base`: each row of its table has a kind, held in
 * its kind field, and each kind's value maps, in order, to the model of its
 * table or to null for a kind with none. Made by {@link disjoint}.
 */
export class Disjoint extends Disjoined {
  /** @type {import("./model-shape.js").Shape} */
  #base;
  /** @type {ReadonlyMap<string, import("./model-shape.js").Shape | null>} */
  #kinds;

  /**
   * @param {unknown} handle
   * @param {import("./model-shape.js").Shape} base
   * @param {ReadonlyMap<string, import("./model-shape.js").Shape | null>} kinds
   * @param {symbol} token
   */
  constructor(handle, base, kinds, token) {
    if (token !== MADE) throw new TypeError("a Disjoint is made by disjoint(name, base, kinds, options)");
    const values = /** @type {string[]} */ (native.disjointKinds(handle));
    /** @type {Map<number, ReturnType<typeof decoder>>} */
    const decoders = new Map();
    /**
     * The decoder of a row of the kind at `index`: the base's record and the
     * kind's, every other kind's columns left unread.
     *
     * @param {number} index
     * @param {number | null} source
     */
    const decoderOf = (index, source) => {
      let decode = decoders.get(index);
      if (decode === undefined) {
        const own = { fields: base.fields, names: native.disjointResultNames(handle, 0), optional: false };
        const model = kinds.get(/** @type {string} */ (values[index])) ?? null;
        const sources = source === null || model === null
          ? [own]
          : [own, { fields: model.fields, names: native.disjointResultNames(handle, source), optional: false }];
        decode = decoder(sources, false);
        decoders.set(index, decode);
      }
      return decode;
    };
    super(handle, {
      key: (key) => modelKey(base, key),
      rows: ([names, found, columnKinds, judged]) =>
        found.map((/** @type {unknown[]} */ row, /** @type {number} */ at) => {
          const [index, source] = judged[at];
          const decoded = decoderOf(index, source)(names, row, columnKinds);
          const kind = values[index];
          return source === null || !Array.isArray(decoded) ? [decoded, kind, null] : [decoded[0], kind, decoded[1]];
        }),
      columns: (fields) => fields.map((field) => fieldOf(base, field).sql),
      boundary: (fields, values) =>
        values.map((value, index) => {
          const field = fieldOf(base, /** @type {string} */ (fields[index]));
          return columnValue(field.column, field.field, value);
        }),
    });
    this.#base = base;
    this.#kinds = kinds;
    Object.freeze(this);
  }

  get base() {
    return /** @type {Model} */ (this.#base.model);
  }

  /**
   * The values a write gives the row of the kind `kind`.
   *
   * @param {unknown} kind
   * @param {unknown} values
   */
  #row(kind, values) {
    if (typeof kind !== "string") throw new TypeError("a kind is its value, a string");
    const model = this.#kinds.get(kind);
    if (model === undefined || model === null) {
      if (values !== undefined && values !== null && (typeof values !== "object" || Array.isArray(values))) {
        throw new TypeError("a write's values are an object of fields");
      }
      return values === undefined || values === null ? null : [];
    }
    return values === undefined || values === null ? [] : pairs(model, values);
  }

  /**
   * Insert a value of the kind `kind`: the base row from `base`, its kind
   * field holding `kind`, and for a kind with a table its row from `values`,
   * its key read from the inserted base row. The base's kind field and the
   * kind row's key fields come from `kind` and from the base row, so a value
   * given for either is refused.
   *
   * @param {unknown} base
   * @param {unknown} kind
   * @param {unknown} [values]
   */
  insert(base, kind, values) {
    const row = this.#row(kind, values);
    return new DisjointWrite(this, native.disjointInsert(handleOf(this), pairs(this.#base, base), kind, row), MADE);
  }

  /**
   * Update a value of the kind `kind`: the fields `base` sets of the base row
   * whose key it holds, and those `values` sets of its kind's row. A row of
   * another kind, and a key no row has, read back nothing.
   *
   * @param {unknown} base
   * @param {unknown} kind
   * @param {unknown} [values]
   */
  update(base, kind, values) {
    const row = this.#row(kind, values);
    return new DisjointWrite(this, native.disjointUpdate(handleOf(this), pairs(this.#base, base), kind, row), MADE);
  }

  /**
   * Change the kind of the row of `key` to `kind`, its row in the new kind's
   * table, where it has one, from `values`.
   *
   * @param {unknown} key
   * @param {unknown} kind
   * @param {unknown} [values]
   */
  changeKind(key, kind, values) {
    const row = this.#row(kind, values);
    return new DisjointWrite(this, native.disjointChangeKind(handleOf(this), modelKey(this.#base, key), kind, row), MADE);
  }
}

/**
 * Declare a disjoint model `name` over the model `base`, `kinds` mapping each
 * kind's value, in order, to the model of its table or to null. The options
 * are `kind`, the base's kind field (`"kind"` by default), and the kind
 * type's `enumName` (`{name}_kind` by default) and `schemaName` (none by
 * default); a model's enum column names its schema, so the base's kind field
 * is of the kind type only when `schemaName` names it. `unkinded: true`
 * declares a base with no kind field instead, a row's kind being the kind
 * whose table has a row for it, null standing for at most one kind of a row
 * with none: such a model is read-only, and takes none of the other options.
 * Declaring one sends nothing; what pgorm's shape refuses is a
 * `ConstructionError` thrown here.
 *
 * @param {unknown} name
 * @param {unknown} base
 * @param {unknown} kinds
 * @param {unknown} [options]
 */
// [spec:pgorm:req:bindings.disjoint-unkinded]
export function disjoint(name, base, kinds, options = {}) {
  if (typeof name !== "string") throw new TypeError("a disjoint model's name is a string");
  const baseShape = modelOf(base, "a disjoint model's base");
  if (typeof kinds !== "object" || kinds === null || Array.isArray(kinds)) {
    throw new TypeError("a disjoint model's kinds are an object of each kind's value to a Model or null");
  }
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  for (const key of Object.keys(options)) {
    if (key !== "kind" && key !== "enumName" && key !== "schemaName" && key !== "unkinded") {
      throw new TypeError(`${JSON.stringify(key)} is no disjoint model option`);
    }
  }
  const { kind = "kind", enumName = `${name}_kind`, schemaName = null, unkinded = false } =
    /** @type {{ kind?: unknown, enumName?: unknown, schemaName?: unknown, unkinded?: unknown }} */ (options);
  if (typeof unkinded !== "boolean") throw new TypeError("a disjoint model's unkinded is a boolean");
  if (unkinded && ["kind", "enumName", "schemaName"].some((key) => Object.hasOwn(options, key))) {
    throw new ConstructionError(
      "an unkinded disjoint model's base holds no kind: it takes no kind, enumName or schemaName",
    );
  }
  if (typeof kind !== "string") throw new TypeError("a disjoint model's kind field is a string");
  if (typeof enumName !== "string") throw new TypeError("a disjoint model's enumName is a string");
  if (schemaName !== null && typeof schemaName !== "string") throw new TypeError("a disjoint model's schemaName is a string");
  /** @type {Map<string, import("./model-shape.js").Shape | null>} */
  const held = new Map();
  const listed = Object.entries(kinds).map(([value, model]) => {
    const shape = model === null ? null : modelOf(model, `the kind ${JSON.stringify(value)}'s model`);
    held.set(value, shape);
    return [value, shape === null ? null : [shape.name, described(shape)]];
  });
  let kinded = null;
  if (!unkinded) {
    const field = baseShape.byField.get(kind);
    if (field === undefined) {
      throw new ConstructionError(
        `${baseShape.name} has no field ${JSON.stringify(kind)}, the kind field a disjoint model's base holds each row's kind in`,
      );
    }
    kinded = [field.sql, kind, new TypeName(enumName, { schema: schemaName ?? undefined })];
  }
  const handle = native.disjointModel(name, [baseShape.name, described(baseShape)], kinded, listed);
  return new Disjoint(handle, baseShape, held, MADE);
}

/**
 * A downstream crate's `#[derive(Disjoint)]` enum, which the module
 * registers with its base and kind entities: its records are the entities'
 * own, and a write takes their ActiveModels.
 */
export class DisjointEntity extends Disjoined {
  /**
   * @param {unknown} handle
   * @param {symbol} token
   */
  constructor(handle, token) {
    if (token !== MADE) throw new TypeError("a DisjointEntity comes from disjointEntity(name)");
    const description = JSON.parse(native.disjointDescribe(handle));
    /** @param {string} name */
    const registered = (name) => ({ name, columns: entity(name).columns });
    const base = registered(description.base);
    /** @type {Map<string, { name: string, columns: string[] } | null>} */
    const kinds = new Map();
    for (const { value, model } of description.kinds) kinds.set(value, model === null ? null : registered(model));
    super(handle, {
      key: (key) => {
        if (typeof key !== "object" || key === null) throw new TypeError("a key is an object of the key's columns");
        const wanted = /** @type {string[]} */ (native.disjointKeyColumns(handle));
        const given = Object.keys(key);
        if (given.length !== wanted.length || !wanted.every((name) => Object.hasOwn(key, name))) {
          throw new TypeError(`a key gives ${wanted.join(", ")}, and nothing else`);
        }
        return wanted.map((column) => /** @type {Record<string, unknown>} */ (key)[column]);
      },
      rows: (outcome) =>
        outcome.map((/** @type {[[unknown[], unknown], string, [unknown[], unknown] | null]} */ [pair, kind, row]) => {
          const model = kinds.get(kind) ?? null;
          return [
            recordOf(base.name, base.columns, pair),
            kind,
            row === null || model === null ? null : recordOf(model.name, model.columns, row),
          ];
        }),
      columns: (fields) => fields,
      boundary: (_fields, values) => values,
    });
    Object.freeze(this);
  }

  /**
   * Insert a value of the kind `kind` from the base entity's ActiveModel and,
   * for a kind with a table, its entity's.
   *
   * @param {unknown} base
   * @param {unknown} kind
   * @param {unknown} [active]
   */
  insert(base, kind, active) {
    if (typeof kind !== "string") throw new TypeError("a kind is its value, a string");
    return new DisjointWrite(this, native.disjointInsert(handleOf(this), activeHandleOf(base), kind, optionalActive(active)), MADE);
  }

  /**
   * Update a value of the kind `kind`, as `Disjoint.update` does.
   *
   * @param {unknown} base
   * @param {unknown} kind
   * @param {unknown} [active]
   */
  update(base, kind, active) {
    if (typeof kind !== "string") throw new TypeError("a kind is its value, a string");
    return new DisjointWrite(this, native.disjointUpdate(handleOf(this), activeHandleOf(base), kind, optionalActive(active)), MADE);
  }

  /**
   * Change the kind of the row of `key`, as `Disjoint.changeKind` does.
   *
   * @param {unknown} key
   * @param {unknown} kind
   * @param {unknown} [active]
   */
  changeKind(key, kind, active) {
    if (typeof kind !== "string") throw new TypeError("a kind is its value, a string");
    const select = native.disjointChangeKind(handleOf(this), viewOf(this).key(key), kind, optionalActive(active));
    return new DisjointWrite(this, select, MADE);
  }
}

/** @param {unknown} active */
function optionalActive(active) {
  return active === undefined || active === null ? null : activeHandleOf(active);
}

/**
 * The disjoint enum the module registers as `name`.
 *
 * @param {unknown} name
 */
export function disjointEntity(name) {
  if (typeof name !== "string") throw new TypeError("a disjoint entity's name is a string");
  return new DisjointEntity(native.disjointEntity(native.registry, name), MADE);
}

/** The names of the disjoint enums the module registers. */
export function disjointEntities() {
  return /** @type {string[]} */ (native.disjointEntities(native.registry));
}

/**
 * The statements creating a disjoint model's kind type, tables and the
 * constraints that hold each base row to exactly its kind's table, in the
 * order they run: pgorm's `DisjointShape::create`, which
 * `Schema::create_disjoint` writes for a registered enum. A model column
 * whose default the declaration does not state has no `DEFAULT` to write, so
 * its table is refused; the types the tables' other columns use are not
 * among the statements.
 *
 * @param {unknown} model
 */
export function createDisjoint(model) {
  if (!(model instanceof Disjoined)) throw new TypeError("createDisjoint takes a Disjoint or a DisjointEntity");
  return statementsOf(/** @type {[string, unknown][]} */ (native.disjointStatements(handleOf(model))));
}
