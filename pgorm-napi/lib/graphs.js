// @ts-check

// A model's graph: its rows with the rows its relations reach, each joined
// source decoded as a record of its own, as pgorm's `SelectGraph` reads
// them. The slot kind is the join type is the decode shape: `joinOne` is an
// INNER JOIN decoded as a record, `joinMaybe` a LEFT JOIN decoded as a
// record or null, and `via` a LEFT JOIN through a table no record reads.
// Every source is projected under its own prefix, `s0_` for the root and
// `s{i}_` for the i-th slot, by the composition pgorm's graph writer uses.
// [spec:pgorm:req:bindings.graphs+2]

import { cursorOver, paginatorOver } from "./cursors.js";
import { Condition } from "./conditions.js";
import { ConstructionError } from "./errors.js";
import { decodeAll, decodeCount, decodeOne, decodeOptional, decodeStream } from "./model-decode.js";
import { countOf, orderingsOf, predicateOf } from "./model-clauses.js";
import { fieldOf, projected, qualified, sameTable } from "./model-shape.js";
import { native } from "./operations.js";
import { answering, conditionOf, keyText, Related, relationOn, shapesOf, startOf } from "./relations.js";
import { select } from "./select.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi graph");

/**
 * A table the graph reads: the root, a decoded slot, or an undecoded hop.
 *
 * @typedef {object} Joined
 * @property {import("./model-shape.js").Shape} shape
 * @property {string} qualifier What its columns are qualified by.
 * @property {"root" | "slot" | "hop"} role
 * @property {boolean} optional Whether its record may be absent.
 */

/**
 * @typedef {object} State
 * @property {readonly Joined[]} tables The root, then each slot and hop in join order.
 * @property {readonly { table: import("./select.js").Table, on: import("./conditions.js").Condition, kind: "inner" | "left" }[]} joins
 * @property {readonly (import("./expressions.js").Expr | import("./conditions.js").Condition)[]} wheres
 * @property {readonly import("./expressions.js").OrderBy[]} orders
 * @property {number | bigint | null} limit
 * @property {number | bigint | null} offset
 * @property {boolean} sliced Set by `limit` or `offset`, whatever their value:
 *   the caller has taken charge of the row range.
 */

/** Why each read that decides the row range itself refuses a sliced graph. */
const REFUSALS = {
  allGrouped: "a window of rows can end part-way through a root's rows, and its group would look complete",
  count: "the count would ignore the window",
  cursor: "its first and last would replace the window, and an offset would skip rows past each boundary",
  cursorOn: "its first and last would replace the window, and an offset would skip rows past each boundary",
  paginate: "its pages would replace the window",
};

/**
 * Refuse a read that decides the row range itself on a graph whose limit or
 * offset already decided it.
 *
 * @param {State} state
 * @param {keyof typeof REFUSALS} terminal
 */
function unsliced(state, terminal) {
  if (state.sliced) {
    throw new ConstructionError(
      `a graph cut by limit or offset has no ${terminal}: ${REFUSALS[terminal]}; ` +
        "read the first roots with find().limit(..) and load their related rows",
    );
  }
}

/**
 * The decoded sources: the root, then each slot.
 *
 * @param {State} state
 */
function decoded(state) {
  return state.tables.filter((table) => table.role !== "hop");
}

/**
 * @param {State} state
 * @returns {import("./model-decode.js").Source[]}
 */
function sourcesOf(state) {
  return decoded(state).map((table, index) => ({
    fields: table.shape.fields,
    names: table.shape.fields.map((field) => native.modelResultName(`s${index}_`, field.sql)),
    optional: table.optional,
  }));
}

/**
 * The graph's SELECT, its ordering, further conditions and window those
 * given.
 *
 * @param {State} state
 * @param {readonly import("./expressions.js").OrderBy[]} orders
 * @param {readonly (import("./expressions.js").Expr | import("./conditions.js").Condition)[]} wheres
 * @param {number | bigint | null} limit
 * @param {number | bigint | null} offset
 */
function selectOf(state, orders = state.orders, wheres = state.wheres, limit = state.limit, offset = state.offset) {
  const sources = sourcesOf(state);
  const projection = decoded(state).flatMap((table, index) =>
    table.shape.fields.map((field, position) =>
      projected(field, qualified(field, table.qualifier), /** @type {string} */ (sources[index]?.names[position]))
    )
  );
  const root = /** @type {Joined} */ (state.tables[0]);
  let query = select(...projection).from(root.shape.table);
  for (const join of state.joins) query = query.join(join.table, join.on, { kind: join.kind });
  for (const where of wheres) query = query.where(where);
  if (orders.length > 0) query = query.orderBy(...orders);
  if (limit !== null) query = query.limit(limit);
  if (offset !== null) query = query.offset(offset);
  return query;
}

/**
 * A model's rows with the rows its relations reach. Every method returns a
 * new graph; the terminals decode each row as the root's record when no slot
 * is joined, and otherwise as a tuple of the root's and each slot's.
 */
export class Graph {
  /** @type {State} */
  #state;

  /**
   * @param {State} state
   * @param {symbol} token
   */
  constructor(state, token) {
    if (token !== MADE) throw new TypeError("a Graph comes from a model's graph()");
    this.#state = Object.freeze(state);
    selectOf(this.#state);
  }

  /** @param {Partial<State>} change */
  #with(change) {
    return new Graph({ ...this.#state, ...change }, MADE);
  }

  /**
   * Join the far end of `relation` from the table it starts at — the first
   * the graph reads, the table its `fromAlias` names, or the decoded source
   * `from` names — under its own name or `alias`, narrowed by `extra`'s
   * condition and joined as `kind` says where they are given.
   *
   * @param {unknown} relation
   * @param {"root" | "slot" | "hop"} role
   * @param {boolean} optional
   * @param {{ alias?: string, from?: number }} options
   * @param {((left: object, right: object) => unknown) | null} [extra]
   * @param {"inner" | "left" | null} [joinKind]
   */
  #edge(relation, role, optional, { alias, from } = {}, extra = null, joinKind = null) {
    const [start, end] = shapesOf(relation);
    const { tables, joins } = this.#state;
    const named = startOf(/** @type {any} */ (relation));
    let left;
    if (named !== null) {
      if (from !== undefined) throw new ConstructionError("a relation from an alias names where it starts: it takes no from");
      left = tables.find((table) => table.qualifier === named);
      if (left === undefined) throw new ConstructionError(`the graph reads no table called ${JSON.stringify(named)}`);
      if (!sameTable(left.shape, start)) {
        throw new ConstructionError(`${JSON.stringify(named)} is ${left.shape.name}, where the relation starts at ${start.name}`);
      }
    } else if (from === undefined) {
      left = tables.find((table) => sameTable(table.shape, start));
    } else {
      left = decoded(this.#state)[from];
      if (left !== undefined && !sameTable(left.shape, start)) {
        throw new ConstructionError(`source ${from} is ${left.shape.name}, where the relation starts at ${start.name}`);
      }
    }
    if (left === undefined) {
      throw new ConstructionError(`the relation starts at ${start.name}, which the graph does not read`);
    }
    const qualifier = alias ?? end.qualifier;
    if (tables.some((table) => table.qualifier === qualifier)) {
      throw new ConstructionError(`the graph already reads a table called ${JSON.stringify(qualifier)}: give this one an alias`);
    }
    const table = alias === undefined ? end.table : end.table.as(alias);
    /** @type {import("./conditions.js").Condition} */
    let on = relationOn(/** @type {any} */ (relation), left.qualifier, qualifier);
    if (extra !== null) {
      on = Condition.all(on, conditionOf(extra(answering(start, left.qualifier), answering(end, qualifier)), "a join's condition"));
    }
    return this.#with({
      tables: [...tables, { shape: end, qualifier, role, optional }],
      joins: [...joins, { table, on, kind: joinKind ?? (role === "slot" && !optional ? "inner" : "left") }],
    });
  }

  /**
   * A slot that must match: an INNER JOIN of the relation's far end, decoded
   * as its record.
   *
   * @param {unknown} relation
   * @param {{ alias?: string, from?: number }} [options]
   */
  joinOne(relation, options) {
    return this.#edge(relation, "slot", false, options);
  }

  /**
   * A slot that may be absent: a LEFT JOIN of the relation's far end,
   * decoded as its record, or null where the join matched nothing.
   *
   * @param {unknown} relation
   * @param {{ alias?: string, from?: number }} [options]
   */
  joinMaybe(relation, options) {
    return this.#edge(relation, "slot", true, options);
  }

  /**
   * `joinMaybe` with `condition`, given the two ends as the join names them,
   * ANDed into the join's ON, as pgorm's `join_maybe_filtered` adds it: it
   * narrows which rows match, and a root with none keeps its row with the
   * slot null.
   *
   * @param {unknown} relation
   * @param {unknown} condition
   * @param {{ alias?: string, from?: number }} [options]
   */
  joinMaybeFiltered(relation, condition, options) {
    if (typeof condition !== "function") throw new TypeError("a join's condition is a function of the two ends");
    return this.#edge(relation, "slot", true, options, /** @type {(left: object, right: object) => unknown} */ (condition));
  }

  /**
   * A hop never decoded — a junction table, a step of a chain — LEFT JOINed
   * so that a missing middle cannot drop a root row by itself.
   *
   * @param {unknown} relation
   * @param {{ alias?: string, from?: number }} [options]
   */
  via(relation, options) {
    return this.#edge(relation, "hop", true, options);
  }

  // [spec:pgorm:req:bindings.related]
  /**
   * A slot that must match, along `path`: a relation, as `joinOne` joins it,
   * or a related path, its junction INNER JOINed as a hop and its far end as
   * the slot, as pgorm's `related_one` folds a `Related` path in.
   *
   * @param {unknown} path
   */
  relatedOne(path) {
    if (path instanceof Related) return this.#edge(path.via, "hop", true, {}, null, "inner").joinOne(path.relation);
    return this.joinOne(path);
  }

  /**
   * A slot that may be absent, along `path`: a relation, as `joinMaybe`
   * joins it, or a related path, its junction a `via` hop and its far end
   * the slot, as pgorm's `related_maybe` folds a `Related` path in.
   *
   * @param {unknown} path
   */
  relatedMaybe(path) {
    if (path instanceof Related) return this.via(path.via).joinMaybe(path.relation);
    return this.joinMaybe(path);
  }

  /**
   * A column of decoded source `source` — 0 the root, i the i-th slot —
   * qualified as that source is named.
   *
   * @param {number} source
   * @param {string} field
   */
  col(source, field) {
    const table = decoded(this.#state)[source];
    if (table === undefined) throw new ConstructionError(`the graph has no source ${source}`);
    return qualified(fieldOf(table.shape, field), table.qualifier);
  }

  /** @param {unknown} predicate */
  where(predicate) {
    return this.#with({ wheres: [...this.#state.wheres, predicateOf(predicate)] });
  }

  /** @param {...unknown} orderings */
  orderBy(...orderings) {
    return this.#with({ orders: [...this.#state.orders, ...orderingsOf(orderings)] });
  }

  /**
   * Read at most `count` rows of the join, not roots, or with null no limit;
   * either way the graph is sliced.
   *
   * @param {number | bigint | null} count
   */
  limit(count) {
    return this.#with({ limit: count, sliced: true });
  }

  /**
   * Skip the first `count` rows of the join, or with null none; either way
   * the graph is sliced.
   *
   * @param {number | bigint | null} count
   */
  offset(count) {
    return this.#with({ offset: count, sliced: true });
  }

  get statement() {
    return selectOf(this.#state);
  }

  /**
   * The SQL and values it runs, or the ones `count` and `allGrouped` run.
   *
   * @param {"count" | "allGrouped"} [terminal]
   */
  inspect(terminal) {
    if (terminal === undefined) return this.statement.inspect();
    if (terminal === "count") {
      unsliced(this.#state, "count");
      return countOf(this.statement).inspect();
    }
    if (terminal === "allGrouped") return this.#grouped().inspect();
    throw new TypeError('a graph inspects itself, its "count" or its "allGrouped"');
  }

  /**
   * How many rows it reads.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    unsliced(this.#state, "count");
    return await decodeCount(db, countOf(this.statement), options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await decodeAll(db, this.statement, sourcesOf(this.#state), options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return await decodeOne(db, this.statement, sourcesOf(this.#state), options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return await decodeOptional(db, this.statement, sourcesOf(this.#state), options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    return decodeStream(db, this.statement, sourcesOf(this.#state), options);
  }

  /** The SELECT `allGrouped` runs: the root's key ordered behind the graph's ordering. */
  #grouped() {
    unsliced(this.#state, "allGrouped");
    if (decoded(this.#state).length !== 2) throw new ConstructionError("allGrouped reads a graph of exactly one slot");
    const key = this.#rootKey();
    if (key.length === 0) throw new ConstructionError("allGrouped groups by the root's primary key, which it has none of");
    const orders = [...this.#state.orders, ...key.map(({ field, qualifier }) => qualified(field, qualifier).asc())];
    return selectOf(this.#state, orders);
  }

  /** The root's primary key, qualified as the root is named. */
  #rootKey() {
    const root = /** @type {Joined} */ (this.#state.tables[0]);
    return root.shape.primaryKey.map((field) => ({ field, qualifier: root.qualifier }));
  }

  /**
   * Each root with the records its one slot holds for it, as pgorm's
   * `all_grouped` reads them: the root's primary key ordered behind the
   * graph's ordering, and the rows consolidated by the decoded root's key,
   * in the order roots first appear.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async allGrouped(db, options) {
    const grouped = this.#grouped();
    const key = this.#rootKey();
    const rows = await decodeAll(db, grouped, sourcesOf(this.#state), options);
    /** @type {Map<string, [Record<string, unknown>, Record<string, unknown>[]]>} */
    const groups = new Map();
    for (const [root, slot] of rows) {
      const text = keyText(key.map(({ field }) => root[field.field]));
      let group = groups.get(text);
      if (group === undefined) {
        group = [root, []];
        groups.set(text, group);
      }
      if (slot !== null) group[1].push(slot);
    }
    return [...groups.values()];
  }

  /**
   * A keyset cursor ordered by `fields` of the root, with the root's primary
   * key and each slot's as tiebreaks behind them, in declaration order, so a
   * join that repeats a root row still has a total order.
   *
   * @param {...string} fields
   */
  cursor(...fields) {
    unsliced(this.#state, "cursor");
    return this.#cursor(0, fields);
  }

  // [spec:pgorm:req:bindings.cursors+1]
  /**
   * A keyset cursor ordered by `fields` of slot `source`, the first slot
   * `1`, as pgorm's `cursor_by_on` orders: the slot's primary key, then the
   * root's, then every other slot's, in declaration order, are its
   * tiebreaks.
   *
   * @param {number} source
   * @param {...string} fields
   */
  cursorOn(source, ...fields) {
    if (!Number.isInteger(source)) throw new TypeError("a graph's source is its index");
    unsliced(this.#state, "cursorOn");
    if (source === 0) throw new ConstructionError("cursorOn orders by a slot's fields, the first slot 1; cursor orders by the root's");
    return this.#cursor(source, fields);
  }

  /**
   * @param {number} source
   * @param {readonly string[]} fields
   */
  #cursor(source, fields) {
    if (fields.length === 0) throw new ConstructionError("a cursor orders by at least one field");
    const state = this.#state;
    const sources = decoded(state);
    const ordered = sources[source];
    if (ordered === undefined) throw new ConstructionError(`the graph has no source ${source}`);
    const order = fields.map((name) => ({ field: fieldOf(ordered.shape, name), qualifier: ordered.qualifier }));
    const others = sources.filter((_, index) => index !== source);
    const tiebreaks = [
      ...ordered.shape.primaryKey
        .filter((field) => !order.some((column) => column.field === field))
        .map((field) => ({ field, qualifier: ordered.qualifier })),
      ...others.flatMap((table) => table.shape.primaryKey.map((field) => ({ field, qualifier: table.qualifier }))),
    ];
    return cursorOver({
      build: (orders, wheres, limit) => selectOf(state, orders, [...state.wheres, ...wheres], limit),
      limit: null,
      sources: sourcesOf(state),
      read: null,
      order,
      tiebreaks,
      tables: state.tables,
    });
  }

  /**
   * Pages of `pageSize` rows: a page boundary falls between rows, so a root
   * with several slot rows can span pages, as the SQL does.
   *
   * @param {number} pageSize
   */
  paginate(pageSize) {
    unsliced(this.#state, "paginate");
    const state = this.#state;
    return paginatorOver({
      page: (limit, offset) => selectOf(state, state.orders, state.wheres, limit, offset),
      counted: countOf(selectOf(state)),
      sources: sourcesOf(state),
      read: null,
    }, pageSize);
  }
}

/**
 * A graph rooted at `shape`.
 *
 * @param {import("./model-shape.js").Shape} shape
 */
export function graphOf(shape) {
  return new Graph({
    tables: [{ shape, qualifier: shape.qualifier, role: "root", optional: false }],
    joins: [],
    wheres: [],
    orders: [],
    limit: null,
    offset: null,
    sliced: false,
  }, MADE);
}
