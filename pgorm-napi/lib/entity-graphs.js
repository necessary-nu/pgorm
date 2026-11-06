// @ts-check

// The graph shapes the application's own module registers: a
// `SelectGraph<E, S>` whose root and slots are fixed in Rust, found by name,
// queried under the aliases JavaScript chooses, decoded by Rust's
// `GraphRow` — a slotless row as the root's record, otherwise a tuple of
// each source's record, an absent `Opt` slot null — and paged by keyset
// through `SelectGraph::cursor_by`.
// [spec:pgorm:req:bindings.entity-graphs+4]

import { arg, TRUSTED } from "./builder.js";
import { decodedStream, runJob } from "./connections.js";
import { predicateOf, rowsOf } from "./entities.js";
import { boundWhole, EntityCursor, EntityPaginator, READS } from "./entity-reads.js";
import { Expr, OrderBy } from "./expressions.js";
import { native } from "./operations.js";
import { decodedWith } from "./streams.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi entity graph");

/** The names of the graph shapes this module registers. */
export function graphs() {
  return /** @type {string[]} */ (native.graphNames(native.registry));
}

/**
 * The graph shape this module registers as `name`.
 *
 * @param {string} name
 */
export function graph(name) {
  if (typeof name !== "string") throw new TypeError("a graph's name is a string");
  return new EntityGraph(native.graphGet(native.registry, name), name, MADE);
}

/** A registered graph shape. */
export class EntityGraph {
  /** @type {unknown} */
  #native;
  /** @type {string} */
  #name;
  /** @type {readonly string[]} */
  #entities;

  /**
   * @param {unknown} handle
   * @param {string} name
   * @param {symbol} token
   */
  constructor(handle, name, token) {
    if (token !== MADE) throw new TypeError("an EntityGraph comes from graph(name)");
    this.#native = handle;
    this.#name = name;
    this.#entities = Object.freeze(this.describe().sources.map((/** @type {{ entity: string }} */ source) => source.entity));
    Object.freeze(this);
  }

  get name() {
    return this.#name;
  }

  /** The shape as data: its Rust type, and each source's entity and slot kind. */
  describe() {
    return JSON.parse(native.graphDescribe(this.#native));
  }

  /**
   * The graph built by its application factory, each joined slot under the
   * alias given for it — `g1`, `g2`, .. when none are.
   *
   * @param {{ aliases?: readonly string[] }} [options]
   */
  find({ aliases } = {}) {
    if (aliases !== undefined && !Array.isArray(aliases)) throw new TypeError("aliases is an array of names");
    return new EntityGraphQuery(native.graphFind(this.#native, aliases ?? null), this.#entities, MADE);
  }
}

/** A query over a registered graph. Every method returns a new query. */
export class EntityGraphQuery {
  /** @type {unknown} */
  #native;
  /** @type {readonly string[]} */
  #entities;

  /**
   * @param {unknown} handle
   * @param {readonly string[]} entities
   * @param {symbol} token
   */
  constructor(handle, entities, token) {
    if (token !== MADE) throw new TypeError("an EntityGraphQuery comes from a graph's find()");
    this.#native = handle;
    this.#entities = entities;
  }

  /** The aliases its slots are joined under, in slot order. */
  get aliases() {
    return /** @type {string[]} */ (native.graphQueryAliases(this.#native));
  }

  /**
   * @param {string} step
   * @param {unknown} argument
   */
  #step(step, argument) {
    return new EntityGraphQuery(native.graphQueryChange(this.#native, step, argument), this.#entities, MADE);
  }

  /** @param {unknown} predicate */
  where(predicate) {
    return this.#step("where", arg(predicateOf(predicate)));
  }

  /** @param {...unknown} orderings */
  orderBy(...orderings) {
    /** @type {EntityGraphQuery} */
    let query = this;
    for (const ordering of orderings) {
      if (!(ordering instanceof OrderBy)) throw new TypeError("an ordering is expr.asc() or expr.desc()");
      query = query.#step("orderBy", arg(ordering));
    }
    return query;
  }

  /**
   * A column of decoded source `source` — 0 the root, i the i-th slot —
   * qualified as the query names it.
   *
   * @param {number} source
   * @param {string} column
   */
  col(source, column) {
    return new Expr(native.graphQueryCol(this.#native, source, column), TRUSTED);
  }

  /**
   * The SQL and values a terminal sends, `optional` with its `LIMIT 1` and
   * `count` the paginator's count.
   *
   * @param {"all" | "optional" | "count"} [terminal]
   */
  inspect(terminal = "all") {
    if (terminal === "count") return this.paginate(1).inspect("count");
    if (terminal !== "all" && terminal !== "optional") throw new TypeError('a graph inspects "all", "optional" or "count"');
    const [sql, values] = native.graphQueryInspect(this.#native, terminal === "optional");
    return Object.freeze({ sql, values: Object.freeze(values) });
  }

  /**
   * Every row, by `SelectGraph::all`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return rowsOf(this.#entities, await runJob(db, native.graphQueryJob(this.#native, false), options));
  }

  /**
   * The first row or null, by `SelectGraph::one_opt`: no row is apart from
   * a row whose optional slot is absent.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return rowsOf(this.#entities, await runJob(db, native.graphQueryJob(this.#native, true), options))[0] ?? null;
  }

  /**
   * A keyset cursor ordered by root columns, by `SelectGraph::cursor_by`,
   * every source's primary key after them as tiebreaks.
   *
   * @param {...string} columns
   */
  cursor(...columns) {
    return this.cursorOn(0, ...columns);
  }

  /**
   * A keyset cursor ordered by columns of decoded source `source`, the first
   * slot being 1, by `SelectGraph::cursor_by_on`: the source's own key, then
   * the root's and the other slots', as tiebreaks.
   *
   * @param {number} source
   * @param {...string} columns
   */
  cursorOn(source, ...columns) {
    return new EntityGraphCursor(native.graphQueryCursor(this.#native, source, columns), this.#entities, READS);
  }

  /**
   * How many rows the graph reads, by `PaginatorTrait::count`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    return (await this.paginate(1).numItemsAndPages(db, options)).items;
  }

  /**
   * Pages of `pageSize` rows of the join, by `PaginatorTrait::paginate`.
   *
   * @param {number} pageSize
   */
  paginate(pageSize) {
    return new EntityPaginator(native.graphQueryPages(this.#native, pageSize), this.#entities, READS);
  }

  /**
   * Each root with its one `Opt` slot's records, by `all_grouped`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async allGrouped(db, options) {
    const job = native.graphQueryGroupedJob(this.#native);
    const [roots, slots, groups] = /** @type {[string[], string[], [[unknown[], unknown], [unknown[], unknown][]][]]} */ (
      await runJob(db, job, options)
    );
    const [root, slot] = /** @type {[string, string]} */ (this.#entities);
    return groups.map(([parent, children]) => [
      rowsOf([root], [[roots], [[parent]]])[0],
      children.map((child) => rowsOf([slot], [[slots], [[child]]])[0]),
    ]);
  }

  /**
   * The rows one per pull, decoded as `SelectGraph::stream` decodes each as
   * it arrives, over a connection the stream holds until it ends.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    const [statement, decoder] = native.graphQueryStream(this.#native);
    const entities = this.#entities;
    const stream = decodedStream(db, statement, decoder, options, "a registered graph");
    return decodedWith(stream, (_names, pulled) => rowsOf(entities, /** @type {[string[][], ([unknown[], unknown] | null)[][]]} */ (pulled))[0]);
  }
}

/**
 * A keyset cursor over a registered graph, by `SelectGraph::cursor_by` or
 * `cursor_by_on`. Every method returns a new cursor.
 */
export class EntityGraphCursor extends EntityCursor {
  /**
   * Rows short of the one whose order-column values, or whole key, these
   * are: the order columns, the ordered source's other key columns, then
   * each other source's.
   *
   * @param {...unknown} values
   */
  beforeWith(...values) {
    return boundWhole(this, true, values);
  }

  /** @param {...unknown} values */
  afterWith(...values) {
    return boundWhole(this, false, values);
  }
}
