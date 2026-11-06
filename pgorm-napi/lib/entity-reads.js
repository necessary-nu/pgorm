// @ts-check

// A registered read's relations, partial projections, pages and keyset
// cursors: the values a registered `Select<E>` and `SelectGraph<E, S>` hand
// out beside their own terminals, each running one of pgorm's.
// [spec:pgorm:req:bindings.entity-reads+4]

import { TRUSTED } from "./builder.js";
import { runJob } from "./connections.js";
import { rowsOf } from "./entities.js";
import { native } from "./operations.js";
import { Select } from "./select.js";

/** Marks a construction from a registered query. */
export const READS = Symbol("pgorm-napi registered read");

/** @type {(relation: unknown) => [unknown, string]} */
export let relationOf;

/**
 * A relation a registered entity's `Relation` enum declares, by its
 * variant's name, for a query to join.
 */
export class EntityRelation {
  /** @type {unknown} */
  #entity;
  /** @type {string} */
  #entityName;
  /** @type {string} */
  #name;

  /**
   * @param {unknown} entity
   * @param {string} entityName
   * @param {string} name
   * @param {symbol} token
   */
  constructor(entity, entityName, name, token) {
    if (token !== READS) throw new TypeError("an EntityRelation comes from an entity's relation(name)");
    this.#entity = entity;
    this.#entityName = entityName;
    this.#name = name;
    Object.freeze(this);
  }

  static {
    relationOf = (relation) => {
      if (!(relation instanceof EntityRelation)) throw new TypeError("a registered query joins an EntityRelation");
      return [relation.#entity, relation.#name];
    };
  }

  /** The registration that declares the relation. */
  get entityName() {
    return this.#entityName;
  }

  /** The `Relation` variant's name. */
  get name() {
    return this.#name;
  }
}

/**
 * A registered query projected onto some of its columns, by `select_only`
 * and `columns`: each row a plain record of those columns, each read through
 * the column's `select_as`.
 */
export class EntitySelection {
  /** @type {unknown} */
  #native;
  /** @type {readonly string[]} */
  #columns;

  /**
   * @param {unknown} handle
   * @param {readonly string[]} columns
   * @param {symbol} token
   */
  constructor(handle, columns, token) {
    if (token !== READS) throw new TypeError("an EntitySelection comes from an entity query's select()");
    this.#native = handle;
    this.#columns = Object.freeze([...columns]);
    this.#statement("all");
  }

  /** The columns it reads, in order. */
  get columns() {
    return this.#columns;
  }

  /** @param {"all" | "one" | "optional"} terminal */
  #statement(terminal) {
    return new Select(native.entitySelectProjected(this.#native, this.#columns, terminal), TRUSTED);
  }

  /**
   * The SQL and values a terminal sends, `one` and `optional` narrowed to
   * one row.
   *
   * @param {"all" | "one" | "optional"} [terminal]
   */
  inspect(terminal = "all") {
    return this.#statement(terminal).inspect();
  }

  /**
   * @param {import("./connections.js").Queryable} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await db.query(this.#statement("all"), options);
  }

  /**
   * The first row, a `DecodeError` when there is none.
   *
   * @param {import("./connections.js").Queryable} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return await db.one(this.#statement("one"), options);
  }

  /**
   * @param {import("./connections.js").Queryable} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return await db.optional(this.#statement("optional"), options);
  }

  /**
   * The records one per pull, over a connection the stream holds until it
   * ends.
   *
   * @param {import("./connections.js").Pool | import("./connections.js").Connection} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    return db.stream(this.#statement("all"), options);
  }
}

/**
 * A registered read's pages, Rust's `Paginator`, which holds no database and
 * no current page: each terminal takes the database it runs on.
 */
export class EntityPaginator {
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
    if (token !== READS) throw new TypeError("an EntityPaginator comes from a registered query's paginate()");
    this.#native = handle;
    this.#entities = entities;
  }

  /**
   * Page `page`, numbered from zero, by `Paginator::fetch_page`.
   *
   * @param {unknown} db
   * @param {number} page
   * @param {{ signal?: AbortSignal }} [options]
   */
  async fetchPage(db, page, options) {
    return rowsOf(this.#entities, await runJob(db, native.pagesFetchJob(this.#native, page), options));
  }

  /**
   * The rows the pages hold and the pages they fill, by
   * `Paginator::num_items_and_pages`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numItemsAndPages(db, options) {
    const [items, pages] = /** @type {[number, number]} */ (await runJob(db, native.pagesCountJob(this.#native), options));
    return Object.freeze({ items, pages });
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numItems(db, options) {
    return (await this.numItemsAndPages(db, options)).items;
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async numPages(db, options) {
    return (await this.numItemsAndPages(db, options)).pages;
  }

  /**
   * Each page in turn from the first, ending at the first empty one.
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

  /**
   * The statement reading page `page`, or with `"count"` the one counting
   * the rows.
   *
   * @param {number | "count"} page
   */
  inspect(page) {
    const [sql, values] = native.pagesInspect(this.#native, page === "count" ? null : page);
    return Object.freeze({ sql, values: Object.freeze(values) });
  }
}

/**
 * `cursor` with a boundary of its order columns' values or its whole key's,
 * which a graph's cursor takes as `beforeWith` and `afterWith`.
 *
 * @type {<C extends EntityCursor>(cursor: C, before: boolean, values: unknown[]) => C}
 */
export let boundWhole;

/** A registered read's keyset cursor. Every method returns a new cursor. */
export class EntityCursor {
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
    if (token !== READS) throw new TypeError("a cursor comes from a registered query's cursor()");
    this.#native = handle;
    this.#entities = entities;
  }

  /**
   * A cursor of this one's class over `handle`: a graph's cursor stays one.
   *
   * @param {unknown} handle
   * @returns {this}
   */
  #next(handle) {
    const Kind = /** @type {new (handle: unknown, entities: readonly string[], token: symbol) => this} */ (this.constructor);
    return new Kind(handle, this.#entities, READS);
  }

  static {
    boundWhole = (cursor, before, values) => cursor.#next(native.cursorBound(cursor.#native, before, values, true));
  }

  /**
   * Rows short of the one whose order-column values these are.
   *
   * @param {...unknown} values
   */
  before(...values) {
    return this.#next(native.cursorBound(this.#native, true, values, false));
  }

  /**
   * Rows past the one whose order-column values these are.
   *
   * @param {...unknown} values
   */
  after(...values) {
    return this.#next(native.cursorBound(this.#native, false, values, false));
  }

  /** @param {number} rows */
  first(rows) {
    return this.#next(native.cursorWindow(this.#native, false, rows));
  }

  /** @param {number} rows */
  last(rows) {
    return this.#next(native.cursorWindow(this.#native, true, rows));
  }

  asc() {
    return this.#next(native.cursorDirection(this.#native, false));
  }

  desc() {
    return this.#next(native.cursorDirection(this.#native, true));
  }

  /** The statement `all` sends, `Cursor::statement`. */
  inspect() {
    const [sql, values] = native.cursorInspect(this.#native);
    return Object.freeze({ sql, values: Object.freeze(values) });
  }

  /**
   * The window's rows by `Cursor::all`, a `last` window's in the cursor's
   * order.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return rowsOf(this.#entities, await runJob(db, native.cursorJob(this.#native), options));
  }
}

