// @ts-check

// Keyset cursors and pages over a model's query or a graph, as pgorm's
// `Cursor` and `Paginator` read them: a cursor orders by its columns and
// tiebreaks, and resumes past or short of a boundary row's key; a paginator
// reads a page at a time by limit and offset and counts the rows it pages.
// [spec:pgorm:req:bindings.cursors]
// [spec:pgorm:req:bindings.pagination]

import { Condition } from "./conditions.js";
import { ConstructionError } from "./errors.js";
import { columnValue } from "./model-columns.js";
import { decodeAll, decodeCount } from "./model-decode.js";
import { qualified } from "./model-shape.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi cursor");

/**
 * One column of a cursor's key, qualified by the table it is read from.
 *
 * @typedef {{ field: import("./model-shape.js").Field, qualifier: string }} KeyColumn
 */

/**
 * What a cursor pages over: the statement it composes with an ordering,
 * further conditions and a limit; the sources its rows decode as; its order
 * columns; and the tiebreaks after them.
 *
 * @typedef {object} Keyed
 * @property {(orders: readonly import("./expressions.js").OrderBy[], wheres: readonly (import("./expressions.js").Expr | Condition)[], limit: number | bigint | null) => import("./select.js").Select} build
 * @property {number | bigint | null} limit The query's own limit, which a window replaces.
 * @property {readonly import("./model-decode.js").Source[]} sources
 * @property {readonly KeyColumn[]} order
 * @property {readonly KeyColumn[]} tiebreaks
 */

/**
 * @typedef {object} State
 * @property {Keyed} keyed
 * @property {unknown[] | null} after
 * @property {unknown[] | null} before
 * @property {{ last: boolean, rows: number } | null} window
 * @property {boolean} asc
 */

/**
 * A count of rows: a non-negative safe integer.
 *
 * @param {unknown} rows
 * @param {string} what
 */
function rowCount(rows, what) {
  if (typeof rows !== "number") throw new TypeError(`${what} is a number`);
  if (!Number.isSafeInteger(rows) || rows < 0) throw new ConstructionError(`${what} is a non-negative integer`);
  return rows;
}

/**
 * A keyset cursor. Every method returns a new cursor; `all` reads the
 * window it describes, in the cursor's order.
 */
export class Cursor {
  /** @type {State} */
  #state;

  /**
   * @param {State} state
   * @param {symbol} token
   */
  constructor(state, token) {
    if (token !== MADE) throw new TypeError("a Cursor comes from a model query's or a graph's cursor()");
    this.#state = Object.freeze(state);
  }

  /** @param {Partial<State>} change */
  #with(change) {
    return new Cursor({ ...this.#state, ...change }, MADE);
  }

  /** The whole key, in comparison order: the order columns, then the tiebreaks. */
  #keyset() {
    return [...this.#state.keyed.order, ...this.#state.keyed.tiebreaks];
  }

  /**
   * The boundary `values` names, converted through each key column's
   * declared kind: as many as the order columns, or with `extended` as many
   * as the whole key too.
   *
   * @param {readonly unknown[]} values
   * @param {boolean} extended
   */
  #boundary(values, extended) {
    const keyset = this.#keyset();
    const primary = this.#state.keyed.order.length;
    if (values.length !== primary && !(extended && values.length === keyset.length)) {
      const expected = extended && keyset.length > primary ? `${primary} or ${keyset.length}` : `${primary}`;
      throw new ConstructionError(`a cursor boundary of ${values.length} values does not match ${expected} key column(s)`);
    }
    return values.map((value, index) => {
      const { field } = /** @type {KeyColumn} */ (keyset[index]);
      return columnValue(field.column, field.field, value);
    });
  }

  /**
   * Rows past the one whose order-column values these are.
   *
   * @param {...unknown} values
   */
  after(...values) {
    return this.#with({ after: this.#boundary(values, false) });
  }

  /**
   * Rows short of the one whose order-column values these are.
   *
   * @param {...unknown} values
   */
  before(...values) {
    return this.#with({ before: this.#boundary(values, false) });
  }

  /**
   * Rows past the one whose whole key this is — the order columns, then the
   * tiebreaks — so a page that ended inside a run of equal order values
   * resumes inside it.
   *
   * @param {...unknown} values
   */
  afterWith(...values) {
    return this.#with({ after: this.#boundary(values, true) });
  }

  /** @param {...unknown} values */
  beforeWith(...values) {
    return this.#with({ before: this.#boundary(values, true) });
  }

  /**
   * The first `rows` rows in the cursor's order, replacing any window.
   *
   * @param {number} rows
   */
  first(rows) {
    return this.#with({ window: { last: false, rows: rowCount(rows, "a window's rows") } });
  }

  /**
   * The last `rows` rows in the cursor's order, replacing any window.
   *
   * @param {number} rows
   */
  last(rows) {
    return this.#with({ window: { last: true, rows: rowCount(rows, "a window's rows") } });
  }

  asc() {
    return this.#with({ asc: true });
  }

  desc() {
    return this.#with({ asc: false });
  }

  /**
   * The key compared with `values` by `beyond` at the last column each
   * disjunct reaches: `(c1, .., cn) ⋈ (v1, .., vn)` written out as n
   * disjuncts, the k-th holding the first k-1 columns equal.
   *
   * @param {readonly unknown[]} values
   * @param {"gt" | "lt"} beyond
   */
  #compared(values, beyond) {
    const columns = this.#keyset().slice(0, values.length).map(({ field, qualifier }) => qualified(field, qualifier));
    const disjuncts = [];
    for (let reach = columns.length; reach >= 1; reach -= 1) {
      disjuncts.push(Condition.all(...columns.slice(0, reach).map((column, index) =>
        index + 1 === reach ? column[beyond](values[index]) : column.eq(values[index])
      )));
    }
    return Condition.any(...disjuncts);
  }

  /** The SELECT the cursor runs: window, order and boundaries applied. */
  get statement() {
    const { keyed, after, before, window, asc } = this.#state;
    const reversed = window?.last === true;
    const direction = asc !== reversed ? "asc" : "desc";
    const orders = this.#keyset().map(({ field, qualifier }) => qualified(field, qualifier)[direction]());
    const wheres = [];
    if (after !== null) wheres.push(this.#compared(after, asc ? "gt" : "lt"));
    if (before !== null) wheres.push(this.#compared(before, asc ? "lt" : "gt"));
    return keyed.build(orders, wheres, window === null ? keyed.limit : window.rows);
  }

  inspect() {
    return this.statement.inspect();
  }

  /**
   * The window's rows, in the cursor's order: a `last` window is read
   * backwards and returned forwards.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    const rows = await decodeAll(db, this.statement, this.#state.keyed.sources, options);
    return this.#state.window?.last ? rows.reverse() : rows;
  }
}

/**
 * A cursor over `keyed`, ascending, with no boundary or window.
 *
 * @param {Keyed} keyed
 */
export function cursorOver(keyed) {
  return new Cursor({ keyed, after: null, before: null, window: null, asc: true }, MADE);
}

/**
 * What a paginator pages: the statement of one page, and the statement
 * whose rows it counts.
 *
 * @typedef {object} Pageable
 * @property {(limit: number, offset: number) => import("./select.js").Select | import("./raw-sql.js").Held} page
 * @property {import("./select.js").Select | import("./raw-sql.js").Held} counted
 * @property {readonly import("./model-decode.js").Source[] | null} sources The sources each row
 *   decodes as, or `null` for rows as plain objects.
 * @property {boolean} [exhaustive] Whether every column is a field's, as a built query's is.
 */

/**
 * Pages of a query's rows, `pageSize` at a time, numbered from zero: each a
 * limit and offset over the query, which should be ordered for its pages to
 * be stable.
 */
export class Paginator {
  /** @type {Pageable} */
  #pageable;
  /** @type {number} */
  #pageSize;

  /**
   * @param {Pageable} pageable
   * @param {unknown} pageSize
   * @param {symbol} token
   */
  constructor(pageable, pageSize, token) {
    if (token !== MADE) throw new TypeError("a Paginator comes from a model query's or a graph's paginate()");
    if (rowCount(pageSize, "a page size") === 0) throw new ConstructionError("a page size is at least 1");
    this.#pageable = pageable;
    this.#pageSize = /** @type {number} */ (pageSize);
  }

  get pageSize() {
    return this.#pageSize;
  }

  /**
   * The SQL and values reading page `page`, or with `"count"` counting the
   * rows.
   *
   * @param {number | "count"} page
   */
  inspect(page) {
    return (page === "count" ? this.#pageable.counted : this.#page(page)).inspect();
  }

  /** @param {number} page */
  #page(page) {
    const offset = this.#pageSize * rowCount(page, "a page number");
    if (!Number.isSafeInteger(offset)) {
      throw new ConstructionError(`page ${page} of ${this.#pageSize} rows is past the offsets a number holds exactly`);
    }
    return this.#pageable.page(this.#pageSize, offset);
  }

  /**
   * @param {unknown} db
   * @param {number} page
   * @param {{ signal?: AbortSignal }} [options]
   */
  async fetchPage(db, page, options) {
    const { sources, exhaustive = true } = this.#pageable;
    if (sources === null) {
      const query = /** @type {{ query?: unknown }} */ (db)?.query;
      if (typeof query !== "function") throw new TypeError("a page is read on a Pool, a Connection or a Transaction");
      return await query.call(db, this.#page(page), options);
    }
    return await decodeAll(db, this.#page(page), sources, options, exhaustive);
  }

  /**
   * How many rows the pages hold.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numItems(db, options) {
    return await decodeCount(db, this.#pageable.counted, options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numPages(db, options) {
    return (await this.numItemsAndPages(db, options)).pages;
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numItemsAndPages(db, options) {
    const items = await this.numItems(db, options);
    return Object.freeze({ items, pages: Math.ceil(items / this.#pageSize) });
  }

  /**
   * Each page in turn, from the first, ending at the first empty one.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async *pages(db, options) {
    for (let page = 0;; page += 1) {
      const rows = await this.fetchPage(db, page, options);
      if (rows.length === 0) return;
      yield rows;
    }
  }
}

/**
 * Pages of `pageable`.
 *
 * @param {Pageable} pageable
 * @param {unknown} pageSize
 */
export function paginatorOver(pageable, pageSize) {
  return new Paginator(pageable, pageSize, MADE);
}
