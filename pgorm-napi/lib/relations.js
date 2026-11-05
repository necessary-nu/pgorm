// @ts-check

// Relations between models: columns of one model's table equal to columns
// of another's, which a query joins by, a graph decodes along, and `find`
// and `load` read the far end of; and the paths of more than one relation,
// a junction's (`Related`) and a chain's (`Linked`).
// [spec:pgorm:req:bindings.relations+2]

import { Condition } from "./conditions.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { Expr, tuple } from "./expressions.js";
import { columnValue } from "./model-columns.js";
import { decodeAll } from "./model-decode.js";
import { ModelQuery, query, queryOf, recipeOf } from "./model-queries.js";
import { fieldOf, projected, qualified, sameTable, shapeOf } from "./model-shape.js";
import { native } from "./operations.js";
import { select } from "./select.js";
import { Decimal, Interval, isIdentifier, Uuid } from "./values.js";

/** Marks a construction from a model's relation methods. */
const MADE = Symbol("pgorm-napi relation");

/**
 * The name a load joins its starting model back under, pgorm's loader's for
 * the input entity; a find joins it under its own name unless that is taken.
 */
const SOURCE = "pgorm_loader_src";

/**
 * @typedef {import("./model-shape.js").Shape} Shape
 * @typedef {import("./model-shape.js").Field} Field
 * @typedef {(left: object, right: object) => unknown} OnCondition
 * @typedef {import("./model-queries.js").Recipe["joins"][number]} Joined
 */

/**
 * @typedef {object} Parts
 * @property {Shape} from
 * @property {Shape} to
 * @property {Field[]} fromFields
 * @property {Field[]} toFields
 * @property {OnCondition | null} onCondition
 * @property {"all" | "any"} conditionType
 * @property {string | null} start
 */

/** @type {(relation: Relation) => Parts} */
let partsOf;

/**
 * @param {Shape} shape
 * @param {unknown} names
 * @param {string} side
 */
function keyFields(shape, names, side) {
  const list = typeof names === "string" ? [names] : names;
  if (!Array.isArray(list)) throw new TypeError(`a relation's ${side} is a field or a list of fields`);
  if (list.length === 0) throw new ConstructionError(`a relation's ${side} names at least one field`);
  const fields = list.map((name) => fieldOf(shape, name));
  if (new Set(fields).size !== fields.length) throw new ConstructionError(`a relation's ${side} names each field once`);
  return fields;
}

const KEY_OPTIONS = new Set(["from", "to", "onCondition", "conditionType"]);

/**
 * A relation from one model to another: its kind, the fields of each end
 * that are equal in a related pair of rows, matched in order, and a
 * condition of its own it adds, as pgorm's `on_condition` does. A
 * `belongsTo` or `hasOne` relation reads at most one row at its far end, a
 * `hasMany` any number.
 */
export class Relation {
  /** @type {"belongsTo" | "hasOne" | "hasMany"} */
  #kind;
  /** @type {Parts} */
  #parts;

  /**
   * @param {"belongsTo" | "hasOne" | "hasMany"} kind
   * @param {Parts} parts
   * @param {symbol} token
   */
  constructor(kind, parts, token) {
    if (token !== MADE) throw new TypeError("a Relation comes from a model's belongsTo(), hasOne() or hasMany()");
    this.#kind = kind;
    this.#parts = Object.freeze(parts);
    Object.freeze(this);
  }

  static {
    partsOf = (relation) => relation.#parts;
  }

  get kind() {
    return this.#kind;
  }

  /** The model the relation starts at. */
  get from() {
    return this.#parts.from.model;
  }

  /** The model at its far end. */
  get to() {
    return this.#parts.to.model;
  }

  get fromFields() {
    return Object.freeze(this.#parts.fromFields.map((field) => field.field));
  }

  get toFields() {
    return Object.freeze(this.#parts.toFields.map((field) => field.field));
  }

  /** The condition of its own the relation adds, given the two ends as a join names them, or null. */
  get onCondition() {
    return this.#parts.onCondition;
  }

  /** How the fields' equality and the `onCondition` compose: `"all"` ANDs them, `"any"` ORs them. */
  get conditionType() {
    return this.#parts.conditionType;
  }

  /** Whether a row reads at most one row at the far end, or any number. */
  get cardinality() {
    return this.#kind === "hasMany" ? "many" : "one";
  }

  /**
   * The relation starting from the table a query or a graph reads under
   * `alias`, as pgorm's `RelationDef::fromAlias` points it: how a path goes
   * on from a second visit to one table.
   *
   * @param {string} alias
   */
  fromAlias(alias) {
    if (typeof alias !== "string") throw new TypeError("an alias is a string");
    if (!isIdentifier(alias)) throw new ConstructionError("an alias is 1–63 UTF-8 bytes without NUL");
    return new Relation(this.#kind, { ...this.#parts, start: alias }, MADE);
  }

  /**
   * The rows at the far end related to `row`, a record of the model the
   * relation starts at: none when a key field of it is null. A relation
   * with an `onCondition` joins the model it starts at back, so the
   * condition reads both ends, keyed by its primary key, as pgorm's
   * `find_related` does.
   *
   * @param {unknown} row
   */
  find(row) {
    const { to, toFields, fromFields, onCondition, from } = this.#parts;
    if (onCondition === null) {
      const values = keyValues(fromFields, row);
      const found = query(to, to.fields);
      if (values === null) return found.where(Condition.any());
      return found.where(Condition.all(...toFields.map((field, index) =>
        qualified(field, to.qualifier).eq(values[index])
      )));
    }
    const start = free(from.qualifier, new Set([to.qualifier]));
    const found = queryOf({ ...recipeOf(query(to, to.fields)), joins: [back(this, start, to.qualifier, "inner")] });
    return keyed(found, from, start, startKey(from, fromFields), row);
  }

  /**
   * The far end of each of `rows`, read in one query, as pgorm's `load_one`
   * and `load_many` read it: `options.query`, a query of the far end's
   * model, filtered and ordered as it says, with the model the relation
   * starts at joined back, so the relation is read whole, its `onCondition`
   * included. An array of records per row for `hasMany`, in the query's
   * order; otherwise a record per row, or null where nothing matched or the
   * row's key holds a NULL. Two different rows for one key of a `hasOne` or
   * `belongsTo` relation are a `DecodeError`.
   *
   * @param {unknown} db
   * @param {readonly unknown[]} rows
   * @param {{ query?: unknown, signal?: AbortSignal }} [options]
   */
  async load(db, rows, options) {
    return await load(db, rows, this, [], options, this.#kind === "hasMany");
  }
}

/**
 * The path from one model to another through a junction table, as a Rust
 * `Related` implementation with a `via` declares it: `via` from the model to
 * the junction, then `relation` from the junction to the far end, which
 * reaches at most one row from each junction row.
 */
// [spec:pgorm:req:bindings.related]
export class Related {
  /** @type {Relation} */
  #via;
  /** @type {Relation} */
  #relation;

  /**
   * @param {Relation} relation
   * @param {Relation} via
   * @param {symbol} token
   */
  constructor(relation, via, token) {
    if (token !== MADE) throw new TypeError("a Related comes from a model's related()");
    this.#relation = relation;
    this.#via = via;
    Object.freeze(this);
  }

  /** The relation from the model the path starts at to the junction. */
  get via() {
    return this.#via;
  }

  /** The relation from the junction to the far end. */
  get relation() {
    return this.#relation;
  }

  /** The model the path starts at. */
  get from() {
    return this.#via.from;
  }

  /** The model at its far end. */
  get to() {
    return this.#relation.to;
  }

  get cardinality() {
    return "many";
  }

  /**
   * The rows at the far end related to `row` through the junction: the far
   * end's query joined to the junction and to the model the path starts at,
   * keyed by that model's primary key, as pgorm's `find_related` writes it;
   * none when one of the row's key fields is null.
   *
   * @param {unknown} row
   */
  find(row) {
    const via = partsOf(this.#via);
    const { to } = partsOf(this.#relation);
    const junction = free(partsOf(this.#relation).from.qualifier, new Set([to.qualifier]));
    const start = free(via.from.qualifier, new Set([to.qualifier, junction]));
    const found = queryOf({
      ...recipeOf(query(to, to.fields)),
      joins: [back(this.#relation, junction, to.qualifier, "inner"), back(this.#via, start, junction, "inner")],
    });
    return keyed(found, via.from, start, startKey(via.from, via.fromFields), row);
  }

  /**
   * The far end of each of `rows` through the junction, read in one query,
   * as pgorm's `load_many_via` reads it: `options.query` of the far end with
   * the junction and the model the path starts at joined back. An array of
   * records per row, in the query's order, an empty one where nothing
   * matched or the row's key holds a NULL.
   *
   * @param {unknown} db
   * @param {readonly unknown[]} rows
   * @param {{ query?: unknown, signal?: AbortSignal }} [options]
   */
  async load(db, rows, options) {
    return await load(db, rows, this.#via, [this.#relation], options, true);
  }
}

/**
 * A chain of relations from one model to another, each starting where the
 * one before it ends, as a Rust `Linked` type lists them. A query along it
 * joins every hop under an alias of its own, `r0` the last relation's start,
 * `r1` the one before, as pgorm's `find_linked` does, so a chain may pass
 * through one table more than once.
 */
// [spec:pgorm:req:bindings.linked]
export class Linked {
  /** @type {readonly Relation[]} */
  #relations;

  /**
   * @param {readonly Relation[]} relations
   * @param {symbol} token
   */
  constructor(relations, token) {
    if (token !== MADE) throw new TypeError("a Linked comes from a model's linked()");
    this.#relations = Object.freeze([...relations]);
    Object.freeze(this);
  }

  get relations() {
    return this.#relations;
  }

  /** The model the chain starts at. */
  get from() {
    return /** @type {Relation} */ (this.#relations[0]).from;
  }

  /** The model at its far end. */
  get to() {
    return /** @type {Relation} */ (this.#relations.at(-1)).to;
  }

  /**
   * The model a query along the chain reads at hop `index`, under the alias
   * pgorm's `LinkedAlias::hop` names it by: `r0` the last relation's start,
   * counting back towards the chain's own.
   *
   * @param {number} index
   */
  hop(index) {
    if (!Number.isInteger(index)) throw new TypeError("a hop is its index");
    const relation = this.#relations[this.#relations.length - 1 - index];
    if (index < 0 || relation === undefined) throw new ConstructionError(`the chain has no hop ${index}`);
    return /** @type {{ as(alias: string): object }} */ (relation.from).as(`r${index}`);
  }

  /**
   * The model the chain starts at, under the alias pgorm's `last_hop_alias`
   * names: what a condition on the starting row names.
   */
  lastHop() {
    return this.hop(this.#relations.length - 1);
  }

  /**
   * Every row at the chain's far end that a chain of rows reaches, read along
   * each hop by an INNER JOIN, as pgorm's `Linked::find_linked` writes it.
   */
  findLinked() {
    const { to } = partsOf(/** @type {Relation} */ (this.#relations.at(-1)));
    /** @type {Joined[]} */
    const joins = [];
    let right = to.qualifier;
    [...this.#relations].reverse().forEach((relation, index) => {
      const left = `r${index}`;
      joins.push(back(relation, left, right, "inner"));
      right = left;
    });
    return queryOf({ ...recipeOf(query(to, to.fields)), joins });
  }

  /**
   * The rows at the far end the chain reaches from `row`, a record of the
   * model it starts at, keyed by that model's primary key, as pgorm's
   * `ModelTrait::find_linked` keys it: none when one of the row's key
   * fields is null.
   *
   * @param {unknown} row
   */
  find(row) {
    const first = partsOf(/** @type {Relation} */ (this.#relations[0]));
    return keyed(this.findLinked(), first.from, `r${this.#relations.length - 1}`, startKey(first.from, first.fromFields), row);
  }
}

/**
 * The values of `fields` in `row`, or null when one of them is null.
 *
 * @param {readonly Field[]} fields
 * @param {unknown} row
 * @returns {unknown[] | null}
 */
function keyValues(fields, row) {
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
 * @param {Relation} relation
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
  if (!(relation instanceof Relation)) throw new TypeError("expected a Relation");
  const { from, to } = partsOf(relation);
  return [from, to];
}

/**
 * The name of the table a relation starts from, when `fromAlias` gave it
 * one, or null.
 *
 * @param {Relation} relation
 */
export function startOf(relation) {
  return partsOf(relation).start;
}

/**
 * `qualifier`, or `SOURCE` where a table the read has answers to it.
 *
 * @param {string} qualifier
 * @param {Set<string>} taken
 */
function free(qualifier, taken) {
  if (!taken.has(qualifier)) return qualifier;
  if (taken.has(SOURCE)) throw new ConstructionError(`the read already has a table called ${JSON.stringify(SOURCE)}`);
  return SOURCE;
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
function back(relation, left, right, kind) {
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
function startKey(shape, fields) {
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
function keyed(found, shape, qualifier, fields, row) {
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
async function load(db, rows, first, hops, options, many) {
  if (!Array.isArray(rows)) throw new TypeError("load takes an array of records");
  const { query: given, ...operation } = options ?? {};
  const far = partsOf(/** @type {Relation} */ (hops.at(-1) ?? first)).to;
  let found;
  if (given === undefined) {
    found = query(far, far.fields);
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

/**
 * The relation options `keys` gives: `from` and `to`, and the relation's
 * `onCondition` and `conditionType`.
 *
 * @param {unknown} keys
 */
function optionsOf(keys) {
  if (typeof keys !== "object" || keys === null) throw new TypeError("a relation's keys are { from, to }");
  for (const option of Object.keys(keys)) {
    if (!KEY_OPTIONS.has(option)) throw new TypeError(`${JSON.stringify(option)} is no relation option`);
  }
  const { from, to, onCondition = null, conditionType = "all" } =
    /** @type {{ from?: unknown, to?: unknown, onCondition?: unknown, conditionType?: unknown }} */ (keys);
  if (onCondition !== null && typeof onCondition !== "function") {
    throw new TypeError("a relation's onCondition is a function of the two ends, giving a condition");
  }
  if (conditionType !== "all" && conditionType !== "any") throw new TypeError('a relation\'s conditionType is "all" or "any"');
  return { from, to, onCondition: /** @type {OnCondition | null} */ (onCondition), conditionType: /** @type {"all" | "any"} */ (conditionType) };
}

/**
 * A relation of `kind` from `from` to `to`.
 *
 * @param {"belongsTo" | "hasOne" | "hasMany"} kind
 * @param {object} from
 * @param {unknown} to
 * @param {unknown} keys
 */
export function relation(kind, from, to, keys) {
  const options = optionsOf(keys);
  const origin = shapeOf(from);
  const target = shapeOf(to);
  const fromFields = keyFields(origin, options.from, "from");
  const toFields = keyFields(target, options.to, "to");
  if (fromFields.length !== toFields.length) throw new ConstructionError("a relation pairs as many fields at each end");
  return new Relation(kind, {
    from: origin,
    to: target,
    fromFields,
    toFields,
    onCondition: options.onCondition,
    conditionType: options.conditionType,
    start: null,
  }, MADE);
}

/**
 * The path from `origin` through a junction: `via` from it to the junction,
 * then `target` from the junction to the far end.
 *
 * @param {object} origin
 * @param {unknown} target
 * @param {unknown} options
 */
export function related(origin, target, options) {
  if (typeof options !== "object" || options === null) throw new TypeError("a related path takes { via }");
  for (const option of Object.keys(options)) {
    if (option !== "via") throw new TypeError(`${JSON.stringify(option)} is no related option`);
  }
  const { via } = /** @type {{ via?: unknown }} */ (options);
  if (!(target instanceof Relation) || !(via instanceof Relation)) throw new TypeError("a related path is a Relation through a via Relation");
  const path = partsOf(target);
  const hop = partsOf(via);
  const start = shapeOf(origin);
  if (path.start !== null || hop.start !== null) throw new ConstructionError("a related path's relations start from their own tables");
  if (!sameTable(hop.from, start)) throw new ConstructionError(`the via relation starts at ${hop.from.name}, not ${start.name}`);
  if (!sameTable(path.from, hop.to)) {
    throw new ConstructionError(`the relation starts at ${path.from.name}, where the via relation reaches ${hop.to.name}`);
  }
  if (target.cardinality !== "one") {
    throw new ConstructionError("a junction's relation to the far end reaches one row, a belongsTo or a hasOne");
  }
  return new Related(target, via, MADE);
}

/**
 * A chain of `relations` from `origin`, each starting where the one before
 * it ends.
 *
 * @param {object} origin
 * @param {readonly unknown[]} relations
 */
export function linked(origin, relations) {
  if (relations.length === 0) throw new ConstructionError("a linked chain has at least one relation");
  if (!relations.every((relation) => relation instanceof Relation)) throw new TypeError("a linked chain is made of Relations");
  const chain = /** @type {Relation[]} */ (relations);
  if (chain.some((relation) => partsOf(relation).start !== null)) {
    throw new ConstructionError("a linked chain's relations start from their own tables");
  }
  const start = shapeOf(origin);
  const first = partsOf(/** @type {Relation} */ (chain[0]));
  if (!sameTable(first.from, start)) {
    throw new ConstructionError(`the chain's first relation starts at ${first.from.name}, not ${start.name}`);
  }
  chain.slice(1).forEach((relation, index) => {
    const before = partsOf(/** @type {Relation} */ (chain[index]));
    const after = partsOf(relation);
    if (!sameTable(after.from, before.to)) {
      throw new ConstructionError(`relation ${index + 1} starts at ${after.from.name}, where the one before it reaches ${before.to.name}`);
    }
  });
  return new Linked(chain, MADE);
}
