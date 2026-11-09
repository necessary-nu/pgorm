// @ts-check

// A registered entity's reads: its columns as expressions whose
// comparisons are the column's own `ColumnTrait` methods, and its
// `Select<E>` with Rust's terminals.
// [spec:pgorm:req:bindings.entity-reads+4]

import { arg, Handle, TRUSTED } from "./builder.js";
import { decodedStream, runJob } from "./runs.js";
import { predicateOf, recordsOf, rowsOf } from "./entities.js";
import { EntityCursor, EntityPaginator, EntitySelection, READS, relationOf } from "./entity-reads.js";
import { Expr, listedValues, OrderBy, Quantified } from "./expressions.js";
import { native } from "./operations.js";
import { decodedWith } from "./streams.js";

/** Marks a construction from a registered entity. */
export const QUERIES = Symbol("pgorm-napi entity query");

/**
 * The registered entity and column name `value` is, when it is an
 * `EntityColumn`: what a pipeline reads a registered column by.
 *
 * @type {(value: unknown) => [unknown, string] | undefined}
 */
export let registeredColumn;

/**
 * A registered entity's column. A value it is compared with converts to the
 * column's declared kind and is written through its `save_as`, by the
 * column's own `ColumnTrait` method, and so do `between`'s bounds, the value
 * of a distinctness test or a range operator, the values of `any(..)` or
 * `all(..)` over a list, and those of a list it is tested for membership in;
 * a subquery's rows are compared by the trait's `in_subquery`; an
 * expression, and a quantifier over a `Select` or an expression, are
 * compared as written.
 */
// [spec:pgorm:req:bindings.column-operators+3]
export class EntityColumn extends Expr {
  /** @type {unknown} */
  #entity;
  /** @type {string} */
  #column;

  /**
   * @param {unknown} handle
   * @param {unknown} entity
   * @param {string} column
   * @param {symbol} token
   */
  constructor(handle, entity, column, token) {
    super(handle, token);
    this.#entity = entity;
    this.#column = column;
  }

  static {
    registeredColumn = (value) => value instanceof EntityColumn ? [value.#entity, value.#column] : undefined;
  }

  /** The column's SQL name. */
  get name() {
    return this.#column;
  }

  /**
   * Membership through the column's `ColumnTrait`, or `null` for a list
   * holding an expression, which is a member as written.
   *
   * @param {boolean} negated
   * @param {readonly unknown[] | import("./select.js").Select} source
   */
  #membership(negated, source) {
    if (Array.isArray(source)) {
      if (source.some((item) => item instanceof Handle)) return null;
      return new Expr(native.entityIn(this.#entity, this.#column, negated, source), TRUSTED);
    }
    return new Expr(native.entityIn(this.#entity, this.#column, negated, arg(source)), TRUSTED);
  }

  /**
   * @override
   * @param {readonly unknown[] | import("./select.js").Select} source
   */
  isIn(source) {
    return this.#membership(false, source) ?? super.isIn(source);
  }

  /**
   * @override
   * @param {readonly unknown[] | import("./select.js").Select} source
   */
  isNotIn(source) {
    return this.#membership(true, source) ?? super.isNotIn(source);
  }

  /**
   * The column compared through its `ColumnTrait`: a value as the trait's
   * method of the operator's meaning compares one, and `any(..)` or `all(..)`
   * over a list as its `compare_any` or `compare_all` does, or `null` for an
   * operand compared as written.
   *
   * @param {string} operator
   * @param {unknown} other
   */
  #compare(operator, other) {
    const listed = listedValues(other);
    if (listed !== undefined && !operator.endsWith("DistinctFrom")) {
      const [quantifier, values] = listed;
      const quantified = native.entityQuantified(this.#entity, this.#column, operator, quantifier, values);
      if (quantified !== null) return new Expr(quantified, TRUSTED);
    }
    if (other instanceof Expr || other instanceof Quantified) return null;
    return new Expr(native.entityCompare(this.#entity, this.#column, operator, other), TRUSTED);
  }

  /**
   * `[NOT] BETWEEN [SYMMETRIC]` through the column's `ColumnTrait`, or `null`
   * for a bound that is an expression, which leaves the test to `Expr`'s,
   * both bounds as written.
   *
   * @param {boolean} negated
   * @param {unknown} lower
   * @param {unknown} upper
   * @param {{ symmetric?: boolean }} options
   */
  #between(negated, lower, upper, { symmetric = false } = {}) {
    if (lower instanceof Expr || upper instanceof Expr) return null;
    return new Expr(
      native.entityBetween(this.#entity, this.#column, negated, symmetric === true, lower, upper),
      TRUSTED,
    );
  }

  /**
   * @override
   * @param {unknown} lower
   * @param {unknown} upper
   * @param {{ symmetric?: boolean }} [options]
   */
  between(lower, upper, options) {
    return this.#between(false, lower, upper, options) ?? super.between(lower, upper, options);
  }

  /**
   * @override
   * @param {unknown} lower
   * @param {unknown} upper
   * @param {{ symmetric?: boolean }} [options]
   */
  notBetween(lower, upper, options) {
    return this.#between(true, lower, upper, options) ?? super.notBetween(lower, upper, options);
  }

  /**
   * @override
   * @param {unknown} other
   */
  isDistinctFrom(other) {
    return this.#compare("isDistinctFrom", other) ?? super.isDistinctFrom(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  isNotDistinctFrom(other) {
    return this.#compare("isNotDistinctFrom", other) ?? super.isNotDistinctFrom(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  contains(other) {
    return this.#compare("contains", other) ?? super.contains(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  containedBy(other) {
    return this.#compare("containedBy", other) ?? super.containedBy(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  overlaps(other) {
    return this.#compare("overlaps", other) ?? super.overlaps(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  strictlyLeftOf(other) {
    return this.#compare("strictlyLeftOf", other) ?? super.strictlyLeftOf(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  strictlyRightOf(other) {
    return this.#compare("strictlyRightOf", other) ?? super.strictlyRightOf(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  notExtendRightOf(other) {
    return this.#compare("notExtendRightOf", other) ?? super.notExtendRightOf(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  notExtendLeftOf(other) {
    return this.#compare("notExtendLeftOf", other) ?? super.notExtendLeftOf(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  adjacentTo(other) {
    return this.#compare("adjacentTo", other) ?? super.adjacentTo(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  eq(other) {
    return this.#compare("eq", other) ?? super.eq(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  ne(other) {
    return this.#compare("ne", other) ?? super.ne(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  gt(other) {
    return this.#compare("gt", other) ?? super.gt(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  gte(other) {
    return this.#compare("gte", other) ?? super.gte(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  lt(other) {
    return this.#compare("lt", other) ?? super.lt(other);
  }

  /**
   * @override
   * @param {unknown} other
   */
  lte(other) {
    return this.#compare("lte", other) ?? super.lte(other);
  }
}

/** A registered entity's `Select<E>`. Every method returns a new query. */
export class EntityQuery {
  /** @type {unknown} */
  #native;
  /** @type {string} */
  #entity;

  /**
   * @param {unknown} handle
   * @param {string} entity
   * @param {symbol} token
   */
  constructor(handle, entity, token) {
    if (token !== QUERIES) throw new TypeError("an EntityQuery comes from an entity's find()");
    this.#native = handle;
    this.#entity = entity;
  }

  /**
   * @param {string} step
   * @param {unknown} argument
   */
  #step(step, argument) {
    return new EntityQuery(native.entitySelectChange(this.#native, step, argument), this.#entity, QUERIES);
  }

  /**
   * Rows that also satisfy `predicate`, by `QueryFilter::filter`.
   *
   * @param {unknown} predicate
   */
  where(predicate) {
    return this.#step("where", arg(predicateOf(predicate)));
  }

  /** @param {...unknown} orderings */
  orderBy(...orderings) {
    /** @type {EntityQuery} */
    let query = this;
    for (const ordering of orderings) {
      if (!(ordering instanceof OrderBy)) throw new TypeError("an ordering is expr.asc() or expr.desc()");
      query = query.#step("orderBy", arg(ordering));
    }
    return query;
  }

  /** @param {number | null} count */
  limit(count) {
    return this.#step("limit", count);
  }

  /** @param {number | null} count */
  offset(count) {
    return this.#step("offset", count);
  }

  /**
   * The SQL and values a terminal sends: `one` and `optional` with the
   * `LIMIT 1` Rust's terminals add, `count` the paginator's count.
   *
   * @param {"all" | "one" | "optional" | "count"} [terminal]
   */
  inspect(terminal = "all") {
    if (terminal === "count") return this.paginate(1).inspect("count");
    const [sql, values] = native.entitySelectInspect(this.#native, terminal);
    return Object.freeze({ sql, values: Object.freeze(values) });
  }

  /**
   * @param {"all" | "one" | "optional"} terminal
   * @param {unknown} db
   * @param {unknown} options
   */
  async #read(terminal, db, options) {
    return recordsOf(this.#entity, await runJob(db, native.entitySelectJob(this.#native, terminal), options));
  }

  /**
   * Every row, by `Select::all`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await this.#read("all", db, options);
  }

  /**
   * The first row, by `Select::one`; none is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return (await this.#read("one", db, options))[0];
  }

  /**
   * The first row or null, by `Select::one_opt`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return (await this.#read("optional", db, options))[0] ?? null;
  }

  /**
   * The relation's far end joined, by `QuerySelect::join`: the rows stay this
   * entity's records, and the joined table's columns go in conditions and
   * orderings through its own entity's `col`.
   *
   * @param {unknown} relation
   * @param {{ kind?: "inner" | "left" | "right" | "full" }} [options]
   */
  join(relation, { kind = "inner" } = {}) {
    const [entity, name] = relationOf(relation);
    return new EntityQuery(native.entitySelectJoin(this.#native, entity, name, kind), this.#entity, QUERIES);
  }

  /**
   * The query projected onto `columns`, by `select_only` and `columns`, read
   * as plain records of those columns rather than as the entity's.
   *
   * @param {...string} columns
   */
  select(...columns) {
    return new EntitySelection(this.#native, columns, READS);
  }

  /**
   * How many rows the query reads, its limit, offset and ordering aside, by
   * `PaginatorTrait::count`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    return (await this.paginate(1).numItemsAndPages(db, options)).items;
  }

  /**
   * Pages of `pageSize` rows, by `PaginatorTrait::paginate`.
   *
   * @param {number} pageSize
   */
  paginate(pageSize) {
    return new EntityPaginator(native.entitySelectPages(this.#native, pageSize), [this.#entity], READS);
  }

  /**
   * A keyset cursor ordered by `columns`, by `Select::cursor_by`; its
   * ordering replaces the query's.
   *
   * @param {...string} columns
   */
  cursor(...columns) {
    return new EntityCursor(native.entitySelectCursor(this.#native, columns), [this.#entity], READS);
  }

  /**
   * The records one per pull, decoded as `Select::stream` decodes each row as
   * it arrives, over a connection the stream holds until it ends.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    const [statement, decoder] = native.entitySelectStream(this.#native);
    const entities = [this.#entity];
    const stream = decodedStream(db, statement, decoder, options, "a registered read");
    return decodedWith(stream, (_names, pulled) => rowsOf(entities, /** @type {[string[][], ([unknown[], unknown] | null)[][]]} */ (pulled))[0]);
  }
}
