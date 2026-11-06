// @ts-check

// Declared models generated from DDL: pgorm-codegen's TypeScript writer,
// which reads a schema as it generates Rust entities and writes the module of
// `model(..)` declarations that describe it. The reading and the writing are
// the addon's, so a schema reads here exactly as it does for Rust.
// [spec:pgorm:req:bindings.models-from-sql]

import { native } from "./operations.js";

/**
 * A table as the addon takes it, `[schema, name]`.
 *
 * @param {unknown} table
 * @param {string} what
 * @returns {[string | null, string]}
 */
function tableOf(table, what) {
  if (typeof table === "string") return [null, table];
  if (Array.isArray(table) && table.length === 2 && table.every((part) => typeof part === "string")) {
    return [table[0], table[1]];
  }
  throw new TypeError(`${what} is a table's name, or a [schema, name] pair`);
}

/**
 * The TypeScript module declaring the models of the tables `ddl` creates,
 * as pgorm-codegen reads them for Rust entities.
 *
 * @param {string} ddl
 * @param {{ schemaName?: string | null, unkindedDisjoint?: readonly { base: unknown, kinds: readonly unknown[], unit?: string | null }[] }} [options]
 * @returns {string}
 */
export function modelsFromSql(ddl, options = {}) {
  if (typeof ddl !== "string") throw new TypeError("modelsFromSql reads DDL text, a string");
  const { schemaName = null, unkindedDisjoint = [], ...unknown } = options;
  const extra = Object.keys(unknown);
  if (extra.length > 0) throw new TypeError(`modelsFromSql takes no option ${extra[0]}`);
  if (schemaName !== null && typeof schemaName !== "string") throw new TypeError("a schemaName is a string");
  if (!Array.isArray(unkindedDisjoint)) throw new TypeError("unkindedDisjoint is an array of table sets");
  const sets = unkindedDisjoint.map((set) => {
    const { base, kinds, unit = null } = set;
    if (!Array.isArray(kinds)) throw new TypeError("an unkinded disjoint set's kinds are an array of tables");
    if (unit !== null && typeof unit !== "string") throw new TypeError("an unkinded disjoint set's unit is a string");
    return [
      tableOf(base, "an unkinded disjoint set's base"),
      kinds.map((kind) => tableOf(kind, "an unkinded disjoint set's kind")),
      unit,
    ];
  });
  return /** @type {string} */ (native.modelsFromSql(ddl, schemaName, sets));
}
