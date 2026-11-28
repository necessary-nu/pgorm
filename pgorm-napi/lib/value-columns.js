// @ts-check

// How the addon builds a statement's result a column at a time
// (`src/rows.rs`): each column's array made from blocks of its values, and
// the values that crossed in the form they are built from built in one loop
// per column.
// [spec:pgorm:req:napi.values+2]
// [spec:pgorm:req:napi.records]

import { Decimal, revive, TRUSTED, Uuid } from "./values.js";

/**
 * `values` as an array when `array` is null, or appended to `array`: how the
 * addon makes a result's column, a block of values per call.
 *
 * @param {unknown[] | null} array
 * @param {...unknown} values
 */
export function gather(array, ...values) {
  if (array === null) return values;
  for (const value of values) array.push(value);
  return array;
}

/**
 * Build, in place, the values of each column of a result that crossed in the
 * form they are built from, as `builds` names it: the text of a `Decimal` or
 * a `Uuid`, an instant's nanoseconds, or the text of a JSON array of the
 * column's documents, split into segments, which a reviver reads where a
 * document holds an integer past `Number.MAX_SAFE_INTEGER` (`json-wide`).
 *
 * @param {unknown[][]} columns
 * @param {string[]} builds
 */
export function finish(columns, builds) {
  for (let at = 0; at < columns.length; at++) {
    const built = builds[at];
    if (built === "") continue;
    const column = /** @type {any[]} */ (columns[at]);
    if (built === "json" || built === "json-wide") {
      const parse = built === "json"
        ? (/** @type {string} */ text) => JSON.parse(text)
        : (/** @type {string} */ text) => JSON.parse(text, /** @type {any} */ (revive));
      columns[at] = column.length === 1 ? parse(column[0]) : column.flatMap(parse);
      continue;
    }
    for (let row = 0; row < column.length; row++) {
      const value = column[row];
      if (value === null) continue;
      column[row] = built === "decimal"
        ? new Decimal(value, TRUSTED)
        : built === "uuid"
        ? new Uuid(value, TRUSTED)
        : new Temporal.Instant(value);
    }
  }
}
