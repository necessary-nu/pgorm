// @ts-check

// SELECT, the tables and other items it reads, and common table expressions.
// [spec:pgorm:req:bindings.select+4]
// [spec:pgorm:req:bindings.from-subqueries]
// [spec:pgorm:req:bindings.row-locks]
// [spec:pgorm:req:bindings.with+1]
// [spec:pgorm:req:bindings.cte-options]

import { arg, args, Builder, trusted, TRUSTED } from "./builder.js";
import { Expr } from "./expressions.js";
import { native } from "./operations.js";

/** @type {(handle: unknown) => Table} */
let tableOf;

/** A table, schema-qualified and aliased when the options say. */
export class Table extends Builder {
  /**
   * @param {string} name
   * @param {{ schema?: string, alias?: string }} [options]
   * @param {symbol} [token]
   */
  constructor(name, options = {}, token) {
    if (token === TRUSTED) {
      super(name);
      return;
    }
    const { schema, alias } = options;
    super(native.tableNew(name, schema, alias));
  }

  static {
    tableOf = (handle) => new Table(/** @type {any} */ (handle), {}, TRUSTED);
  }

  /** @param {string} alias */
  as(alias) {
    return tableOf(native.tableAlias(arg(this), alias));
  }

  /**
   * A column, qualified by the table's alias if it has one, else by its name.
   *
   * @param {string} name
   */
  col(name) {
    return new Expr(native.tableCol(arg(this), name), TRUSTED);
  }

  /** Every column, `"table".*`. */
  star() {
    return new Expr(native.tableStar(arg(this)), TRUSTED);
  }

  /**
   * The table read through `TABLESAMPLE`: each row (`"bernoulli"`) or page
   * (`"system"`) drawn with probability `percentage` out of 100, and the same
   * sample drawn again for the same `repeatable` seed over unchanged data.
   * Both numbers are bound; one PostgreSQL refuses is its error, not this
   * method's.
   *
   * @param {"bernoulli" | "system"} method
   * @param {number} percentage
   * @param {{ repeatable?: number }} [options]
   */
  tablesample(method, percentage, { repeatable } = {}) {
    return new FromItem(native.tableSample(arg(this), method, percentage, repeatable), TRUSTED);
  }
}

/**
 * A FROM item that is not a named table, always aliased: a subquery,
 * `JSON_TABLE`, a table function, or a sampled table, which answers to its
 * table's alias or name.
 */
export class FromItem extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "FromItem", "select.as(alias), jsonTable(), tableFunction(..).alias(alias) or table.tablesample()");
    super(handle);
  }

  /** @param {string} name */
  col(name) {
    return new Expr(native.tableCol(arg(this), name), TRUSTED);
  }

  star() {
    return new Expr(native.tableStar(arg(this)), TRUSTED);
  }
}

/**
 * @param {Select} select
 * @param {string} operation
 * @param {Select} other
 */
function setOperation(select, operation, other) {
  return new Select(native.selectSetOperation(arg(select), operation, arg(other)), TRUSTED);
}

/** A `SELECT`. Every method returns a new statement. */
export class Select extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "Select", "select()");
    super(handle);
  }

  /**
   * The projection replaced: at least one expression or `expr.as(name)`.
   *
   * @param {...(Expr | import("./expressions.js").Aliased | import("./windows.js").Windowed)} items
   */
  select(...items) {
    return new Select(native.selectProject(arg(this), args(items)), TRUSTED);
  }

  /** @param {Table | FromItem} item */
  from(item) {
    return new Select(native.selectFrom(arg(this), arg(item)), TRUSTED);
  }

  /**
   * @param {Table | FromItem} item
   * @param {Expr | import("./conditions.js").Condition} on
   * @param {{ kind?: "inner" | "left" | "right" | "full", lateral?: boolean }} [options]
   */
  join(item, on, { kind = "inner", lateral = false } = {}) {
    return new Select(native.selectJoin(arg(this), kind, arg(item), arg(on), lateral === true), TRUSTED);
  }

  /** @param {Table | FromItem} item */
  crossJoin(item) {
    return new Select(native.selectJoin(arg(this), "cross", arg(item), null, false), TRUSTED);
  }

  /** @param {Expr | import("./conditions.js").Condition} predicate */
  where(predicate) {
    return new Select(native.selectWhere(arg(this), arg(predicate)), TRUSTED);
  }

  /**
   * Items added to the GROUP BY list in order: expressions, and the grouping
   * elements `GroupingElement.set`, `.empty`, `.rollup`, `.cube` and `.sets`
   * make.
   *
   * @param {...(Expr | import("./grouping.js").GroupingElement)} items
   */
  groupBy(...items) {
    return new Select(native.selectGroupBy(arg(this), args(items)), TRUSTED);
  }

  /**
   * `GROUP BY DISTINCT`: a grouping set the list reaches more than once —
   * `ROLLUP (a, b)` beside `ROLLUP (a)` reaches `(a)` three ways — grouped
   * once.
   */
  groupByDistinct() {
    return new Select(native.selectGroupByDistinct(arg(this)), TRUSTED);
  }

  /** @param {Expr | import("./conditions.js").Condition} predicate */
  having(predicate) {
    return new Select(native.selectHaving(arg(this), arg(predicate)), TRUSTED);
  }

  /** @param {...import("./expressions.js").OrderBy} orderings */
  orderBy(...orderings) {
    return new Select(native.selectOrderBy(arg(this), args(orderings)), TRUSTED);
  }

  /**
   * LIMIT, or none with `null`, which removes a tie limit too.
   *
   * @param {number | bigint | null} count
   */
  limit(count) {
    return new Select(native.selectLimit(arg(this), "limit", count), TRUSTED);
  }

  /**
   * `FETCH FIRST count ROWS WITH TIES`: the first `count` rows and every row
   * tied with the last of them on the ORDER BY, in LIMIT's place. The ties
   * need an ORDER BY and PostgreSQL refuses them beside SKIP LOCKED; both
   * are checked when the statement is inspected or run, since the order may
   * be given after the limit.
   *
   * @param {number | bigint} count
   */
  limitWithTies(count) {
    return new Select(native.selectLimit(arg(this), "withTies", count), TRUSTED);
  }

  /** @param {number | bigint | null} count */
  offset(count) {
    return new Select(native.selectLimit(arg(this), "offset", count), TRUSTED);
  }

  distinct() {
    return new Select(native.selectDistinct(arg(this)), TRUSTED);
  }

  /** @param {Select} other */
  union(other) {
    return setOperation(this, "union", other);
  }

  /** @param {Select} other */
  unionAll(other) {
    return setOperation(this, "unionAll", other);
  }

  /** @param {Select} other */
  intersect(other) {
    return setOperation(this, "intersect", other);
  }

  /** @param {Select} other */
  intersectAll(other) {
    return setOperation(this, "intersectAll", other);
  }

  /** @param {Select} other */
  except(other) {
    return setOperation(this, "except", other);
  }

  /** @param {Select} other */
  exceptAll(other) {
    return setOperation(this, "exceptAll", other);
  }

  /**
   * Lock the rows read: `FOR UPDATE` and the weaker strengths.
   *
   * @param {"update" | "noKeyUpdate" | "share" | "keyShare"} strength
   * @param {{ of?: readonly (Table | FromItem)[], wait?: "nowait" | "skipLocked" }} [options]
   */
  lock(strength, { of = [], wait } = {}) {
    return new Select(native.selectLock(arg(this), strength, args(of), wait), TRUSTED);
  }

  /** @param {With} clause */
  with(clause) {
    return new Select(native.selectWith(arg(this), arg(clause)), TRUSTED);
  }

  /**
   * An entry appended to the WINDOW clause, `"name" AS (..)`, which
   * `over(name)` reads: a `Window`, or an `ExtendedWindow` built on an entry
   * appended before it. A clause PostgreSQL refuses is refused when the
   * statement is inspected, run or read into another.
   *
   * @param {string} name
   * @param {import("./windows.js").Window | import("./windows.js").ExtendedWindow} window
   */
  window(name, window) {
    return new Select(native.selectWindow(arg(this), name, arg(window)), TRUSTED);
  }

  /**
   * This query as a FROM item, `(SELECT ..) AS "alias"`.
   *
   * @param {string} alias
   */
  as(alias) {
    return new FromItem(native.fromSubquery(arg(this), alias), TRUSTED);
  }
}

/**
 * `SELECT items`, or `SELECT *` when there are none.
 *
 * @param {...(Expr | import("./expressions.js").Aliased | import("./windows.js").Windowed)} items
 */
export function select(...items) {
  return new Select(native.selectNew(args(items)), TRUSTED);
}

/**
 * @typedef {{ columns?: readonly string[], materialized?: boolean }} CteOptions
 */

/** @type {(handle: unknown) => With} */
let withOf;

/** A WITH clause: common table expressions a statement reads by name. */
export class With extends Builder {
  /**
   * @param {string} name
   * @param {Select | import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | import("./merge.js").Merge} body
   * @param {CteOptions} [options]
   * @param {symbol} [token]
   */
  constructor(name, body, options = {}, token) {
    if (token === TRUSTED) {
      super(name);
      return;
    }
    const { columns, materialized } = options;
    super(native.withNew(name, arg(body), columns, materialized));
  }

  static {
    withOf = (handle) => new With(/** @type {any} */ (handle), /** @type {any} */ (null), {}, TRUSTED);
  }

  /**
   * One more common table expression, which may read the ones before it.
   *
   * @param {string} name
   * @param {Select | import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | import("./merge.js").Merge} body
   * @param {CteOptions} [options]
   */
  cte(name, body, { columns, materialized } = {}) {
    return withOf(native.withCte(arg(this), name, arg(body), columns, materialized));
  }

  /**
   * `WITH RECURSIVE`: one common table expression whose body may read itself,
   * with SEARCH and CYCLE when the options give them.
   *
   * @param {string} name
   * @param {Select} body
   * @param {CteOptions & {
   *   search?: { order: "breadth" | "depth", by: Expr, set: string },
   *   cycle?: { by: Expr, set: string, using: string },
   * }} [options]
   */
  static recursive(name, body, { columns, materialized, search, cycle } = {}) {
    const handle = native.withRecursive(
      name,
      arg(body),
      columns,
      materialized,
      search?.order,
      arg(search?.by),
      search?.set,
      arg(cycle?.by),
      cycle?.set,
      cycle?.using,
    );
    return withOf(handle);
  }
}
