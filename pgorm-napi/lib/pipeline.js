// @ts-check

// pgorm's PRQL-shaped pipeline: relation-to-relation stages over sources,
// compiled through prqlc to PostgreSQL SQL. Each stage returns a new
// pipeline. A stage's `With` form calls its function once with a `Binder`,
// whose placeholders belong to that stage alone, as pgorm brands them.
// [spec:pgorm:req:bindings.pipeline+1]
// [spec:pgorm:req:bindings.pipeline-binder]

import { arg, args, Builder, Handle, trusted, TRUSTED } from "./builder.js";
import { Queryable } from "./connections.js";
import { isModel, shapeOf } from "./model-shape.js";
import { native } from "./operations.js";
import { PipelineExpr } from "./pipeline-expressions.js";
import { modelRead, selectModels, tupleRead } from "./pipeline-reads.js";
import { SourceSelection, selectSourcesOf } from "./entity-sources.js";

export {
  alias,
  average,
  caseWhen,
  col,
  count,
  countDistinct,
  countRows,
  first,
  lag,
  last,
  lead,
  literal,
  max,
  min,
  PipelineExpr,
  rank,
  rankDense,
  rowNumber,
  stddev,
  sum,
  thatColumn,
  thisColumn,
} from "./pipeline-expressions.js";

/**
 * @typedef {import("./select.js").Table | string | Pipeline | Source | object} SourceInput
 * @typedef {(binder: Binder) => unknown} BinderFunction
 */

/**
 * The relation `source` names, a declared model read as its table, under its
 * alias when it has one.
 * [spec:pgorm:req:bindings.pipeline-models]
 *
 * @param {unknown} source
 */
function relation(source) {
  return arg(isModel(source) ? shapeOf(source).table : source);
}

/** Mints a stage's placeholders; it binds only while its `With` function runs. */
export class Binder {
  /** @type {unknown} */
  #scope;

  /**
   * @param {unknown} scope
   * @param {symbol} [token]
   */
  constructor(scope, token) {
    trusted(token, "Binder", "a pipeline stage's With form");
    this.#scope = scope;
  }

  /**
   * One placeholder for one value, usable only in the expressions the
   * function returns to its stage.
   *
   * @param {unknown} value
   */
  bind(value) {
    return new PipelineExpr(native.pipelineBind(this.#scope, arg(value)), TRUSTED);
  }
}

/**
 * Call a `With` function with a binder whose scope closes as it returns or
 * throws, and hand its result to `apply` with the scope: one expression, or a
 * list of them for a list-taking stage.
 *
 * @template T
 * @param {BinderFunction} fn
 * @param {boolean} single
 * @param {(result: unknown, scope: unknown) => T} apply
 * @returns {T}
 */
function scoped(fn, single, apply) {
  if (typeof fn !== "function") throw new TypeError("a With stage takes a function of its binder");
  const scope = native.pipelineScopeOpen();
  /** @type {unknown} */
  let result;
  try {
    result = fn(new Binder(scope, TRUSTED));
  } finally {
    native.pipelineScopeClose(scope);
  }
  if (result !== null && typeof result === "object" && typeof Object(result).then === "function") {
    throw new TypeError("a binder's function is synchronous: it returns expressions, not a promise");
  }
  if (single) return apply(arg(result), scope);
  if (result instanceof PipelineExpr) return apply([arg(result)], scope);
  if (!Array.isArray(result)) {
    throw new TypeError("a list stage's function returns an expression or an array of them");
  }
  return apply(args(result), scope);
}

/** @type {(handle: unknown) => Pipeline} */
let pipelineOf;

/** A pipeline grouped and not yet aggregated, which is no relation. */
export class Grouped extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "Grouped", "pipeline.group(..)");
    super(handle);
  }

  /** @param {...unknown} aggregates */
  aggregate(...aggregates) {
    return pipelineOf(native.pipelineAggregate(arg(this), args(aggregates)));
  }

  /** @param {BinderFunction} fn */
  aggregateWith(fn) {
    return scoped(fn, false, (list, scope) => pipelineOf(native.pipelineAggregate(arg(this), list, scope)));
  }
}

/** A relation read under a name of its own. */
export class Source extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "Source", "pipeline.source(relation)");
    super(handle);
  }

  /** @param {string} name */
  named(name) {
    return new Source(native.pipelineNamed(arg(this), name), TRUSTED);
  }
}

/** A window: its partition, ordering and frame. */
export class Over extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "Over", "pipeline.over()");
    super(handle);
  }

  /** @param {...unknown} keys */
  by(...keys) {
    return new Over(native.pipelineOverKeys(arg(this), "by", args(keys)), TRUSTED);
  }

  /** @param {...unknown} keys */
  sortBy(...keys) {
    return new Over(native.pipelineOverKeys(arg(this), "sortBy", args(keys)), TRUSTED);
  }

  /**
   * @param {number | bigint | null} start
   * @param {number | bigint | null} end
   */
  rows(start, end) {
    return new Over(native.pipelineOverFrame(arg(this), "rows", start, end), TRUSTED);
  }

  /**
   * @param {number | bigint | null} start
   * @param {number | bigint | null} end
   */
  range(start, end) {
    return new Over(native.pipelineOverFrame(arg(this), "range", start, end), TRUSTED);
  }
}

/**
 * @param {Pipeline} pipeline
 * @param {"derive" | "select" | "sort"} stage
 * @param {readonly unknown[]} list
 * @param {unknown} [scope]
 */
function listed(pipeline, stage, list, scope) {
  return pipelineOf(native.pipelineList(arg(pipeline), stage, list, scope));
}

/**
 * What a pipeline's own terminal runs on.
 *
 * @param {unknown} db
 */
function runner(db) {
  if (!(db instanceof Queryable)) throw new TypeError("a pipeline runs on a Pool, a Connection or a Transaction");
  return db;
}

/**
 * A pipeline: a relation built stage by stage, run by its own terminals, as
 * Rust's `Pipeline` is, or as any statement is.
 */
export class Pipeline extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "Pipeline", "pipeline.from(source)");
    super(handle);
  }

  static {
    pipelineOf = (handle) => new Pipeline(handle, TRUSTED);
  }

  /** @param {unknown} predicate */
  filter(predicate) {
    return pipelineOf(native.pipelineFilter(arg(this), arg(predicate)));
  }

  /** @param {BinderFunction} fn */
  filterWith(fn) {
    return scoped(fn, true, (predicate, scope) => pipelineOf(native.pipelineFilter(arg(this), predicate, scope)));
  }

  /** @param {...unknown} columns */
  derive(...columns) {
    return listed(this, "derive", args(columns));
  }

  /** @param {BinderFunction} fn */
  deriveWith(fn) {
    return scoped(fn, false, (list, scope) => listed(this, "derive", /** @type {unknown[]} */ (list), scope));
  }

  /** @param {...unknown} columns */
  select(...columns) {
    return listed(this, "select", args(columns));
  }

  /** @param {BinderFunction} fn */
  selectWith(fn) {
    return scoped(fn, false, (list, scope) => listed(this, "select", /** @type {unknown[]} */ (list), scope));
  }

  /** @param {...unknown} keys */
  sort(...keys) {
    return listed(this, "sort", args(keys));
  }

  /** @param {BinderFunction} fn */
  sortWith(fn) {
    return scoped(fn, false, (list, scope) => listed(this, "sort", /** @type {unknown[]} */ (list), scope));
  }

  /** @param {...unknown} keys */
  group(...keys) {
    return new Grouped(native.pipelineGroup(arg(this), args(keys)), TRUSTED);
  }

  /** @param {BinderFunction} fn */
  groupWith(fn) {
    return scoped(fn, false, (list, scope) => new Grouped(native.pipelineGroup(arg(this), list, scope), TRUSTED));
  }

  /**
   * @param {Over} over
   * @param {...unknown} columns
   */
  window(over, ...columns) {
    return pipelineOf(native.pipelineWindow(arg(this), arg(over), args(columns)));
  }

  /**
   * @param {Over} over
   * @param {BinderFunction} fn
   */
  windowWith(over, fn) {
    return scoped(fn, false, (list, scope) => pipelineOf(native.pipelineWindow(arg(this), arg(over), list, scope)));
  }

  /** @param {number | bigint} count */
  take(count) {
    return pipelineOf(native.pipelineTake(arg(this), count));
  }

  /**
   * Rows `start` through `end`, counted from 1, both included.
   *
   * @param {number | bigint} start
   * @param {number | bigint} end
   */
  takeRange(start, end) {
    return pipelineOf(native.pipelineTake(arg(this), start, end));
  }

  /**
   * @param {SourceInput} source
   * @param {unknown} on
   * @param {{ kind?: "inner" | "left" | "right" | "full" }} [options]
   */
  join(source, on, { kind = "inner" } = {}) {
    return pipelineOf(native.pipelineJoin(arg(this), kind, relation(source), arg(on)));
  }

  /**
   * @param {SourceInput} source
   * @param {BinderFunction} fn
   * @param {{ kind?: "inner" | "left" | "right" | "full" }} [options]
   */
  joinWith(source, fn, { kind = "inner" } = {}) {
    return scoped(fn, true, (on, scope) => pipelineOf(native.pipelineJoin(arg(this), kind, relation(source), on, scope)));
  }

  /** @param {SourceInput} source */
  append(source) {
    return pipelineOf(native.pipelineSet(arg(this), "append", relation(source)));
  }

  /** @param {SourceInput} source */
  intersect(source) {
    return pipelineOf(native.pipelineSet(arg(this), "intersect", relation(source)));
  }

  /** @param {SourceInput} source */
  remove(source) {
    return pipelineOf(native.pipelineSet(arg(this), "remove", relation(source)));
  }

  /**
   * The pipeline's last stage: a registered source tuple's sources, each
   * read under the qualifier given for it or its table's name, decoded into
   * their models by the terminals of what this returns; or declared models,
   * each read under its alias or its table's name, every row a tuple of one
   * record per model, `null` where a join left that model's side absent.
   *
   * @param {...unknown} sources
   */
  selectSources(...sources) {
    const [first, options] = sources;
    if (first instanceof SourceSelection) {
      if (sources.length > 2) throw new TypeError("selectSources takes one source tuple and its options");
      return selectSourcesOf(this, first, /** @type {any} */ (options));
    }
    return selectModels(this, sources, (handle) => pipelineOf(handle));
  }

  /**
   * The rows read as a declared model's records, each field from the result
   * column of its SQL name, or as what `fn` makes of each row's record, as
   * Rust's `into_model::<M>` reads them.
   *
   * @param {unknown} target
   */
  intoModel(target) {
    return modelRead(this, target);
  }

  /**
   * The rows read by position, each an array of its values in column order,
   * as Rust's `into_tuple` reads them, so a result naming one column twice is
   * read rather than refused.
   */
  intoTuple() {
    return tupleRead(this);
  }

  distinct() {
    return pipelineOf(native.pipelineDistinct(arg(this)));
  }

  // [spec:pgorm:req:bindings.pipeline-terminals]
  /**
   * The SQL and values a terminal sends: `all`'s, or `one`'s and
   * `optional`'s, which compile with the `take 1` Rust appends.
   *
   * @override
   * @param {"all" | "one" | "optional"} [terminal]
   * @returns {import("./index.d.ts").Compiled}
   */
  inspect(terminal = "all") {
    if (terminal === "all") return super.inspect();
    if (terminal !== "one" && terminal !== "optional") {
      throw new TypeError('a pipeline inspects its terminal "all", "one" or "optional"');
    }
    return this.take(1).inspect();
  }

  /**
   * Every row, by `Pipeline::all`.
   *
   * @param {unknown} db
   * @param {unknown} [options]
   */
  async all(db, options) {
    return await runner(db).query(this, options);
  }

  /**
   * The first row, by `Pipeline::one`, which appends `take 1`; none is a
   * `DecodeError`.
   *
   * @param {unknown} db
   * @param {unknown} [options]
   */
  async one(db, options) {
    return await runner(db).one(this.take(1), options);
  }

  /**
   * The first row, or null, by `Pipeline::one_opt`, which appends `take 1`.
   *
   * @param {unknown} db
   * @param {unknown} [options]
   */
  async optional(db, options) {
    return await runner(db).optional(this.take(1), options);
  }
}

/** @param {SourceInput} source */
export function from(source) {
  return pipelineOf(native.pipelineFrom(relation(source)));
}

/** @param {SourceInput} relationInput the relation, which `named` reads under a name of its own */
export function source(relationInput) {
  return new Source(native.pipelineSource(relation(relationInput)), TRUSTED);
}

export function over() {
  return new Over(native.pipelineOver(), TRUSTED);
}

export { SelectedSources, SourceSelection, sources, sourceTuples } from "./entity-sources.js";
export { PipelineSelector } from "./pipeline-reads.js";
