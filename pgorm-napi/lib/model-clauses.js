// @ts-check

// What a model query and a projection of one share: the recipe a statement
// is built from — fields or items, joins, conditions, grouping, DISTINCT,
// ordering, window and lock — the clauses that change it, and the cursor and
// pages over it.
// [spec:pgorm:req:bindings.model-reads]
// [spec:pgorm:req:bindings.model-clauses]

import { arg, TRUSTED } from "./builder.js";
import { Condition } from "./conditions.js";
import { ConstructionError } from "./errors.js";
import { Expr, OrderBy } from "./expressions.js";
import { cursorOver, paginatorOver } from "./cursors.js";
import { fieldOf, projected, qualified, sameTable } from "./model-shape.js";
import { native } from "./operations.js";
import { relationOn, shapesOf } from "./relation-parts.js";
import { Select, select } from "./select.js";

const JOINS = ["inner", "left", "right", "full"];

/** The lock strengths and waits a model query takes, as the statement builder's `lock` does. */
const STRENGTHS = ["update", "noKeyUpdate", "share", "keyShare"];
const WAITS = ["nowait", "skipLocked"];

/**
 * One item of a projection: the key its value is read under, the result
 * column it is projected as, the expression projected, and the declaration
 * its value decodes by.
 *
 * @typedef {object} Item
 * @property {import("./model-shape.js").Field} field `field.field` is the key, `field.sql` the result column.
 * @property {Expr | import("./expressions.js").Aliased} projected
 */

/**
 * What a model query reads and how: the fields it selects, or a projection's
 * items; the tables it joins, its conditions, grouping, DISTINCT, ordering,
 * window and lock.
 *
 * @typedef {object} Recipe
 * @property {import("./model-shape.js").Shape} shape
 * @property {readonly import("./model-shape.js").Field[]} fields
 * @property {readonly { table: import("./select.js").Table, on: Expr | Condition, kind: string, shape: import("./model-shape.js").Shape, qualifier: string }[]} joins
 * @property {readonly (Expr | Condition)[]} wheres
 * @property {readonly OrderBy[]} orders
 * @property {number | bigint | null} limit
 * @property {number | bigint | null} offset
 * @property {boolean | readonly Expr[]} distinct `true` is DISTINCT, a list of columns DISTINCT ON; the later call wins.
 * @property {readonly unknown[]} groups
 * @property {readonly (Expr | Condition)[]} havings
 * @property {{ strength: string, wait: string | undefined, of: readonly unknown[] } | null} lock
 * @property {readonly Item[] | null} items
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
  let projection;
  if (recipe.items === null) {
    projection = recipe.fields.map((field) => projected(field, qualified(field, shape.qualifier), field.sql));
  } else if (recipe.items.length === 0) {
    throw new ConstructionError("a projection selects at least one item");
  } else {
    projection = recipe.items.map((item) => item.projected);
  }
  let query = select(...projection).from(shape.table);
  for (const join of recipe.joins) query = query.join(join.table, join.on, { kind: /** @type {any} */ (join.kind) });
  for (const where of wheres) query = query.where(where);
  if (recipe.groups.length > 0) query = query.groupBy(.../** @type {any[]} */ (recipe.groups));
  for (const having of recipe.havings) query = query.having(having);
  if (recipe.distinct === true) query = query.distinct();
  else if (recipe.distinct !== false) query = query.distinctOn(...recipe.distinct);
  if (orders.length > 0) query = query.orderBy(...orders);
  if (limit !== null) query = query.limit(limit);
  if (recipe.offset !== null) query = query.offset(recipe.offset);
  if (recipe.lock !== null) {
    const { strength, wait, of } = recipe.lock;
    query = query.lock(/** @type {any} */ (strength), { of: /** @type {any[]} */ (of), wait: /** @type {any} */ (wait) });
  }
  return query;
}

/**
 * The source a recipe's rows decode as: its fields, or its projection's
 * items, each under the result column it is projected as.
 *
 * @param {Recipe} recipe
 * @returns {import("./model-decode.js").Source[]}
 */
export function sourcesOf(recipe) {
  const fields = recipe.items === null ? recipe.fields : recipe.items.map((item) => item.field);
  return [{ fields, names: fields.map((field) => field.sql), optional: false }];
}

/**
 * Each model a recipe reads, with the name its table answers to.
 *
 * @param {Recipe} recipe
 * @returns {{ shape: import("./model-shape.js").Shape, qualifier: string }[]}
 */
export function tablesOf(recipe) {
  return [{ shape: recipe.shape, qualifier: recipe.shape.qualifier }, ...recipe.joins];
}

/** @type {(query: Clauses) => Recipe} */
export let recipeOf;

/**
 * The clauses a model query and a projection of one share: each returns a
 * new query over the changed recipe.
 */
// [spec:pgorm:req:bindings.model-clauses]
export class Clauses {
  /** @type {Recipe} */
  #recipe;
  /** @type {(recipe: Recipe) => any} */
  #make;

  /**
   * @param {Recipe} recipe
   * @param {(recipe: Recipe) => any} make
   */
  constructor(recipe, make) {
    this.#recipe = Object.freeze(recipe);
    this.#make = make;
  }

  static {
    recipeOf = (query) => query.#recipe;
  }

  /**
   * @param {Partial<Recipe>} change
   * @returns {this}
   */
  #with(change) {
    return this.#make({ ...this.#recipe, ...change });
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

  /** `SELECT DISTINCT`, replacing a `distinctOn`. */
  distinct() {
    return this.#with({ distinct: true });
  }

  /**
   * `SELECT DISTINCT ON (..)` the columns, of the queried model or a table
   * it joined, replacing `distinct`.
   *
   * @param {...unknown} columns
   */
  distinctOn(...columns) {
    if (columns.length === 0) throw new ConstructionError("distinctOn names at least one column");
    for (const column of columns) {
      if (!(column instanceof Expr)) throw new TypeError("distinctOn takes columns");
    }
    return this.#with({ distinct: /** @type {Expr[]} */ (columns) });
  }

  /**
   * Items added to the GROUP BY list, as the statement builder's `groupBy`
   * takes them.
   *
   * @param {...unknown} items
   */
  groupBy(...items) {
    return this.#with({ groups: [...this.#recipe.groups, ...items] });
  }

  /** @param {unknown} predicate */
  having(predicate) {
    return this.#with({ havings: [...this.#recipe.havings, predicateOf(predicate)] });
  }

  /**
   * Lock the rows read, replacing any lock set before, as the statement
   * builder's `lock` does: Rust's `lock`, `lock_shared`, `lock_exclusive`
   * and `lock_with_behavior`.
   *
   * @param {unknown} strength
   * @param {{ of?: readonly unknown[], wait?: unknown }} [options]
   */
  lock(strength, { of = [], wait } = {}) {
    if (typeof strength !== "string" || !STRENGTHS.includes(strength)) {
      throw new TypeError(`a lock's strength is ${STRENGTHS.map((name) => JSON.stringify(name)).join(", ")}`);
    }
    if (wait !== undefined && (typeof wait !== "string" || !WAITS.includes(wait))) {
      throw new TypeError('a lock\'s wait is "nowait" or "skipLocked"');
    }
    if (!Array.isArray(of)) throw new TypeError("a lock's of is a list of Table or FromItem");
    // Refused as the statement builder refuses it, as the query is made.
    select().lock(/** @type {any} */ (strength), { of: /** @type {any[]} */ (of), wait: /** @type {any} */ (wait) });
    return this.#with({ lock: { strength, wait: /** @type {string | undefined} */ (wait), of: [...of] } });
  }

  /**
   * Join the table at the other end of `relation`, from the query's model or
   * a table it joined, without reading its columns: what a condition on a
   * related row needs.
   *
   * @param {import("./relations.js").Relation} relation
   * @param {{ kind?: "inner" | "left" | "right" | "full", alias?: string }} [options]
   */
  join(relation, { kind = "inner", alias } = {}) {
    const [from, to] = shapesOf(relation);
    const start = this.#start(from, "starts", kind);
    const qualifier = alias ?? to.qualifier;
    const table = alias === undefined ? to.table : to.table.as(alias);
    const on = relationOn(relation, start, qualifier);
    return this.#with({ joins: [...this.#recipe.joins, { table, on, kind, shape: to, qualifier }] });
  }

  /**
   * Join the table `relation` starts at, from its far end, which the query's
   * model or a table it joined is: Rust's `join_rev`, the relation's
   * condition as it stands.
   *
   * @param {import("./relations.js").Relation} relation
   * @param {{ kind?: "inner" | "left" | "right" | "full", alias?: string }} [options]
   */
  joinRev(relation, { kind = "inner", alias } = {}) {
    const [from, to] = shapesOf(relation);
    const start = this.#start(to, "ends", kind);
    const qualifier = alias ?? from.qualifier;
    const table = alias === undefined ? from.table : from.table.as(alias);
    const on = relationOn(relation, qualifier, start);
    return this.#with({ joins: [...this.#recipe.joins, { table, on, kind, shape: from, qualifier }] });
  }

  /**
   * The name of the table a join starts from: `shape`'s, which the relation
   * `end` at.
   *
   * @param {import("./model-shape.js").Shape} shape
   * @param {string} end
   * @param {unknown} kind
   */
  #start(shape, end, kind) {
    if (typeof kind !== "string" || !JOINS.includes(kind)) {
      throw new TypeError('a model join is "inner", "left", "right" or "full"');
    }
    const found = tablesOf(this.#recipe).find((table) => sameTable(table.shape, shape));
    if (found === undefined) {
      throw new ConstructionError(`the relation ${end} at ${shape.name}, which the query does not read`);
    }
    return found.qualifier;
  }
}

/**
 * A keyset cursor over `recipe`'s rows ordered by `fields` of its model, each
 * row decoded as `sources` and then made what `read` makes it.
 *
 * @param {Recipe} recipe
 * @param {readonly string[]} fields
 * @param {readonly import("./model-decode.js").Source[]} sources
 * @param {((record: any) => unknown) | null} read
 */
export function keyedCursor(recipe, fields, sources, read) {
  if (fields.length === 0) throw new ConstructionError("a cursor orders by at least one field");
  return cursorOver({
    build: (orders, wheres, limit) => selectOf(recipe, orders, [...recipe.wheres, ...wheres], limit),
    limit: recipe.limit,
    sources,
    read,
    order: fields.map((name) => ({ field: fieldOf(recipe.shape, name), qualifier: recipe.shape.qualifier })),
    tiebreaks: [],
    tables: tablesOf(recipe),
  });
}

/**
 * Pages of `pageSize` of `recipe`'s rows, decoded as `sources` and made what
 * `read` makes each.
 *
 * @param {Recipe} recipe
 * @param {readonly import("./model-decode.js").Source[]} sources
 * @param {((record: any) => unknown) | null} read
 * @param {number} pageSize
 */
export function pagesOf(recipe, sources, read, pageSize) {
  return paginatorOver({
    page: (limit, offset) => selectOf({ ...recipe, offset }, recipe.orders, recipe.wheres, limit),
    counted: countOf(selectOf(recipe)),
    sources,
    read,
  }, pageSize);
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
