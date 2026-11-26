// @ts-check

// Reading along a relation: the tables a find joins back, each held to the
// scopes its context holds of it, the narrowing of a find to one row's key,
// and a load's one read of the far end for every row's key, its rows
// bucketed back by key. Held apart from the classes declaring relations,
// whose finds and loads build on it.
// [spec:pgorm:req:bindings.relations+3]

import { Condition } from "./conditions.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { tuple } from "./expressions.js";
import { columnValue } from "./model-columns.js";
import { decodeAll } from "./model-decode.js";
import { recipeOf } from "./model-clauses.js";
import { ModelQuery, query } from "./model-queries.js";
import { heldOn, projected, qualified, sameTable } from "./model-shape.js";
import { native } from "./operations.js";
import { partsOf, relationOn } from "./relation-parts.js";
import { select } from "./select.js";
import { Decimal, Interval, Uuid } from "./values.js";

/**
 * @typedef {import("./model-shape.js").Shape} Shape
 * @typedef {import("./model-shape.js").Field} Field
 * @typedef {import("./model-queries.js").Recipe["joins"][number]} Joined
 * @typedef {import("./relations.js").Relation} Relation
 */

/**
 * The name a load joins its starting model back under, pgorm's loader's for
 * the input entity; a find joins it under its own name unless that is taken.
 */
const SOURCE = "pgorm_loader_src";

/**
 * The values of `fields` in `row`, or null when one of them is null.
 *
 * @param {readonly Field[]} fields
 * @param {unknown} row
 * @returns {unknown[] | null}
 */
export function keyValues(fields, row) {
  if (typeof row !== "object" || row === null) throw new TypeError("a relation reads a record of the model it starts at");
  const values = fields.map((field) => {
    if (!Object.hasOwn(row, field.field)) throw new ConstructionError(`the record has no ${field.field}`);
    return /** @type {Record<string, unknown>} */ (row)[field.field];
  });
  return values.some((value) => value === null) ? null : values;
}

/**
 * A key as text two equal keys share: integers whatever their JavaScript
 * type, and the module's values and Temporal's by their text.
 *
 * @param {readonly unknown[]} key
 */
export function keyText(key) {
  return JSON.stringify(key.map((value) => {
    if (typeof value === "bigint" || typeof value === "number") return `n:${value}`;
    if (typeof value === "string") return `s:${value}`;
    if (value instanceof Uint8Array) return `b:${Array.from(value, (byte) => byte.toString(16).padStart(2, "0")).join("")}`;
    if (value instanceof Decimal || value instanceof Uuid || value instanceof Interval) return `v:${value}`;
    return `o:${String(value)}`;
  }));
}

/**
 * `qualifier`, or `SOURCE` where a table the read has answers to it.
 *
 * @param {string} qualifier
 * @param {Set<string>} taken
 */
export function free(qualifier, taken) {
  if (!taken.has(qualifier)) return qualifier;
  if (taken.has(SOURCE)) throw new ConstructionError(`the read already has a table called ${JSON.stringify(SOURCE)}`);
  return SOURCE;
}

// [spec:pgorm:req:napi.scopes]
/**
 * `join` held to its table's scopes as `held` holds them, ANDed into its
 * `ON`: refused where its table is scoped and `held` is `null`.
 *
 * @param {import("./model-shape.js").Held} held
 * @param {Joined} join
 * @returns {Joined}
 */
export function scoped(held, join) {
  const condition = heldOn(join.shape, held, join.qualifier);
  return condition === null ? join : { ...join, on: Condition.all(join.on, condition) };
}

/**
 * The relation's starting table joined under `left` from the table
 * answering to `right`, its far end: the relation walked backwards, its
 * condition written in the roles it was declared with.
 *
 * @param {Relation} relation
 * @param {string} left
 * @param {string} right
 * @param {"inner" | "left"} kind
 * @returns {Joined}
 */
export function back(relation, left, right, kind) {
  const { from } = partsOf(relation);
  const table = left === from.qualifier ? from.table : from.table.as(left);
  return { table, on: relationOn(relation, left, right), kind, shape: from, qualifier: left };
}

/**
 * The fields a find joining its starting model keys the row by: the model's
 * primary key, as pgorm keys a model, or where it declares none, the fields
 * its path starts from.
 *
 * @param {Shape} shape
 * @param {readonly Field[]} fields
 */
export function startKey(shape, fields) {
  return shape.primaryKey.length > 0 ? shape.primaryKey : fields;
}

/**
 * `found` narrowed to the rows whose `fields` of `shape`, answering to
 * `qualifier`, are `row`'s: none when one of the row's is null.
 *
 * @param {ModelQuery} found
 * @param {Shape} shape
 * @param {string} qualifier
 * @param {readonly Field[]} fields
 * @param {unknown} row
 */
export function keyed(found, shape, qualifier, fields, row) {
  const values = keyValues(fields, row);
  if (values === null) return found.where(Condition.any());
  return found.where(Condition.all(...fields.map((field, index) => qualified(field, qualifier).eq(values[index]))));
}

/**
 * `fields`, answering to `qualifier`, among `keys`: an IN list of one
 * column, or of a row of the key's columns.
 *
 * @param {readonly Field[]} fields
 * @param {string} qualifier
 * @param {unknown[][]} keys
 */
function among(fields, qualifier, keys) {
  const columns = fields.map((field) => qualified(field, qualifier));
  const bound = keys.map((key) => key.map((value, index) => {
    const field = /** @type {Field} */ (fields[index]);
    return columnValue(field.column, field.field, value);
  }));
  if (columns.length === 1) return /** @type {import("./model-columns.js").ModelColumn} */ (columns[0]).isIn(bound.map((key) => key[0]));
  return tuple(...columns).isIn(bound.map((key) => tuple(...key)));
}

/**
 * The one read a load runs, as pgorm's loader builds it: the far end's query
 * re-rooted, each hop walked backwards from it, and the model the path starts
 * at joined back under `SOURCE` as the one required source, whose key fields
 * carry each row's key out of the join.
 *
 * @param {import("./model-queries.js").Recipe} recipe
 * @param {Relation} first
 * @param {readonly Relation[]} hops
 * @param {unknown[][]} keys
 */
function loader(recipe, first, hops, keys) {
  const root = recipe.shape;
  const joins = [...recipe.joins];
  const taken = new Set([root.qualifier, ...joins.map((join) => join.qualifier)]);
  let right = root.qualifier;
  for (const relation of [...hops].reverse()) {
    let left = partsOf(relation).from.qualifier;
    if (taken.has(left)) left = "pgorm_loader_via";
    joins.push(back(relation, left, right, "left"));
    taken.add(left);
    right = left;
  }
  if (taken.has(SOURCE)) throw new ConstructionError(`a loaded query cannot read a table called ${JSON.stringify(SOURCE)}`);
  joins.push(back(first, SOURCE, right, "inner"));
  const { fromFields } = partsOf(first);
  const targetNames = recipe.fields.map((field) => native.modelResultName("s0_", field.sql));
  const sourceNames = fromFields.map((field) => native.modelResultName("s1_", field.sql));
  let statement = select(
    ...recipe.fields.map((field, index) => projected(field, qualified(field, root.qualifier), /** @type {string} */ (targetNames[index]))),
    ...fromFields.map((field, index) => projected(field, qualified(field, SOURCE), /** @type {string} */ (sourceNames[index]))),
  ).from(root.table);
  for (const join of joins) statement = statement.join(join.table, join.on, { kind: join.kind });
  for (const where of [...recipe.wheres, among(fromFields, SOURCE, keys)]) statement = statement.where(where);
  if (recipe.orders.length > 0) statement = statement.orderBy(...recipe.orders);
  if (recipe.limit !== null) statement = statement.limit(recipe.limit);
  if (recipe.offset !== null) statement = statement.offset(recipe.offset);
  return {
    statement,
    sources: [
      { fields: recipe.fields, names: targetNames, optional: false },
      { fields: fromFields, names: sourceNames, optional: false },
    ],
  };
}

/**
 * @param {unknown} db
 * @param {readonly unknown[]} rows
 * @param {Relation} first
 * @param {readonly Relation[]} hops
 * @param {{ query?: unknown, signal?: AbortSignal } | undefined} options
 * @param {boolean} many
 */
export async function load(db, rows, first, hops, options, many) {
  if (!Array.isArray(rows)) throw new TypeError("load takes an array of records");
  const { query: given, ...operation } = options ?? {};
  const far = partsOf(/** @type {Relation} */ (hops.at(-1) ?? first)).to;
  let found;
  if (given === undefined) {
    found = query(far, far.fields, null);
  } else if (!(given instanceof ModelQuery)) {
    throw new TypeError("load's query is a query of the far end's model");
  } else {
    found = given;
    const shape = recipeOf(found).shape;
    if (!sameTable(shape, far)) throw new ConstructionError(`load reads ${far.name}, and the query reads ${shape.name}`);
  }
  const { fromFields } = partsOf(first);
  const keys = rows.map((row) => keyValues(fromFields, row));
  /** @type {Map<string, unknown[]>} */
  const wanted = new Map();
  for (const key of keys) if (key !== null) wanted.set(keyText(key), key);
  /** @type {Map<string, Record<string, unknown>[]>} */
  const buckets = new Map([...wanted.keys()].map((text) => [text, []]));
  if (wanted.size > 0) {
    const { statement, sources } = loader(recipeOf(found), first, hops, [...wanted.values()]);
    for (const [target, source] of await decodeAll(db, statement, sources, operation)) {
      const text = keyText(fromFields.map((field) => source[field.field]));
      const bucket = buckets.get(text);
      if (bucket === undefined) {
        throw new DecodeError(`a loaded row's key ${text}, read back from ${fromFields.map((field) => field.field).join(", ")}, equals none of the rows' keys`);
      }
      bucket.push(target);
    }
  }
  if (many) return keys.map((key) => key === null ? [] : [.../** @type {Record<string, unknown>[]} */ (buckets.get(keyText(key)))]);
  return keys.map((key) => key === null ? null : single(/** @type {Record<string, unknown>[]} */ (buckets.get(keyText(key))), key, far));
}

/**
 * The one row a key of a relation to one row reached: repeats of one row,
 * which joining the starting model back makes, collapse, counted by the far
 * end's primary key where the query reads it.
 *
 * @param {Record<string, unknown>[]} bucket
 * @param {unknown[]} key
 * @param {Shape} far
 */
function single(bucket, key, far) {
  /** @type {Map<string, Record<string, unknown>>} */
  const distinct = new Map();
  for (const record of bucket) {
    const fields = far.primaryKey.length > 0 && far.primaryKey.every((field) => Object.hasOwn(record, field.field))
      ? far.primaryKey.map((field) => field.field)
      : Object.keys(record);
    const text = keyText(fields.map((field) => record[field]));
    if (!distinct.has(text)) distinct.set(text, record);
  }
  if (distinct.size > 1) throw new DecodeError(`${distinct.size} ${far.name} rows have the key ${keyText(key)}, where one is`);
  return distinct.values().next().value ?? null;
}
