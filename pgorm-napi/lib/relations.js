// @ts-check

// Relations between models: columns of one model's table equal to columns
// of another's, which a query joins by, a graph decodes along, and `find`
// and `load` read the far end of; and the paths of more than one relation,
// a junction's (`Related`) and a chain's (`Linked`).
// [spec:pgorm:req:bindings.relations+3]

import { Condition } from "./conditions.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { recipeOf } from "./model-clauses.js";
import { ModelQuery, query, queryOf } from "./model-queries.js";
import { fieldOf, qualified, sameTable, shapeOf } from "./model-shape.js";
import { partsOf, registerRelation } from "./relation-parts.js";
import { back, free, keyed, keyValues, load, scoped, startKey } from "./relation-reads.js";
import { heldOf } from "./scopes.js";
import { isIdentifier } from "./values.js";

export { answering, conditionOf, relationOn, shapesOf, startOf } from "./relation-parts.js";

/** Marks a construction from a model's relation methods. */
const MADE = Symbol("pgorm-napi relation");

/**
 * @typedef {import("./model-shape.js").Shape} Shape
 * @typedef {import("./model-shape.js").Field} Field
 * @typedef {import("./relation-parts.js").OnCondition} OnCondition
 * @typedef {import("./relation-parts.js").Parts} Parts
 * @typedef {import("./model-queries.js").Recipe["joins"][number]} Joined
 */

/** A foreign key's referential actions, as the schema builders spell them. */
const ACTIONS = new Set(["noAction", "restrict", "cascade", "setNull", "setDefault"]);

/**
 * @param {unknown} action
 * @param {string} what
 * @returns {string | null}
 */
function actionOf(action, what) {
  if (action === undefined || action === null) return null;
  if (typeof action !== "string" || !ACTIONS.has(action)) {
    throw new TypeError(`a relation's ${what} is one of ${[...ACTIONS].join(", ")}`);
  }
  return action;
}

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

const KEY_OPTIONS = new Set(["from", "to", "onCondition", "conditionType", "onDelete", "onUpdate", "fkName"]);

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
    registerRelation(this, this.#parts);
    Object.freeze(this);
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

  /** The action of the foreign key a `belongsTo` relation holds, `ON DELETE`. */
  get onDelete() {
    return this.#parts.onDelete;
  }

  /** The action of the foreign key a `belongsTo` relation holds, `ON UPDATE`. */
  get onUpdate() {
    return this.#parts.onUpdate;
  }

  /** The name of the foreign key a `belongsTo` relation holds, or null for pgorm's `fk-{table}-{columns}`. */
  get fkName() {
    return this.#parts.fkName;
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
    return this.#find(null, row);
  }

  /**
   * Rust's `find_related_for`: `find` under `cx`, the far end's scopes in
   * the `WHERE` and a table joined back held to its own in its join's `ON`.
   *
   * @param {unknown} cx
   * @param {unknown} row
   */
  findFor(cx, row) {
    return this.#find(heldOf(cx), row);
  }

  /**
   * @param {import("./model-shape.js").Held} held
   * @param {unknown} row
   */
  #find(held, row) {
    const { to, toFields, fromFields, onCondition, from } = this.#parts;
    if (onCondition === null) {
      const values = keyValues(fromFields, row);
      const found = query(to, to.fields, held);
      if (values === null) return found.where(Condition.any());
      return found.where(Condition.all(...toFields.map((field, index) =>
        qualified(field, to.qualifier).eq(values[index])
      )));
    }
    const start = free(from.qualifier, new Set([to.qualifier]));
    const found = queryOf({ ...recipeOf(query(to, to.fields, held)), joins: [scoped(held, back(this, start, to.qualifier, "inner"))] });
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
    return this.#find(null, row);
  }

  /**
   * Rust's `find_related_for`: `find` under `cx`, the far end's scopes in
   * the `WHERE`, and the junction's and the starting model's in their
   * joins' `ON`.
   *
   * @param {unknown} cx
   * @param {unknown} row
   */
  findFor(cx, row) {
    return this.#find(heldOf(cx), row);
  }

  /**
   * @param {import("./model-shape.js").Held} held
   * @param {unknown} row
   */
  #find(held, row) {
    const via = partsOf(this.#via);
    const { to } = partsOf(this.#relation);
    const junction = free(partsOf(this.#relation).from.qualifier, new Set([to.qualifier]));
    const start = free(via.from.qualifier, new Set([to.qualifier, junction]));
    const found = queryOf({
      ...recipeOf(query(to, to.fields, held)),
      joins: [
        scoped(held, back(this.#relation, junction, to.qualifier, "inner")),
        scoped(held, back(this.#via, start, junction, "inner")),
      ],
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
// [spec:pgorm:req:bindings.linked+1]
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
    return this.#linked(null);
  }

  /**
   * Rust's `find_linked_for`: `findLinked` under `cx`, the far end's scopes
   * in the `WHERE` and each hop's in its join's `ON`.
   *
   * @param {unknown} cx
   */
  findLinkedFor(cx) {
    return this.#linked(heldOf(cx));
  }

  /** @param {import("./model-shape.js").Held} held */
  #linked(held) {
    const { to } = partsOf(/** @type {Relation} */ (this.#relations.at(-1)));
    /** @type {Joined[]} */
    const joins = [];
    let right = to.qualifier;
    [...this.#relations].reverse().forEach((relation, index) => {
      const left = `r${index}`;
      joins.push(scoped(held, back(relation, left, right, "inner")));
      right = left;
    });
    return queryOf({ ...recipeOf(query(to, to.fields, held)), joins });
  }

  /**
   * The rows at the far end the chain reaches from `row`, a record of the
   * model it starts at, keyed by that model's primary key, as pgorm's
   * `pgorm::Model::find_linked` keys it: none when one of the row's key
   * fields is null.
   *
   * @param {unknown} row
   */
  find(row) {
    return this.#found(this.#linked(null), row);
  }

  /**
   * Rust's `pgorm::Model::find_linked_for`: `find` under `cx`, each table it
   * reads held to its scopes.
   *
   * @param {unknown} cx
   * @param {unknown} row
   */
  findFor(cx, row) {
    return this.#found(this.#linked(heldOf(cx)), row);
  }

  /**
   * @param {ModelQuery} linked
   * @param {unknown} row
   */
  #found(linked, row) {
    const first = partsOf(/** @type {Relation} */ (this.#relations[0]));
    return keyed(linked, first.from, `r${this.#relations.length - 1}`, startKey(first.from, first.fromFields), row);
  }
}

/**
 * The relation options `keys` gives: `from` and `to`; the relation's
 * `onCondition` and `conditionType`; and the actions and name of the foreign
 * key a `belongsTo` relation holds, which only it takes.
 *
 * @param {"belongsTo" | "hasOne" | "hasMany"} kind
 * @param {unknown} keys
 */
function optionsOf(kind, keys) {
  if (typeof keys !== "object" || keys === null) throw new TypeError("a relation's keys are { from, to }");
  for (const option of Object.keys(keys)) {
    if (!KEY_OPTIONS.has(option)) throw new TypeError(`${JSON.stringify(option)} is no relation option`);
  }
  const { from, to, onCondition = null, conditionType = "all", onDelete, onUpdate, fkName = null } =
    /** @type {{ from?: unknown, to?: unknown, onCondition?: unknown, conditionType?: unknown, onDelete?: unknown, onUpdate?: unknown, fkName?: unknown }} */ (keys);
  if (onCondition !== null && typeof onCondition !== "function") {
    throw new TypeError("a relation's onCondition is a function of the two ends, giving a condition");
  }
  if (conditionType !== "all" && conditionType !== "any") throw new TypeError('a relation\'s conditionType is "all" or "any"');
  if (fkName !== null) {
    if (typeof fkName !== "string") throw new TypeError("a relation's fkName is a string");
    if (!isIdentifier(fkName)) throw new ConstructionError("an identifier is 1–63 UTF-8 bytes without NUL");
  }
  if (kind !== "belongsTo" && (onDelete !== undefined || onUpdate !== undefined || fkName !== null)) {
    throw new TypeError("a foreign key is the belongsTo side's: only it takes onDelete, onUpdate and fkName");
  }
  return {
    from,
    to,
    onCondition: /** @type {OnCondition | null} */ (onCondition),
    conditionType: /** @type {"all" | "any"} */ (conditionType),
    onDelete: actionOf(onDelete, "onDelete"),
    onUpdate: actionOf(onUpdate, "onUpdate"),
    fkName: /** @type {string | null} */ (fkName),
  };
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
  const options = optionsOf(kind, keys);
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
    onDelete: options.onDelete,
    onUpdate: options.onUpdate,
    fkName: options.fkName,
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

// [spec:pgorm:req:bindings.schema-from-model]
/**
 * The foreign key a `belongsTo` relation holds, as the native table shape
 * takes it: the table it reaches, the columns it pairs at each end, its
 * actions and its name.
 *
 * @param {Relation} relation
 */
export function foreignKeyOf(relation) {
  const { to, fromFields, toFields } = partsOf(relation);
  return {
    schema: to.schema,
    table: to.name,
    from: fromFields.map((field) => field.sql),
    to: toFields.map((field) => field.sql),
    onDelete: relation.onDelete,
    onUpdate: relation.onUpdate,
    fkName: relation.fkName,
  };
}
