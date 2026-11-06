// @ts-check

// Models JavaScript declares: a table, schema-qualified or not, and its
// columns' kinds, nullability, keys and defaults, read and written through
// the statement builders. Declaring a model sends nothing and runs no DDL,
// and claims no Rust entity: no derive, `EntityTrait` or ActiveModel hook is
// behind it.
// [spec:pgorm:req:bindings.models+6]
// [spec:pgorm:req:bindings.model-unique-keys+2]
// [spec:pgorm:req:bindings.schema-from-model]

import { Condition } from "./conditions.js";
import { ConstructionError } from "./errors.js";
import { graphOf } from "./graphs.js";
import { Column, columnKey } from "./model-columns.js";
import { query } from "./model-queries.js";
import { described, fieldOf, qualified, register, shapeOf } from "./model-shape.js";
import { picked } from "./model-rows.js";
import { active, hooksOf } from "./model-active.js";
import { deleteRows, insertMany, insertOne, updateSet } from "./model-writes.js";
import { native } from "./operations.js";
import { linked, related, relation } from "./relations.js";
import { Table } from "./select.js";
import { isIdentifier } from "./values.js";

/** Marks a construction from `model()`. */
const MADE = Symbol("pgorm-napi model");

/**
 * The fields of `columns`, each with the SQL column it names, refusing a
 * field that is no column declaration and two fields of one column.
 *
 * @param {unknown} columns
 * @param {string | null} renameAll The case style a field's column is named in, where its column names none.
 * @returns {import("./model-shape.js").Field[]}
 */
function fieldsOf(columns, renameAll) {
  if (typeof columns !== "object" || columns === null || Array.isArray(columns)) {
    throw new TypeError("a model's columns are an object of column() declarations");
  }
  const fields = Object.entries(columns).map(([field, column]) => {
    if (!(column instanceof Column)) throw new TypeError(`${field} is no column() declaration`);
    if (field === "") throw new ConstructionError("a field has a name");
    const sql = column.name ?? (renameAll === null ? field : native.modelRenamed(field, renameAll));
    if (!isIdentifier(sql)) throw new ConstructionError("an identifier is 1–63 UTF-8 bytes without NUL");
    return { field, sql, column };
  });
  if (fields.length === 0) throw new ConstructionError("a model has at least one column");
  const sql = new Set(fields.map((field) => field.sql));
  if (sql.size !== fields.length) throw new ConstructionError("two fields of a model name one column");
  return fields;
}

const KEY_OPTIONS = new Set(["fields", "name", "nullsNotDistinct"]);

/**
 * One of a model's unique keys, its fields among `fields`, refused as
 * PostgreSQL or the Rust derive would refuse it.
 *
 * @param {unknown} key
 * @param {ReadonlyMap<string, import("./model-shape.js").Field>} fields
 * @returns {import("./model-shape.js").UniqueKey}
 */
function uniqueKeyOf(key, fields) {
  if (typeof key !== "object" || key === null || Array.isArray(key)) {
    throw new TypeError("a unique key is an object of its fields and options");
  }
  for (const option of Object.keys(key)) {
    if (!KEY_OPTIONS.has(option)) throw new TypeError(`${JSON.stringify(option)} is no unique key option`);
  }
  const { fields: given = [], name = null, nullsNotDistinct = false } =
    /** @type {{ fields?: unknown, name?: unknown, nullsNotDistinct?: unknown }} */ (key);
  const named = typeof given === "string" ? [given] : given;
  if (!Array.isArray(named)) throw new TypeError("a unique key's fields are a field or an array of them");
  /** @type {import("./model-shape.js").Field[]} */
  const keyed = [];
  for (const field of named) {
    if (typeof field !== "string") throw new TypeError("a unique key's field is named by a string");
    const declared = fields.get(field);
    if (!declared) throw new ConstructionError(`a unique key names ${JSON.stringify(field)}, which is no field of the model`);
    if (keyed.includes(declared)) {
      throw new ConstructionError(`a unique key names ${JSON.stringify(field)} twice, which PostgreSQL refuses`);
    }
    keyed.push(declared);
  }
  if (keyed.length === 0) throw new ConstructionError("a unique key names at least one field");
  if (name !== null) {
    if (typeof name !== "string") throw new TypeError("a unique key's name is a string");
    if (!isIdentifier(name)) throw new ConstructionError("an identifier is 1–63 UTF-8 bytes without NUL");
  }
  if (typeof nullsNotDistinct !== "boolean") throw new TypeError("a unique key's nullsNotDistinct is a boolean");
  return Object.freeze({ name, fields: Object.freeze(keyed), nullsNotDistinct });
}

// [spec:pgorm:req:bindings.schema-from-model]
/**
 * Refuse a key the serial family or a period cannot be part of: a serial key
 * is of one column, and a period is the last column of a key of two or more,
 * as PostgreSQL requires of `WITHOUT OVERLAPS`.
 *
 * @param {readonly import("./model-shape.js").Field[]} key
 */
function checkKey(key) {
  if (key.some((field) => field.column.autoIncrement) && key.length !== 1) {
    throw new ConstructionError("autoIncrement draws a one-column key from the serial family");
  }
  const periods = key.flatMap((field, index) => (field.column.withoutOverlaps ? [index] : []));
  if (periods.length > 0 && (periods.length !== 1 || periods[0] !== key.length - 1 || key.length < 2)) {
    throw new ConstructionError("withoutOverlaps marks the last field of a key of two or more as its period");
  }
}

/**
 * @param {unknown} keys
 * @param {readonly import("./model-shape.js").Field[]} fields
 */
function uniqueKeysOf(keys, fields) {
  if (!Array.isArray(keys)) throw new TypeError("a model's uniqueKeys are an array of keys");
  const byField = new Map(fields.map((field) => [field.field, field]));
  return Object.freeze(keys.map((key) => uniqueKeyOf(key, byField)));
}

/**
 * A model: a table and its columns' declarations. Its methods build the
 * queries and writes over it, each a new value; nothing about a model
 * changes once it is made.
 */
export class Model {
  /**
   * @param {string} name
   * @param {string | null} schema
   * @param {string | null} alias
   * @param {readonly import("./model-shape.js").Field[]} fields
   * @param {readonly import("./model-shape.js").UniqueKey[]} uniqueKeys
   * @param {string | null} comment
   * @param {Readonly<Record<string, Function>>} hooks
   * @param {symbol} token
   */
  constructor(name, schema, alias, fields, uniqueKeys, comment, hooks, token) {
    if (token !== MADE) throw new TypeError("a Model is made by model(name, { columns })");
    const table = new Table(name, { schema: schema ?? undefined, alias: alias ?? undefined });
    const qualifier = alias ?? name;
    for (const field of fields) qualified(field, qualifier);
    const primaryKey = fields.filter((field) => field.column.primaryKey);
    checkKey(primaryKey);
    /** @type {import("./model-shape.js").Shape} */
    const shape = {
      model: this,
      name,
      schema,
      alias,
      qualifier,
      table,
      fields,
      byField: new Map(fields.map((field) => [field.field, field])),
      primaryKey,
      uniqueKeys,
      comment,
      hooks,
    };
    native.modelTable(described(shape));
    register(this, shape);
    Object.freeze(this);
  }

  /** The table's name. */
  get name() {
    return shapeOf(this).name;
  }

  get schema() {
    return shapeOf(this).schema;
  }

  get alias() {
    return shapeOf(this).alias;
  }

  /** The table's comment, which its schema writes. */
  get comment() {
    return shapeOf(this).comment;
  }

  /** The table, schema-qualified and aliased as the model is. */
  get table() {
    return shapeOf(this).table;
  }

  /** Each field's column declaration. */
  get columns() {
    return Object.freeze(Object.fromEntries(shapeOf(this).fields.map((field) => [field.field, field.column])));
  }

  /** The primary key's fields, in declaration order. */
  get primaryKey() {
    return Object.freeze(shapeOf(this).primaryKey.map((field) => field.field));
  }

  /**
   * The same model read under `alias`: its columns qualified by it, as a
   * self-join needs.
   *
   * @param {string} alias
   */
  as(alias) {
    const shape = shapeOf(this);
    return new Model(shape.name, shape.schema, alias, shape.fields, shape.uniqueKeys, shape.comment, shape.hooks, MADE);
  }

  /**
   * A field's column, qualified as the model's table is named, whose
   * comparisons convert a value through the field's declared kind.
   *
   * @param {string} field
   */
  col(field) {
    const shape = shapeOf(this);
    return qualified(fieldOf(shape, field), shape.qualifier);
  }

  /**
   * The condition matching the row whose primary key `values` gives: every
   * key field, and no other.
   *
   * @param {unknown} values
   */
  key(values) {
    const shape = shapeOf(this);
    if (shape.primaryKey.length === 0) throw new ConstructionError(`${shape.name} declares no primary key`);
    if (typeof values !== "object" || values === null) throw new TypeError("a key is an object of the key's fields");
    const given = Object.keys(values);
    const wanted = shape.primaryKey.map((field) => field.field);
    if (given.length !== wanted.length || !wanted.every((name) => Object.hasOwn(values, name))) {
      throw new TypeError(`a key of ${shape.name} gives ${wanted.join(", ")}, and nothing else`);
    }
    return Condition.all(...shape.primaryKey.map((field) =>
      qualified(field, shape.qualifier).eq(/** @type {Record<string, unknown>} */ (values)[field.field])
    ));
  }

  /** Every field of every row. */
  find() {
    const shape = shapeOf(this);
    return query(shape, shape.fields);
  }

  /**
   * The fields named, of every row.
   *
   * @param {...string} fields
   */
  select(...fields) {
    const shape = shapeOf(this);
    if (fields.length === 0) throw new ConstructionError("select names at least one field");
    return query(shape, picked(shape, fields));
  }

  /**
   * The row whose primary key `values` gives.
   *
   * @param {unknown} values
   */
  findByKey(values) {
    return this.find().where(this.key(values));
  }

  /**
   * An INSERT of one row: the fields `values` sets, each converted through
   * its declared kind. A field left out takes its default; one the model
   * declares neither nullable nor defaulted must be set.
   *
   * @param {unknown} values
   */
  insert(values) {
    return insertOne(shapeOf(this), values);
  }

  /**
   * An INSERT of rows that each set the same fields.
   *
   * @param {unknown} rows
   */
  insertMany(rows) {
    return insertMany(shapeOf(this), rows);
  }

  /**
   * An UPDATE of the fields `values` sets, which needs a `where` or
   * `allRows()` before it runs.
   *
   * @param {unknown} values
   */
  update(values) {
    return updateSet(shapeOf(this), values);
  }

  /**
   * The fields one record write sets, whose `insert`, `update` and `delete`
   * run the model's hooks around the write, as a Rust `ActiveModel`'s do.
   *
   * @param {unknown} [values]
   */
  active(values) {
    return active(this, values);
  }

  /** A DELETE, which needs a `where` or `allRows()` before it runs. */
  delete() {
    return deleteRows(shapeOf(this));
  }

  /**
   * A relation from this model's foreign key to the key of `target`.
   * `keys.onCondition`, given the two ends as a join names them, adds a
   * condition of its own, ANDed with the fields' equality, or ORed when
   * `keys.conditionType` is `"any"`.
   *
   * @param {unknown} target
   * @param {unknown} keys
   */
  belongsTo(target, keys) {
    return relation("belongsTo", this, target, keys);
  }

  /**
   * A relation from this model's key to at most one row of `target`.
   *
   * @param {unknown} target
   * @param {unknown} keys
   */
  hasOne(target, keys) {
    return relation("hasOne", this, target, keys);
  }

  /**
   * A relation from this model's key to any number of rows of `target`.
   *
   * @param {unknown} target
   * @param {unknown} keys
   */
  hasMany(target, keys) {
    return relation("hasMany", this, target, keys);
  }

  /**
   * The path from this model through a junction, as a Rust `Related`
   * implementation with a `via` declares it: `options.via` from this model
   * to the junction, then `target` from the junction to the far end.
   *
   * @param {unknown} target
   * @param {unknown} options
   */
  related(target, options) {
    return related(this, target, options);
  }

  /**
   * A chain of relations from this model, each starting where the one
   * before it ends, as a Rust `Linked` type lists them.
   *
   * @param {...unknown} relations
   */
  linked(...relations) {
    return linked(this, relations);
  }

  /** A graph rooted at this model. */
  graph() {
    return graphOf(shapeOf(this));
  }

  /** The declaration as data: the table, its keys, and each field's column. */
  describe() {
    const shape = shapeOf(this);
    return {
      schema: shape.schema,
      table: shape.name,
      alias: shape.alias,
      primaryKey: shape.primaryKey.map((field) => field.field),
      hooks: Object.keys(shape.hooks),
      uniqueKeys: shape.uniqueKeys.map((key) => ({
        name: key.name,
        fields: key.fields.map((field) => field.field),
        nullsNotDistinct: key.nullsNotDistinct,
      })),
      fields: Object.fromEntries(shape.fields.map(({ field, sql, column }) => [field, {
        column: sql,
        kind: columnKey(column),
        nullable: column.nullable,
        primaryKey: column.primaryKey,
        default: column.default,
        generated: column.generated,
        values: column.values,
      }])),
    };
  }
}

/**
 * Declare a model of the table `name`.
 *
 * @param {unknown} name
 * @param {unknown} declaration
 */
export function model(name, declaration) {
  if (typeof name !== "string") throw new TypeError("a model's table name is a string");
  if (typeof declaration !== "object" || declaration === null) {
    throw new TypeError("a model's declaration is { columns, schema?, uniqueKeys? }");
  }
  for (const key of Object.keys(declaration)) {
    if (!MODEL_OPTIONS.has(key)) throw new TypeError(`${JSON.stringify(key)} is no model option`);
  }
  const { columns, schema = null, uniqueKeys = [], comment = null, renameAll = null, hooks } =
    /** @type {{ columns?: unknown, schema?: unknown, uniqueKeys?: unknown, comment?: unknown, renameAll?: unknown, hooks?: unknown }} */ (
      declaration
    );
  if (schema !== null && typeof schema !== "string") throw new TypeError("a model's schema is a string");
  if (comment !== null && typeof comment !== "string") throw new TypeError("a model's comment is a string");
  if (renameAll !== null && typeof renameAll !== "string") throw new TypeError("a model's renameAll is a case style's name");
  const fields = fieldsOf(columns, renameAll);
  return new Model(name, schema, null, fields, uniqueKeysOf(uniqueKeys, fields), comment, hooksOf(hooks), MADE);
}

const MODEL_OPTIONS = new Set(["columns", "schema", "uniqueKeys", "comment", "renameAll", "hooks"]);
