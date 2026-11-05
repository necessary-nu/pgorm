// @ts-check

// What the model modules know of a model: its table, how its columns are
// qualified, and its fields in declaration order. Held apart from the Model
// class so the query, relation and graph modules read it without importing
// the class that builds on them.
// [spec:pgorm:req:bindings.models+5]
// [spec:pgorm:req:bindings.model-unique-keys+2]

import { ConstructionError } from "./errors.js";
import { modelColumn, readsText } from "./model-columns.js";
import { native } from "./operations.js";

/**
 * One field: its property in a record, its SQL column, its declaration.
 *
 * @typedef {object} Field
 * @property {string} field
 * @property {string} sql
 * @property {import("./model-columns.js").Column} column
 */

/**
 * A unique key of a model's table: the constraint's name, or null for
 * PostgreSQL to choose one, its fields in key order, and whether nulls are
 * equal under it.
 *
 * @typedef {object} UniqueKey
 * @property {string | null} name
 * @property {readonly Field[]} fields
 * @property {boolean} nullsNotDistinct
 */

/**
 * A model as the other model modules read it.
 *
 * @typedef {object} Shape
 * @property {object} model The model itself.
 * @property {string} name
 * @property {string | null} schema
 * @property {string | null} alias
 * @property {string} qualifier What its columns are qualified by: its alias, or else its table's name.
 * @property {import("./select.js").Table} table
 * @property {readonly Field[]} fields
 * @property {ReadonlyMap<string, Field>} byField
 * @property {readonly Field[]} primaryKey
 * @property {readonly UniqueKey[]} uniqueKeys
 */

/** @type {WeakMap<object, Shape>} */
const shapes = new WeakMap();

/**
 * @param {object} model
 * @param {Shape} shape
 */
export function register(model, shape) {
  shapes.set(model, shape);
}

/**
 * The shape of `model`, which `model()` must have made.
 *
 * @param {unknown} model
 * @returns {Shape}
 */
export function shapeOf(model) {
  const shape = typeof model === "object" && model !== null ? shapes.get(model) : undefined;
  if (!shape) throw new TypeError("expected a model made by model()");
  return shape;
}

/**
 * The field of `shape` named `name`.
 *
 * @param {Shape} shape
 * @param {unknown} name
 * @returns {Field}
 */
export function fieldOf(shape, name) {
  if (typeof name !== "string") throw new TypeError("a field is named by a string");
  const field = shape.byField.get(name);
  if (!field) throw new ConstructionError(`${shape.name} has no field ${JSON.stringify(name)}`);
  return field;
}

/**
 * A field's column as an expression qualified by `qualifier`, as pgorm
 * qualifies an entity's columns: by the table's alias or its name, not its
 * schema.
 *
 * @param {Field} field
 * @param {string} qualifier
 */
export function qualified(field, qualifier) {
  return modelColumn(native.exprCol(field.sql, qualifier, undefined), field.field, field.column);
}

/**
 * `expr`, a field's column, as a statement projects it under the result
 * column `name`: as it is when that is the column's own name, aliased
 * otherwise, and cast to `text` when the field is read from its text
 * (`readsText`).
 *
 * @param {Field} field
 * @param {import("./expressions.js").Expr} expr
 * @param {string} name
 */
export function projected(field, expr, name) {
  if (readsText(field.column)) return expr.cast("text").as(name);
  return name === field.sql ? expr : expr.as(name);
}

/**
 * The same table, which two shapes are when they name one schema and name.
 *
 * @param {Shape} left
 * @param {Shape} right
 */
export function sameTable(left, right) {
  return left.name === right.name && left.schema === right.schema;
}
