// @ts-check

// What the query and graph modules read of a relation: its two ends' shapes
// and the fields each pairs. Held apart from the Relation class, whose `find`
// builds a model query, so the module the queries build on reads a relation
// without importing it.
// [spec:pgorm:req:bindings.relations+2]

import { Condition } from "./conditions.js";
import { qualified } from "./model-shape.js";

/**
 * A relation's two ends and the fields each pairs, matched in order.
 *
 * @typedef {object} Parts
 * @property {import("./model-shape.js").Shape} from
 * @property {import("./model-shape.js").Shape} to
 * @property {readonly import("./model-shape.js").Field[]} fromFields
 * @property {readonly import("./model-shape.js").Field[]} toFields
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
 * The join condition of `relation` between the tables answering to
 * `left` and `right`: each pair of its fields equal, as pgorm's
 * `join_condition` writes a relation's columns.
 *
 * @param {unknown} relation
 * @param {string} left
 * @param {string} right
 */
export function relationOn(relation, left, right) {
  const { fromFields, toFields } = partsOf(relation);
  return Condition.all(Condition.all(...fromFields.map((field, index) =>
    qualified(field, left).eq(qualified(/** @type {import("./model-shape.js").Field} */ (toFields[index]), right))
  )));
}

/**
 * The shapes at a relation's two ends.
 *
 * @param {unknown} relation
 * @returns {[import("./model-shape.js").Shape, import("./model-shape.js").Shape]}
 */
export function shapesOf(relation) {
  const { from, to } = partsOf(relation);
  return [from, to];
}
