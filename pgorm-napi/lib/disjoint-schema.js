// @ts-check

// A disjoint model's or registration's statements as the addon gives them:
// the schema `createDisjoint` writes, and the kind changes pgorm's
// `DisjointShape` writes from the declaration as it will be, each a list of
// steps run in order, each step's statements in one transaction, with the
// rows the program writes between two steps where a change needs them.
// [spec:pgorm:req:bindings.disjoint-evolve]

import { arg, TRUSTED } from "./builder.js";
import { Disjoined, handleOf } from "./disjoint-query.js";
import { CHANGE } from "./migration-file.js";
import { native } from "./operations.js";
import { AlterTable } from "./schema-alter.js";
import { CreateTable, statement } from "./schema.js";
import { CreateType } from "./schema-types.js";
import { Table } from "./select.js";

/**
 * Each `[kind, statement]` pair the addon gives, as the class of its kind.
 *
 * @param {[string, unknown][]} pairs
 */
export function statementsOf(pairs) {
  return pairs.map(([kind, handle]) => {
    switch (kind) {
      case "createType":
        return new CreateType(handle, TRUSTED);
      case "createTable":
        return new CreateTable(handle, TRUSTED);
      case "alterTable":
        return new AlterTable(handle, TRUSTED);
      default:
        return statement(handle);
    }
  });
}

/**
 * The kind change `change` of `model`, which the function `name` writes:
 * its steps, each its statements and its gap, frozen.
 *
 * @param {unknown} model
 * @param {string} name
 * @param {string} change
 * @param {unknown} value
 * @param {unknown} [table] The table a kind loses, which only that change takes.
 */
function changed(model, name, change, value, table = null) {
  if (!(model instanceof Disjoined)) throw new TypeError(`${name} takes a Disjoint or a DisjointEntity`);
  if (typeof value !== "string") throw new TypeError("a kind's value is a string");
  const [steps, written] = /** @type {[[[string, unknown][], [string, string | null, string] | null][], unknown]} */ (
    native.disjointChange(handleOf(model), change, value, table === null ? null : arg(table))
  );
  const result = {
    steps: Object.freeze(steps.map(([statements, gap]) =>
      Object.freeze({
        statements: Object.freeze(statementsOf(statements)),
        gap: gap === null
          ? null
          : Object.freeze({ kind: gap[0], table: new Table(gap[2], { schema: gap[1] ?? undefined }) }),
      })
    )),
  };
  Object.defineProperty(result, CHANGE, { value: written });
  return Object.freeze(result);
}

/**
 * Add the kind `value`, which the declaration holds and the database's kind
 * type does not yet: in two steps for a kind with a table, since PostgreSQL
 * uses an added value only once the transaction adding it has committed.
 *
 * @param {unknown} model
 * @param {unknown} value
 */
export function addDisjointKind(model, value) {
  return changed(model, "addDisjointKind", "addKind", value);
}

/**
 * Add the kind `value` in one step that recreates the kind type and rewrites
 * the base and every kind table.
 *
 * @param {unknown} model
 * @param {unknown} value
 */
export function addDisjointKindRecreating(model, value) {
  return changed(model, "addDisjointKindRecreating", "addKindRecreating", value);
}

/**
 * Remove the value `value`, which the database's kind type holds and the
 * declaration does not, in one step that recreates the kind type and fails
 * while any base row has the kind.
 *
 * @param {unknown} model
 * @param {unknown} value
 */
export function removeDisjointKind(model, value) {
  return changed(model, "removeDisjointKind", "removeKind", value);
}

/**
 * Give the kind `value`, a kind without a table in the database, the table
 * the declaration gives it, in two steps with a gap between them for the
 * rows its base rows lack.
 *
 * @param {unknown} model
 * @param {unknown} value
 */
export function addDisjointKindTable(model, value) {
  return changed(model, "addDisjointKindTable", "addKindTable", value);
}

/**
 * Drop `table`, the table of the kind `value`, which the declaration gives
 * none, and the base's generated key columns and foreign key for it, in one
 * step that loses every row of the table.
 *
 * @param {unknown} model
 * @param {unknown} value
 * @param {unknown} table
 */
export function removeDisjointKindTable(model, value, table) {
  return changed(model, "removeDisjointKindTable", "removeKindTable", value, table);
}
