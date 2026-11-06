// @ts-check

// What the query and graph modules read of a relation: its two ends' shapes,
// the fields each pairs, its own condition and where it starts. Held apart
// from the Relation class, whose `find` builds a model query, so the modules
// the queries build on read a relation without importing it.
// [spec:pgorm:req:bindings.relations+3]

import { Condition } from "./conditions.js";
import { Expr } from "./expressions.js";
import { qualified } from "./model-shape.js";

/**
 * @typedef {import("./model-shape.js").Shape} Shape
 * @typedef {import("./model-shape.js").Field} Field
 * @typedef {(left: object, right: object) => unknown} OnCondition
 */

/**
 * A relation's two ends and the fields each pairs, matched in order; the
 * condition of its own it adds and how the two compose; the table a
 * `fromAlias` starts it from; and the actions and name of the foreign key a
 * `belongsTo` relation holds.
 *
 * @typedef {object} Parts
 * @property {Shape} from
 * @property {Shape} to
 * @property {Field[]} fromFields
 * @property {Field[]} toFields
 * @property {OnCondition | null} onCondition
 * @property {"all" | "any"} conditionType
 * @property {string | null} start
 * @property {string | null} onDelete
 * @property {string | null} onUpdate
 * @property {string | null} fkName
 */

/** @type {WeakMap<object, Parts>} */
const parts = new WeakMap();

/**
 * @param {object} relation
 * @param {Parts} relationParts
 */
export function registerRelation(relation, relationParts) {
  parts.set(relation, relationParts);
}

/**
 * The parts of `relation`, which a model's relation method must have made.
 *
 * @param {unknown} relation
 * @returns {Parts}
 */
export function partsOf(relation) {
  const found = typeof relation === "object" && relation !== null ? parts.get(relation) : undefined;
  if (!found) throw new TypeError("expected a Relation");
  return found;
}

/**
 * `shape`'s model answering to `qualifier`: itself when that is its name,
 * otherwise aliased to it.
 *
 * @param {Shape} shape
 * @param {string} qualifier
 * @returns {object}
 */
export function answering(shape, qualifier) {
  return shape.qualifier === qualifier ? shape.model : /** @type {{ as(alias: string): object }} */ (shape.model).as(qualifier);
}

/**
 * `value`, a condition a function gave, which is an expression or a
 * `Condition`.
 *
 * @param {unknown} value
 * @param {string} what
 * @returns {Expr | Condition}
 */
export function conditionOf(value, what) {
  if (!(value instanceof Expr) && !(value instanceof Condition)) throw new TypeError(`${what} gives an expression or a Condition`);
  return value;
}

/**
 * The join condition of `relation` between the tables answering to `left`
 * and `right`, as pgorm's `join_condition` writes it: each pair of its
 * fields equal, the pairs ANDed, then its `onCondition` given the two ends
 * as they are named, the two composed by its `conditionType`.
 *
 * @param {unknown} relation
 * @param {string} left
 * @param {string} right
 */
export function relationOn(relation, left, right) {
  const { from, to, fromFields, toFields, onCondition, conditionType } = partsOf(relation);
  /** @type {(Expr | Condition)[]} */
  const parts = [Condition.all(...fromFields.map((field, index) =>
    qualified(field, left).eq(qualified(/** @type {Field} */ (toFields[index]), right))
  ))];
  if (onCondition !== null) {
    parts.push(conditionOf(onCondition(answering(from, left), answering(to, right)), "a relation's onCondition"));
  }
  return conditionType === "any" ? Condition.any(...parts) : Condition.all(...parts);
}

/**
 * The shapes at a relation's two ends.
 *
 * @param {unknown} relation
 * @returns {[Shape, Shape]}
 */
export function shapesOf(relation) {
  const { from, to } = partsOf(relation);
  return [from, to];
}

/**
 * The name of the table a relation starts from, when `fromAlias` gave it
 * one, or null.
 *
 * @param {unknown} relation
 */
export function startOf(relation) {
  return partsOf(relation).start;
}
