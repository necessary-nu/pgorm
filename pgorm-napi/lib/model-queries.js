// @ts-check

// A model's reads: a SELECT of its fields, filtered, ordered, limited,
// grouped, locked and joined along its relations, run through terminals that
// decode each row into a record of the model's fields, or chosen into a
// projection.
// [spec:pgorm:req:bindings.model-reads]

import { Clauses, countOf, keyedCursor, pagesOf, recipeOf, selectOf, sourcesOf } from "./model-clauses.js";
import { decodeAll, decodeCount, decodeOne, decodeOptional, decodeStream } from "./model-decode.js";
import { partialSelector, projectionOf } from "./model-projections.js";
import { projected, qualified } from "./model-shape.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi model query");

/** @typedef {import("./model-clauses.js").Recipe} Recipe */
/** @typedef {import("./select.js").Select} Select */

/**
 * A model's SELECT: every method returns a new query, and the terminals
 * decode each row into a record of the fields it selects.
 */
export class ModelQuery extends Clauses {
  /** @type {Select} */
  #statement;

  /**
   * @param {Recipe} recipe
   * @param {symbol} token
   */
  constructor(recipe, token) {
    if (token !== MADE) throw new TypeError("a ModelQuery comes from a model's find() or select()");
    super(recipe, (changed) => new ModelQuery(changed, MADE));
    this.#statement = selectOf(recipe);
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

  /** A projection of no items yet, the query's clauses kept: Rust's `select_only`. */
  selectOnly() {
    return projectionOf({ ...recipeOf(this), items: [] });
  }

  /** A projection of the fields the query selects, each projected as the query projects it. */
  #projection() {
    const recipe = recipeOf(this);
    const items = recipe.fields.map((field) => ({
      field,
      projected: projected(field, qualified(field, recipe.shape.qualifier), field.sql),
    }));
    return projectionOf({ ...recipe, items });
  }

  /** @param {unknown} column */
  column(column) {
    return this.#projection().column(column);
  }

  /** @param {...unknown} columns */
  columns(...columns) {
    return this.#projection().columns(...columns);
  }

  /**
   * @param {unknown} column
   * @param {unknown} name
   */
  columnAs(column, name) {
    return this.#projection().columnAs(column, name);
  }

  /**
   * @param {unknown} expr
   * @param {unknown} name
   * @param {unknown} declaration
   */
  exprAs(expr, name, declaration) {
    return this.#projection().exprAs(expr, name, declaration);
  }

  /** The rows as arrays of the fields' values, in the order selected. */
  intoTuple() {
    return this.#projection().intoTuple();
  }

  /** @param {...unknown} names */
  intoValues(...names) {
    return this.#projection().intoValues(...names);
  }

  /** @param {unknown} [make] */
  intoModel(make) {
    return this.#projection().intoModel(make);
  }

  /**
   * The rows read as `shape` declares them, the query's clauses kept: Rust's
   * `into_partial_model`.
   *
   * @param {unknown} shape
   */
  intoPartialModel(shape) {
    return partialSelector(shape, recipeOf(this));
  }

  /**
   * Every row.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await decodeAll(db, this.#statement, sourcesOf(recipeOf(this)), options);
  }

  /**
   * Exactly one row; any other count is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return await decodeOne(db, this.#statement, sourcesOf(recipeOf(this)), options);
  }

  /**
   * At most one row, or null; more is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return await decodeOptional(db, this.#statement, sourcesOf(recipeOf(this)), options);
  }

  /**
   * The rows, one per pull, over a pool's or a connection's connection.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    return decodeStream(db, this.#statement, sourcesOf(recipeOf(this)), options);
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
    return keyedCursor(recipeOf(this), fields, sourcesOf(recipeOf(this)), null);
  }

  /**
   * Pages of `pageSize` rows.
   *
   * @param {number} pageSize
   */
  paginate(pageSize) {
    return pagesOf(recipeOf(this), sourcesOf(recipeOf(this)), null, pageSize);
  }
}

/**
 * A query reading `fields` of `shape`.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {readonly import("./model-shape.js").Field[]} fields
 */
export function query(shape, fields) {
  return new ModelQuery({
    shape,
    fields,
    joins: [],
    wheres: [],
    orders: [],
    limit: null,
    offset: null,
    distinct: false,
    groups: [],
    havings: [],
    lock: null,
    items: null,
  }, MADE);
}
