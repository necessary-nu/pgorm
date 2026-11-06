// @ts-check

// What a disjoint model's statements run and are read through: the reads,
// their pages and cursor, and a write's terminal. Every statement is the
// native shape's; a row read back is viewed through the model's `View`.
// [spec:pgorm:req:bindings.disjoint+2]

import { arg, TRUSTED } from "./builder.js";
import { decodedStream, runJob } from "./connections.js";
import { predicateOf } from "./entities.js";
import { ConstructionError } from "./errors.js";
import { OrderBy } from "./expressions.js";
import { native } from "./operations.js";
import { Select } from "./select.js";
import { decodedWith } from "./streams.js";

/** Marks a construction from within the disjoint modules. */
export const MADE = Symbol("pgorm-napi disjoint");

/** PostgreSQL reads LIMIT and OFFSET as bigint; a number holds this many exactly. */
const ROWS_MAX = Number.MAX_SAFE_INTEGER;

/**
 * @param {unknown} value
 * @param {string} what
 */
function rows(value, what) {
  if (typeof value !== "number" || !Number.isInteger(value)) throw new TypeError(`${what} is an integer`);
  if (value < 0 || value > ROWS_MAX) throw new ConstructionError(`${what} is a non-negative safe integer`);
  return value;
}

/**
 * @param {unknown} options
 * @returns {{ signal?: AbortSignal }}
 */
function terminalOptions(options) {
  if (options === undefined) return {};
  if (typeof options !== "object" || options === null) throw new TypeError("options is an object");
  for (const key of Object.keys(options)) {
    if (key !== "signal") throw new TypeError(`${JSON.stringify(key)} is no option of a disjoint read`);
  }
  return /** @type {{ signal?: AbortSignal }} */ (options);
}

/**
 * How a disjoint model reads its key and its rows: a model's through its
 * fields, a registration's through its entities.
 *
 * @typedef {object} View
 * @property {(key: unknown) => unknown[]} key The key's values in key order, as the native read takes them.
 * @property {(outcome: any) => any[]} rows Each row of a native read: `[base record, kind, kind record or null]`.
 * @property {(fields: string[]) => string[]} columns The base's columns a cursor ordered by `fields` orders by.
 * @property {(fields: readonly string[], values: unknown[]) => unknown[]} boundary A cursor boundary's values for `fields`, as the native cursor binds them.
 */

/**
 * A disjoint model's native handle.
 *
 * @type {(model: Disjoined) => unknown}
 */
export let handleOf;

/**
 * How a disjoint model reads its key and its rows.
 *
 * @type {(model: Disjoined) => View}
 */
export let viewOf;

/**
 * What a disjoint model and a registered one share.
 */
export class Disjoined {
  /** @type {unknown} */
  #native;
  /** @type {View} */
  #view;

  /**
   * @param {unknown} handle
   * @param {View} view
   */
  constructor(handle, view) {
    this.#native = handle;
    this.#view = view;
  }

  static {
    handleOf = (model) => model.#native;
    viewOf = (model) => model.#view;
  }

  /** The disjoint model's name. */
  get name() {
    return /** @type {string} */ (this.describe().name);
  }

  /**
   * The declaration as data: its `name`, its `base` model's name, its
   * `kindField`, its `kindType`'s `schema` and `name`, and its `kinds`, each
   * a `value` and its `model`'s name or null.
   */
  describe() {
    return JSON.parse(native.disjointDescribe(this.#native));
  }

  /** Every base row with its kind's row, in one statement. */
  find() {
    return new DisjointQuery(this, new Select(native.disjointFind(this.#native), TRUSTED), MADE);
  }

  /**
   * The base row of the key `key` gives, every key part and no other named.
   *
   * @param {unknown} key
   */
  findByKey(key) {
    const select = native.disjointNarrowKey(this.#native, native.disjointFind(this.#native), this.#view.key(key));
    return new DisjointQuery(this, new Select(select, TRUSTED), MADE);
  }
}

/**
 * A read of a disjoint model: each base row with its kind's row, one row of
 * the statement each, given as `[base record, kind, kind record or null]`.
 */
export class DisjointQuery {
  /** @type {Disjoined} */
  #owner;
  /** @type {Select} */
  #select;

  /**
   * @param {Disjoined} owner
   * @param {Select} select
   * @param {symbol} token
   */
  constructor(owner, select, token) {
    if (token !== MADE) throw new TypeError("a DisjointQuery comes from a disjoint model's find()");
    this.#owner = owner;
    this.#select = select;
    Object.freeze(this);
  }

  /** @param {Select} select */
  #with(select) {
    return new DisjointQuery(this.#owner, select, MADE);
  }

  /**
   * Rows passing `predicate`, an ordinary `WHERE`: a kind table's column is
   * NULL in every row of another kind.
   *
   * @param {unknown} predicate
   */
  where(predicate) {
    return this.#with(this.#select.where(predicateOf(predicate)));
  }

  /** @param {...unknown} orderings */
  orderBy(...orderings) {
    for (const ordering of orderings) {
      if (!(ordering instanceof OrderBy)) throw new TypeError("an ordering is an expression's asc() or desc()");
    }
    return this.#with(this.#select.orderBy(.../** @type {OrderBy[]} */ (orderings)));
  }

  /** @param {unknown} count */
  limit(count) {
    return this.#with(this.#select.limit(count === null ? null : rows(count, "a limit")));
  }

  /** @param {unknown} count */
  offset(count) {
    return this.#with(this.#select.offset(count === null ? null : rows(count, "an offset")));
  }

  /**
   * Only base rows of the kind `kind`.
   *
   * @param {unknown} kind
   */
  kind(kind) {
    if (typeof kind !== "string") throw new TypeError("a kind is its value, a string");
    return this.#with(new Select(native.disjointNarrowKind(handleOf(this.#owner), arg(this.#select), kind), TRUSTED));
  }

  /**
   * Only base rows of one of `kinds`.
   *
   * @param {unknown} kinds
   */
  kinds(kinds) {
    if (!Array.isArray(kinds) || !kinds.every((kind) => typeof kind === "string")) {
      throw new TypeError("kinds are an array of kinds' values");
    }
    return this.#with(new Select(native.disjointNarrowKinds(handleOf(this.#owner), arg(this.#select), kinds), TRUSTED));
  }

  /**
   * Only base rows of the kind `kind` whose row passes `predicate`: the kind
   * is part of the condition, so one a missing row satisfies reads no row of
   * another kind.
   *
   * @param {unknown} kind
   * @param {unknown} predicate
   */
  filterKind(kind, predicate) {
    if (typeof kind !== "string") throw new TypeError("a kind is its value, a string");
    const select = native.disjointNarrowFilterKind(handleOf(this.#owner), arg(this.#select), kind, arg(predicateOf(predicate)));
    return this.#with(new Select(select, TRUSTED));
  }

  /**
   * The SQL and values `all` or `one` sends.
   *
   * @param {{ terminal?: "all" | "one" }} [options]
   * @returns {import("./index.d.ts").Compiled}
   */
  inspect(options = {}) {
    const { terminal = "all" } = options;
    if (terminal !== "all" && terminal !== "one") throw new TypeError('a disjoint read inspects "all" or "one"');
    return inspectOf(this.#owner, this.#select, terminal);
  }

  /**
   * Every row; a row the constraints do not admit is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await runOn(this.#owner, db, this.#select, "all", options);
  }

  /**
   * The first row of the read narrowed to one, a `DecodeError` when there is
   * none.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return (await runOn(this.#owner, db, this.#select, "one", options))[0];
  }

  /**
   * The rows as they arrive, over a pool's or a connection's connection.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    const owner = this.#owner;
    const stream = decodedStream(db, arg(this.#select), handleOf(owner), terminalOptions(options), "a disjoint read", true);
    return decodedWith(stream, (_names, pulled) => viewOf(owner).rows(pulled)[0]);
  }

  /**
   * The base rows the read matches, counted as pgorm's paginator counts.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    const counted = native.disjointCounted(handleOf(this.#owner), arg(this.#select));
    const job = native.disjointJob(handleOf(this.#owner), counted, "count");
    return /** @type {number} */ (await runJob(db, job, terminalOptions(options)));
  }

  /**
   * Pages of `pageSize` rows, numbered from zero.
   *
   * @param {unknown} pageSize
   */
  paginate(pageSize) {
    return new DisjointPaginator(this.#owner, this.#select, rows(pageSize, "a page size"), MADE);
  }

  /**
   * A keyset cursor ordered by the base's `fields`, its boundaries bound
   * under their columns' casts.
   *
   * @param {...unknown} fields
   */
  cursor(...fields) {
    if (fields.length === 0) throw new ConstructionError("a cursor is ordered by at least one field");
    if (!fields.every((field) => typeof field === "string")) throw new TypeError("a cursor's field is a string");
    const owner = this.#owner;
    const columns = viewOf(owner).columns(/** @type {string[]} */ (fields));
    return new DisjointCursor(owner, this.#select, Object.freeze({
      fields: Object.freeze(/** @type {string[]} */ (fields)), columns, after: null, before: null, window: null, descending: false,
    }), MADE);
  }
}

/**
 * @param {Disjoined} owner
 * @param {Select} select
 * @param {"all" | "one" | "write"} terminal
 * @returns {import("./index.d.ts").Compiled}
 */
function inspectOf(owner, select, terminal) {
  const [sql, values] = native.disjointInspect(handleOf(owner), arg(select), terminal);
  return Object.freeze({ sql, values: Object.freeze(values) });
}

/**
 * @param {Disjoined} owner
 * @param {unknown} db
 * @param {Select} select
 * @param {"all" | "one" | "write"} terminal
 * @param {unknown} options
 */
async function runOn(owner, db, select, terminal, options) {
  const job = native.disjointJob(handleOf(owner), arg(select), terminal);
  return viewOf(owner).rows(await runJob(db, job, terminalOptions(options)));
}

/**
 * An insert, update or change of kind: one statement, its parts joined by
 * data-modifying `WITH` clauses, which reads back the written row.
 */
export class DisjointWrite {
  /** @type {Disjoined} */
  #owner;
  /** @type {Select} */
  #select;

  /**
   * @param {Disjoined} owner
   * @param {unknown} handle
   * @param {symbol} token
   */
  constructor(owner, handle, token) {
    if (token !== MADE) throw new TypeError("a DisjointWrite comes from a disjoint model's insert, update or changeKind");
    this.#owner = owner;
    this.#select = new Select(handle, TRUSTED);
    Object.freeze(this);
  }

  /** @returns {import("./index.d.ts").Compiled} */
  inspect() {
    return inspectOf(this.#owner, this.#select, "write");
  }

  /**
   * Send the write and read back what a read of the written row gives. A
   * write that touches no row — a key no row has, a row of another kind, a
   * change to the row's own kind — changes nothing and is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningModel(db, options) {
    return (await runOn(this.#owner, db, this.#select, "write", options))[0];
  }
}

/**
 * Pages of a disjoint read, each the read with pgorm's paginator's limit and
 * offset, replacing its own.
 */
export class DisjointPaginator {
  /** @type {Disjoined} */
  #owner;
  /** @type {Select} */
  #select;
  /** @type {number} */
  #pageSize;

  /**
   * @param {Disjoined} owner
   * @param {Select} select
   * @param {number} pageSize
   * @param {symbol} token
   */
  constructor(owner, select, pageSize, token) {
    if (token !== MADE) throw new TypeError("a DisjointPaginator comes from a disjoint read's paginate()");
    if (pageSize < 1) throw new ConstructionError("a page size is at least 1");
    this.#owner = owner;
    this.#select = select;
    this.#pageSize = pageSize;
    Object.freeze(this);
  }

  /** @param {unknown} page */
  #page(page) {
    return new Select(native.disjointPage(handleOf(this.#owner), arg(this.#select), this.#pageSize, rows(page, "a page number")), TRUSTED);
  }

  /**
   * The SQL and values reading page `page`, or with `"count"` the ones
   * counting the rows.
   *
   * @param {number | "count"} page
   * @returns {import("./index.d.ts").Compiled}
   */
  inspect(page) {
    if (page === "count") {
      return inspectOf(this.#owner, new Select(native.disjointCounted(handleOf(this.#owner), arg(this.#select)), TRUSTED), "all");
    }
    if (typeof page === "string") throw new TypeError('a paginator inspects a page, or its "count"');
    return inspectOf(this.#owner, this.#page(page), "all");
  }

  /**
   * @param {unknown} db
   * @param {unknown} page
   * @param {{ signal?: AbortSignal }} [options]
   */
  async fetchPage(db, page, options) {
    return await runOn(this.#owner, db, this.#page(page), "all", options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numItems(db, options) {
    return await new DisjointQuery(this.#owner, this.#select, MADE).count(db, options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numPages(db, options) {
    return (await this.numItemsAndPages(db, options)).pages;
  }

  /**
   * The rows the pages hold and the pages, the last one rounded up, as a
   * model query's paginator gives them.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   * @returns {Promise<{ readonly items: number, readonly pages: number }>}
   */
  async numItemsAndPages(db, options) {
    const items = await this.numItems(db, options);
    return Object.freeze({ items, pages: Math.ceil(items / this.#pageSize) });
  }

  /**
   * Each page in turn from the first, ending at the first empty one.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async *pages(db, options) {
    for (let page = 0; ; page += 1) {
      const found = await this.fetchPage(db, page, options);
      if (found.length === 0) return;
      yield found;
    }
  }
}

/**
 * @typedef {object} Position
 * @property {readonly string[]} fields
 * @property {string[]} columns
 * @property {unknown[] | null} after
 * @property {unknown[] | null} before
 * @property {[boolean, number] | null} window
 * @property {boolean} descending
 */

/**
 * A keyset cursor over a disjoint read, as pgorm's `Cursor` pages one:
 * ordered by base fields, each base row one row of the read, so the order
 * fields are the whole keyset. Every method returns a new cursor.
 */
export class DisjointCursor {
  /** @type {Disjoined} */
  #owner;
  /** @type {Select} */
  #select;
  /** @type {Position} */
  #position;

  /**
   * @param {Disjoined} owner
   * @param {Select} select
   * @param {Position} position
   * @param {symbol} token
   */
  constructor(owner, select, position, token) {
    if (token !== MADE) throw new TypeError("a DisjointCursor comes from a disjoint read's cursor()");
    this.#owner = owner;
    this.#select = select;
    this.#position = position;
    Object.freeze(this);
  }

  /** @param {Partial<Position>} change */
  #with(change) {
    return new DisjointCursor(this.#owner, this.#select, Object.freeze({ ...this.#position, ...change }), MADE);
  }

  /** @param {unknown[]} values */
  #boundary(values) {
    const { fields } = this.#position;
    if (values.length !== fields.length) {
      throw new ConstructionError(`a cursor boundary of ${values.length} values does not match ${fields.length} order field(s)`);
    }
    return viewOf(this.#owner).boundary(/** @type {readonly string[]} */ (fields), values);
  }

  /** @param {...unknown} values */
  after(...values) {
    return this.#with({ after: this.#boundary(values) });
  }

  /** @param {...unknown} values */
  before(...values) {
    return this.#with({ before: this.#boundary(values) });
  }

  /** @param {unknown} count */
  first(count) {
    return this.#with({ window: [false, rows(count, "a window's rows")] });
  }

  /** @param {unknown} count */
  last(count) {
    return this.#with({ window: [true, rows(count, "a window's rows")] });
  }

  asc() {
    return this.#with({ descending: false });
  }

  desc() {
    return this.#with({ descending: true });
  }

  /** @returns {[Select, boolean]} */
  #statement() {
    const { columns, after, before, window, descending } = this.#position;
    const [select, reversed] = native.disjointCursor(
      handleOf(this.#owner), arg(this.#select), columns, after, before, window, descending,
    );
    return [new Select(select, TRUSTED), reversed];
  }

  /** @returns {import("./index.d.ts").Compiled} */
  inspect() {
    return inspectOf(this.#owner, this.#statement()[0], "all");
  }

  /**
   * The window's rows in the cursor's order: a `last` window is read
   * backwards and returned forwards.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    const [select, reversed] = this.#statement();
    const found = await runOn(this.#owner, db, select, "all", options);
    return reversed ? found.reverse() : found;
  }
}
