// @ts-check

// A pipeline's rows staged for one shape of read, as Rust's `select_sources`,
// `into_model` and `into_tuple` stage them: declared models' records through
// pgorm's `select_described`, one model's records or a function's results,
// and each row's values by position.
// [spec:pgorm:req:bindings.pipeline-models]

import { arg, trusted, TRUSTED } from "./builder.js";
import { Queryable, runStatement } from "./connections.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { readsText } from "./model-columns.js";
import { decodeAll, decodeOne, decodeOptional } from "./model-decode.js";
import { isModel, shapeOf } from "./model-shape.js";
import { native } from "./operations.js";
import { TypeName } from "./values.js";

/**
 * What a staged read runs on.
 *
 * @param {unknown} db
 */
function runner(db) {
  if (!(db instanceof Queryable)) throw new TypeError("a pipeline runs on a Pool, a Connection or a Transaction");
  return db;
}

/**
 * The rows of `pipeline` read as a declared model's records, each field from
 * the result column of its SQL name, or as what `target` makes of each row.
 *
 * @param {import("./pipeline.js").Pipeline} pipeline
 * @param {unknown} target
 */
export function modelRead(pipeline, target) {
  if (isModel(target)) {
    const shape = shapeOf(target);
    const sources = [{ fields: shape.fields, names: shape.fields.map((field) => field.sql), optional: false }];
    return new PipelineSelector(pipeline, same, decoded(sources, false), TRUSTED);
  }
  if (typeof target !== "function") throw new TypeError("intoModel takes a model or a function of a row");
  const fn = /** @type {(row: unknown) => unknown} */ (target);
  return new PipelineSelector(pipeline, same, async (db, statement, terminal, options) => {
    const runs = runner(db);
    if (terminal === "all") return (await runs.query(statement, options)).map((row) => fn(row));
    const row = terminal === "one" ? await runs.one(statement, options) : await runs.optional(statement, options);
    return row === null ? null : fn(row);
  }, TRUSTED);
}

/**
 * The rows of `pipeline` read by position, each an array of its values in
 * column order.
 *
 * @param {import("./pipeline.js").Pipeline} pipeline
 */
export function tupleRead(pipeline) {
  return new PipelineSelector(pipeline, same, async (db, statement, terminal, options) => {
    const [, rows] = await runStatement(runner(db), terminal, statement, options);
    return terminal === "all" ? rows : rows[0] ?? null;
  }, TRUSTED);
}

/** @type {(pipeline: import("./pipeline.js").Pipeline) => import("./pipeline.js").Pipeline} */
const same = (pipeline) => pipeline;

/**
 * A read decoding each row as `sources` say, a tuple of their records always
 * when `tupled`.
 *
 * @param {readonly import("./model-decode.js").Source[]} sources
 * @param {boolean} tupled
 * @returns {Read}
 */
function decoded(sources, tupled) {
  /** @param {unknown} item */
  const shaped = (item) => (tupled && sources.length === 1 ? [item] : item);
  return async (db, statement, terminal, options) => {
    if (terminal === "all") return (await decodeAll(db, statement, sources, options)).map(shaped);
    if (terminal === "one") return shaped(await decodeOne(db, statement, sources, options));
    const row = await decodeOptional(db, statement, sources, options);
    return row === null ? null : shaped(row);
  };
}

/**
 * Rust's `select_sources` over declared models: pgorm's `select_described`
 * projects each model's fields under its source's prefix, each read as the
 * model's own statements read it.
 * [spec:pgorm:req:bindings.pipeline-models]
 *
 * @param {import("./pipeline.js").Pipeline} pipeline
 * @param {readonly unknown[]} models
 * @param {(handle: unknown) => import("./pipeline.js").Pipeline} wrap the pipeline a native handle is
 */
export function selectModels(pipeline, models, wrap) {
  if (models.length === 0) throw new ConstructionError("selectSources lists at least one source");
  if (!models.every(isModel)) {
    throw new TypeError("selectSources takes a registered SourceSelection, or one or more models");
  }
  const shapes = models.map(shapeOf);
  const sources = shapes.map((shape, index) => ({
    fields: shape.fields,
    names: shape.fields.map((field) => native.modelResultName(`s${index}_`, field.sql)),
    optional: true,
  }));
  const described = shapes.map((shape) => [
    shape.qualifier,
    shape.fields.map((field) => [field.sql, readsText(field.column) ? new TypeName("text") : null, field.column.array]),
  ]);
  return new PipelineSelector(
    pipeline,
    (selected) => wrap(native.pipelineSelectDescribed(arg(selected), described)),
    decoded(sources, true),
    TRUSTED,
  );
}

/**
 * How a staged read runs its statement for one terminal.
 *
 * @typedef {(db: unknown, statement: import("./pipeline.js").Pipeline, terminal: "all" | "one" | "optional", options: unknown) => Promise<any>} Read
 */

/**
 * A pipeline's rows staged for one shape of read: `all`, `one` and
 * `optional` run it as the pipeline's own terminals do, `one` and
 * `optional` taking one row before the read's projection, and
 * `inspect(terminal)` gives the SQL and values each sends.
 * [spec:pgorm:req:bindings.pipeline-models]
 */
export class PipelineSelector {
  /** @type {import("./pipeline.js").Pipeline} */
  #pipeline;
  /** @type {(pipeline: import("./pipeline.js").Pipeline) => import("./pipeline.js").Pipeline} */
  #finish;
  /** @type {Read} */
  #read;

  /**
   * @param {import("./pipeline.js").Pipeline} pipeline
   * @param {(pipeline: import("./pipeline.js").Pipeline) => import("./pipeline.js").Pipeline} finish
   * @param {Read} read
   * @param {symbol} [token]
   */
  constructor(pipeline, finish, read, token) {
    trusted(token, "PipelineSelector", "a pipeline's selectSources, intoModel or intoTuple");
    this.#pipeline = pipeline;
    this.#finish = finish;
    this.#read = read;
    Object.freeze(this);
  }

  /** @param {unknown} terminal */
  #statement(terminal) {
    if (terminal !== "all" && terminal !== "one" && terminal !== "optional") {
      throw new TypeError('a pipeline read inspects its terminal "all", "one" or "optional"');
    }
    return this.#finish(terminal === "all" ? this.#pipeline : this.#pipeline.take(1));
  }

  /**
   * @param {"all" | "one" | "optional"} [terminal]
   * @returns {import("./index.d.ts").Compiled}
   */
  inspect(terminal = "all") {
    return this.#statement(terminal).inspect();
  }

  /**
   * @param {unknown} db
   * @param {unknown} [options]
   */
  async all(db, options) {
    return await this.#read(db, this.#statement("all"), "all", options);
  }

  /**
   * Exactly one row; none is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {unknown} [options]
   */
  async one(db, options) {
    const row = await this.#read(db, this.#statement("one"), "one", options);
    if (row === null || row === undefined) throw new DecodeError("expected exactly one row, received 0");
    return row;
  }

  /**
   * @param {unknown} db
   * @param {unknown} [options]
   */
  async optional(db, options) {
    return await this.#read(db, this.#statement("optional"), "optional", options);
  }
}
