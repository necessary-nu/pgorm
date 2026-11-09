// @ts-check

// Registered source tuples: one to six entity types fixed in Rust, which a
// pipeline's last stage projects — `pipeline.selectSources(sources(name))` —
// and whose rows Rust decodes into each entity's model through its absence
// witness, every position optional, as pgorm's `select_sources` does.
// [spec:pgorm:req:bindings.pipeline-sources+3]

import { arg } from "./builder.js";
import { runJob } from "./runs.js";
import { recordOf } from "./entities.js";
import { native } from "./operations.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi source tuple");

/** The names of the source tuples this module registers. */
export function sourceTuples() {
  return /** @type {string[]} */ (native.sourcesNames(native.registry));
}

/**
 * The source tuple this module registers as `name`.
 *
 * @param {string} name
 */
export function sources(name) {
  if (typeof name !== "string") throw new TypeError("a source tuple's name is a string");
  return new SourceSelection(native.sourcesGet(native.registry, name), name, MADE);
}

/** @type {(selection: SourceSelection) => unknown} */
let selectionHandle;

/** A registered source tuple, which a pipeline selects as its last stage. */
export class SourceSelection {
  /** @type {unknown} */
  #native;
  /** @type {string} */
  #name;

  /**
   * @param {unknown} handle
   * @param {string} name
   * @param {symbol} token
   */
  constructor(handle, name, token) {
    if (token !== MADE) throw new TypeError("a SourceSelection comes from pipeline.sources(name)");
    this.#native = handle;
    this.#name = name;
    Object.freeze(this);
  }

  static {
    selectionHandle = (selection) => selection.#native;
  }

  get name() {
    return this.#name;
  }

  /** The tuple as data: its Rust shape and each source's entity. */
  describe() {
    return JSON.parse(native.sourcesDescribe(this.#native));
  }
}

/**
 * The rows of a selection job: each a tuple of the sources' records or null.
 *
 * @param {readonly string[]} entities
 * @param {[string[][], ([unknown[], unknown] | null)[][]]} outcome
 */
function rowsOf(entities, [columns, rows]) {
  return rows.map((row) =>
    row.map((pair, index) =>
      pair === null ? null : recordOf(/** @type {string} */ (entities[index]), /** @type {string[]} */ (columns[index]), pair)
    )
  );
}

/**
 * A pipeline whose last stage selects a registered tuple's sources: only its
 * terminals remain, each row a tuple of the sources' records, a source a row
 * does not carry null.
 */
export class SelectedSources {
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
    if (token !== MADE) throw new TypeError("SelectedSources come from a pipeline's selectSources()");
    this.#native = handle;
    this.#entities = entities;
  }

  /**
   * The SQL and values a terminal sends, `one` and `optional` taking one row; a
   * pipeline reshaped before the selection is a `ConstructionError`.
   *
   * @param {"all" | "one" | "optional"} [terminal]
   */
  inspect(terminal = "all") {
    const [sql, values] = native.selectedInspect(this.#native, terminal);
    return Object.freeze({ sql, values: Object.freeze(values) });
  }

  /**
   * @param {"all" | "one" | "optional"} terminal
   * @param {unknown} db
   * @param {unknown} options
   */
  async #read(terminal, db, options) {
    return rowsOf(this.#entities, await runJob(db, native.selectedJob(this.#native, terminal), options));
  }

  /**
   * Every row, by `SelectedSources::all`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return await this.#read("all", db, options);
  }

  /**
   * Exactly one row, by `SelectedSources::one`; none is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return (await this.#read("one", db, options))[0];
  }

  /**
   * The first row, or null, by `SelectedSources::one_opt`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    return (await this.#read("optional", db, options))[0] ?? null;
  }
}

/**
 * `pipeline` with `selection`'s sources as its last stage, each read under
 * the qualifier given for it, or its table's name.
 *
 * @param {unknown} pipeline
 * @param {unknown} selection
 * @param {{ qualifiers?: readonly string[] }} [options]
 */
export function selectSourcesOf(pipeline, selection, { qualifiers } = {}) {
  if (!(selection instanceof SourceSelection)) throw new TypeError("selectSources takes pipeline.sources(name)");
  if (qualifiers !== undefined && !Array.isArray(qualifiers)) throw new TypeError("qualifiers is an array of names");
  const entities = Object.freeze(selection.describe().entities);
  const handle = native.sourcesSelect(selectionHandle(selection), arg(pipeline), qualifiers ?? null);
  return new SelectedSources(handle, entities, MADE);
}
