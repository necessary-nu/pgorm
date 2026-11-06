// @ts-check

// A model's reads: a SELECT of its fields, filtered, ordered, limited and
// joined along its relations, run through terminals that decode each row
// into a record of the model's fields.
// [spec:pgorm:req:bindings.model-reads]

import { arg, TRUSTED } from "./builder.js";
import { Condition } from "./conditions.js";
import { ConstructionError } from "./errors.js";
import { Expr, OrderBy } from "./expressions.js";
import { cursorOver, paginatorOver } from "./cursors.js";
import { decodeAll, decodeCount, decodeOne, decodeOptional, decodeStream } from "./model-decode.js";
import { fieldOf, projected, qualified, sameTable } from "./model-shape.js";
import { native } from "./operations.js";
import { relationOn, shapesOf, startOf } from "./relations.js";
import { Select, select } from "./select.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi model query");

/** @type {(query: ModelQuery) => Recipe} */
let recipeOfQuery;

/**
 * What a model query reads and how: the fields it selects, the tables it
 * joins, its conditions, ordering and window.
 *
 * @typedef {object} Recipe
 * @property {import("./model-shape.js").Shape} shape
 * @property {readonly import("./model-shape.js").Field[]} fields
 * @property {readonly { table: import("./select.js").Table, on: Expr | Condition, kind: "inner" | "left", shape: import("./model-shape.js").Shape, qualifier: string }[]} joins
 * @property {readonly (Expr | Condition)[]} wheres
 * @property {readonly OrderBy[]} orders
 * @property {number | bigint | null} limit
 * @property {number | bigint | null} offset
 */

/**
 * @param {unknown} predicate
 * @returns {Expr | Condition}
 */
export function predicateOf(predicate) {
  if (!(predicate instanceof Expr) && !(predicate instanceof Condition)) {
    throw new TypeError("a condition is an expression or a Condition");
  }
  return predicate;
}

/**
 * @param {readonly unknown[]} orderings
 * @returns {OrderBy[]}
 */
export function orderingsOf(orderings) {
  return orderings.map((ordering) => {
    if (!(ordering instanceof OrderBy)) throw new TypeError("an ordering is expr.asc() or expr.desc()");
    return ordering;
  });
}

/**
 * The SELECT a recipe reads, its ordering and window those given.
 *
 * @param {Recipe} recipe
 * @param {readonly OrderBy[]} orders
 * @param {readonly (Expr | Condition)[]} wheres
 * @param {number | bigint | null} limit
 * @returns {Select}
 */
export function selectOf(recipe, orders = recipe.orders, wheres = recipe.wheres, limit = recipe.limit) {
  const { shape } = recipe;
  const projection = recipe.fields.map((field) => projected(field, qualified(field, shape.qualifier), field.sql));
  let query = select(...projection).from(shape.table);
  for (const join of recipe.joins) query = query.join(join.table, join.on, { kind: join.kind });
  for (const where of wheres) query = query.where(where);
  if (orders.length > 0) query = query.orderBy(...orders);
  if (limit !== null) query = query.limit(limit);
  if (recipe.offset !== null) query = query.offset(recipe.offset);
  return query;
}

/**
 * The source a recipe's rows decode as: its fields, under their own names.
 *
 * @param {Recipe} recipe
 * @returns {import("./model-decode.js").Source[]}
 */
export function sourcesOf(recipe) {
  return [{ fields: recipe.fields, names: recipe.fields.map((field) => field.sql), optional: false }];
}

/**
 * A model's SELECT: every method returns a new query, and the terminals
 * decode each row into a record of the fields it selects.
 */
export class ModelQuery {
  /** @type {Recipe} */
  #recipe;
  /** @type {Select} */
  #statement;

  /**
   * @param {Recipe} recipe
   * @param {symbol} token
   */
  constructor(recipe, token) {
    if (token !== MADE) throw new TypeError("a ModelQuery comes from a model's find() or select()");
    this.#recipe = Object.freeze(recipe);
    this.#statement = selectOf(recipe);
  }

  static {
    recipeOfQuery = (query) => query.#recipe;
  }

  /** @param {Partial<Recipe>} change */
  #with(change) {
    return new ModelQuery({ ...this.#recipe, ...change }, MADE);
  }

  /** The SELECT it runs. */
  get statement() {
    return this.#statement;
  }

  /**
   * The SQL and values it runs, or with `"count"` the ones `count` runs.
   *
   * @param {"count"} [terminal]
   */
  inspect(terminal) {
    if (terminal === undefined) return this.#statement.inspect();
    if (terminal === "count") return countOf(this.#statement).inspect();
    throw new TypeError('a model query inspects itself, or its "count"');
  }

  /**
   * Rows that also satisfy `predicate`.
   *
   * @param {unknown} predicate
   */
  where(predicate) {
    return this.#with({ wheres: [...this.#recipe.wheres, predicateOf(predicate)] });
  }

  /** @param {...unknown} orderings */
  orderBy(...orderings) {
    return this.#with({ orders: [...this.#recipe.orders, ...orderingsOf(orderings)] });
  }

  /** @param {number | bigint | null} count */
  limit(count) {
    return this.#with({ limit: count });
  }

  /** @param {number | bigint | null} count */
  offset(count) {
    return this.#with({ offset: count });
  }

  /**
   * Join the table at the other end of `relation`, from the query's model or
   * a table it joined, without reading its columns: what a condition on a
   * related row needs. The join starts from the first table of the
   * relation's model the query reads, or the one its `fromAlias` names.
   *
   * @param {import("./relations.js").Relation} relation
   * @param {{ kind?: "inner" | "left", alias?: string }} [options]
   */
  join(relation, { kind = "inner", alias } = {}) {
    if (kind !== "inner" && kind !== "left") throw new TypeError('a model join is "inner" or "left"');
    const [from, to] = shapesOf(relation);
    const recipe = this.#recipe;
    const tables = [{ shape: recipe.shape, qualifier: recipe.shape.qualifier }, ...recipe.joins];
    const start = startOf(relation);
    let leftQualifier;
    if (start === null) {
      const found = tables.find((table) => sameTable(table.shape, from));
      if (found === undefined) throw new ConstructionError(`the relation starts at ${from.name}, which the query does not read`);
      leftQualifier = found.qualifier;
    } else {
      const named = tables.find((table) => table.qualifier === start);
      if (named === undefined) throw new ConstructionError(`the query reads no table called ${JSON.stringify(start)}`);
      if (!sameTable(named.shape, from)) {
        throw new ConstructionError(`${JSON.stringify(start)} is ${named.shape.name}, where the relation starts at ${from.name}`);
      }
      leftQualifier = start;
    }
    const qualifier = alias ?? to.qualifier;
    const table = alias === undefined ? to.table : to.table.as(alias);
    const on = relationOn(relation, leftQualifier, qualifier);
    return this.#with({ joins: [...recipe.joins, { table, on, kind, shape: to, qualifier }] });
  }

  /**
   * Every row.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await decodeAll(db, this.#statement, sourcesOf(this.#recipe), options);
  }

  /**
   * Exactly one row; any other count is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return await decodeOne(db, this.#statement, sourcesOf(this.#recipe), options);
  }

  /**
   * At most one row, or null; more is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return await decodeOptional(db, this.#statement, sourcesOf(this.#recipe), options);
  }

  /**
   * The rows, one per pull, over a pool's or a connection's connection.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    return decodeStream(db, this.#statement, sourcesOf(this.#recipe), options);
  }

  /**
   * How many rows the query reads, its limit and offset aside, counted as
   * pgorm's paginator counts.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    return await decodeCount(db, countOf(this.#statement), options);
  }

  /**
   * A keyset cursor over the rows ordered by `fields` of the model.
   *
   * @param {...string} fields
   */
  cursor(...fields) {
    const recipe = this.#recipe;
    if (fields.length === 0) throw new ConstructionError("a cursor orders by at least one field");
    return cursorOver({
      build: (orders, wheres, limit) => selectOf(recipe, orders, [...recipe.wheres, ...wheres], limit),
      limit: recipe.limit,
      sources: sourcesOf(recipe),
      order: fields.map((name) => ({ field: fieldOf(recipe.shape, name), qualifier: recipe.shape.qualifier })),
      tiebreaks: [],
    });
  }

  /**
   * Pages of `pageSize` rows.
   *
   * @param {number} pageSize
   */
  paginate(pageSize) {
    const recipe = this.#recipe;
    return paginatorOver({
      page: (limit, offset) => selectOf({ ...recipe, offset }, recipe.orders, recipe.wheres, limit),
      counted: countOf(this.#statement),
      sources: sourcesOf(recipe),
    }, pageSize);
  }
}

/**
 * The statement counting the rows `statement` reads.
 *
 * @param {Select} statement
 * @returns {Select}
 */
export function countOf(statement) {
  return new Select(native.modelCount(arg(statement)), TRUSTED);
}

/**
 * What `query` reads, which a relation's loader re-roots.
 *
 * @param {ModelQuery} query
 */
export function recipeOf(query) {
  return recipeOfQuery(query);
}

/**
 * The query reading `recipe`.
 *
 * @param {Recipe} recipe
 */
export function queryOf(recipe) {
  return new ModelQuery(recipe, MADE);
}

/**
 * A query reading `fields` of `shape`.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {readonly import("./model-shape.js").Field[]} fields
 */
export function query(shape, fields) {
  return new ModelQuery({ shape, fields, joins: [], wheres: [], orders: [], limit: null, offset: null }, MADE);
}
