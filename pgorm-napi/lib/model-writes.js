// @ts-check

// A model's writes: INSERT, UPDATE and DELETE of its fields, each value
// converted through its column's declared kind, a field left out apart from
// one set to NULL; RETURNING the fields as records; and the terminals that
// read a written row's two versions, as pgorm's `exec_returning_change(s)`
// and `exec_returning_upsert(s)` read them.
// [spec:pgorm:req:bindings.model-writes+3]

import { runStatement } from "./connections.js";
import { ConstructionError, DecodeError } from "./errors.js";
import { col } from "./expressions.js";
import { columnValue, omissible, saved } from "./model-columns.js";
import { decodeAll, decodeOne, decodeOptional } from "./model-decode.js";
import { fieldOf, projected } from "./model-shape.js";
import { native } from "./operations.js";
import { predicateOf } from "./model-clauses.js";
import { Conflict, ConflictUpdate } from "./conflicts.js";
import { deleteFrom, insert, update } from "./writes.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi model write");

/**
 * The names a RETURNING list reads a written row's versions by: always
 * renamed, as pgorm renames them, so a table called `old` or `new` cannot
 * take the keyword.
 */
const OLD = "pgorm_old";
const NEW = "pgorm_new";

/**
 * The fields `values` sets, in declaration order whatever order it names
 * them in, each with the value it becomes.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} values
 * @param {"insert" | "update"} write
 * @returns {[import("./model-shape.js").Field, unknown][]}
 */
function assignments(shape, values, write) {
  if (typeof values !== "object" || values === null || Array.isArray(values)) {
    throw new TypeError(`an ${write} takes an object of the fields it sets`);
  }
  const given = /** @type {Record<string, unknown>} */ (values);
  for (const name of Object.keys(given)) {
    const field = fieldOf(shape, name);
    if (field.column.generated === "always") {
      throw new ConstructionError(`${field.field} is generated always: the server writes it`);
    }
  }
  if (write === "insert") {
    for (const field of shape.fields) {
      const { column } = field;
      if (!omissible(column) && !Object.hasOwn(given, field.field)) {
        throw new ConstructionError(`${field.field} is required: it is not nullable and has no default`);
      }
    }
  }
  return shape.fields
    .filter((field) => Object.hasOwn(given, field.field))
    .map((field) => [field, saved(field.column, columnValue(field.column, field.field, given[field.field]))]);
}

/**
 * The RETURNING items reading `fields` as the records decode them.
 *
 * @param {readonly import("./model-shape.js").Field[]} fields
 */
function returned(fields) {
  return fields.map((field) => projected(field, col(field.sql), field.sql));
}

/**
 * The RETURNING list of both versions of every field, the old under `o_` and
 * the new under `n_`, and the sources they decode as.
 *
 * @param {import("./model-shape.js").Shape} shape
 */
function versions(shape) {
  /** @type {import("./model-decode.js").Source[]} */
  const sources = [];
  const items = [[OLD, "o_"], [NEW, "n_"]].flatMap(([row, prefix]) => {
    const names = shape.fields.map((field) => native.modelResultName(prefix, field.sql));
    sources.push({ fields: shape.fields, names, optional: row === OLD });
    return shape.fields.map((field, index) => projected(field, col(field.sql, { table: row }), /** @type {string} */ (names[index])));
  });
  return { items, sources };
}

/**
 * @param {unknown} pair
 */
function change(pair) {
  const [old, now] = /** @type {[unknown, unknown]} */ (pair);
  if (old === null) throw new DecodeError("an updated row returned no old version");
  return Object.freeze({ old, new: now });
}

/**
 * @param {unknown} pair
 */
function upserted(pair) {
  const [old, now] = /** @type {[unknown, unknown]} */ (pair);
  return Object.freeze(old === null ? { kind: "inserted", new: now } : { kind: "updated", old, new: now });
}

/**
 * The fields `names` picks out of `shape`, every field when it names none.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {readonly unknown[]} names
 */
export function picked(shape, names) {
  const fields = names.length === 0 ? shape.fields : names.map((name) => fieldOf(shape, name));
  if (new Set(fields).size !== fields.length) throw new ConstructionError("a field is selected once");
  return fields;
}

/**
 * A write with a RETURNING list of a model's fields: its terminals decode
 * each returned row into a record.
 */
export class ModelRows {
  /** @type {import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | null} */
  #statement;
  /** @type {import("./model-decode.js").Source[]} */
  #sources;

  /**
   * @param {import("./writes.js").Insert | import("./writes.js").Update | import("./writes.js").Delete | null} statement
   * @param {readonly import("./model-shape.js").Field[]} fields
   * @param {symbol} token
   */
  constructor(statement, fields, token) {
    if (token !== MADE) throw new TypeError("ModelRows come from a model write's returning()");
    this.#statement = statement;
    this.#sources = [{ fields, names: fields.map((field) => field.sql), optional: false }];
  }

  get statement() {
    return written(this.#statement);
  }

  inspect() {
    return written(this.#statement).inspect();
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    if (this.#statement === null) return [];
    return await decodeAll(db, this.#statement, this.#sources, options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    if (this.#statement === null) throw new DecodeError("expected exactly one row, and the insert wrote none");
    return await decodeOne(db, this.#statement, this.#sources, options);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    if (this.#statement === null) return null;
    return await decodeOptional(db, this.#statement, this.#sources, options);
  }
}

/**
 * @template T
 * @param {T | null} statement
 * @returns {T}
 */
function written(statement) {
  if (statement === null) throw new ConstructionError("an insert of no rows writes nothing and has no statement");
  return statement;
}

/**
 * @typedef {{
 *   shape: import("./model-shape.js").Shape,
 *   statement: import("./writes.js").Insert | null,
 *   columns: readonly string[] | null,
 *   conflict: Conflict | ConflictUpdate | null,
 * }} InsertState
 */

/** @type {(insert: Inserting) => InsertState} */
let stateOf;

/**
 * What a model's INSERT of one row and of a batch share: its rows' statement,
 * the SQL names of the fields they set — `null` for an insert of no rows, an
 * empty list for a row of defaults — its conflict clause, kept apart from the
 * statement so that an insert of no rows, which has none, keeps it for the
 * rows added after, and the methods both kinds have.
 */
class Inserting {
  /** @type {InsertState} */
  #state;

  /**
   * @param {import("./model-shape.js").Shape} shape
   * @param {import("./writes.js").Insert | null} statement
   * @param {readonly string[] | null} columns
   * @param {Conflict | ConflictUpdate | null} conflict
   * @param {symbol} token
   */
  constructor(shape, statement, columns, conflict, token) {
    if (token !== MADE) throw new TypeError(`a ${new.target.name} comes from a model's insert() or insertMany()`);
    this.#state = { shape, statement, columns, conflict };
  }

  static {
    stateOf = (insert) => insert.#state;
  }

  get statement() {
    return written(this.#state.statement);
  }

  /**
   * What a conflict does: the statement builders' `Conflict`, naming SQL
   * columns. An insert of no rows takes one too, still sending nothing.
   * [spec:pgorm:req:bindings.model-writes+3]
   *
   * @param {unknown} action
   * @returns {this}
   */
  onConflict(action) {
    if (!(action instanceof Conflict) && !(action instanceof ConflictUpdate)) {
      throw new TypeError("a conflict's action is Conflict.doNothing(), or a target's doNothing() or update(..)");
    }
    const { shape, statement, columns } = this.#state;
    const Kind = /** @type {new (...args: unknown[]) => this} */ (this.constructor);
    return new Kind(shape, statement && statement.onConflict(action), columns, action, MADE);
  }

  /**
   * The insert with one more row, which makes it a batch, as Rust's
   * `Insert::add` does; its conflict clause is kept.
   *
   * @param {unknown} values
   */
  add(values) {
    return this.addMany([values]);
  }

  /**
   * The insert with `rows` after its own, a batch, as Rust's
   * `Insert::add_many` makes one.
   *
   * @param {unknown} rows
   */
  addMany(rows) {
    if (!Array.isArray(rows)) throw new TypeError("addMany takes an array of rows");
    const { shape, statement, columns, conflict } = this.#state;
    return new ModelInsertMany(shape, ...appended(shape, statement, columns, rows, conflict), conflict, MADE);
  }

  /** @param {...string} fields */
  returning(...fields) {
    const { shape, statement } = this.#state;
    const picks = picked(shape, fields);
    return new ModelRows(statement && statement.returning(returned(picks)), picks, MADE);
  }

  /**
   * The rows written; an insert of no rows sends nothing and writes none.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    const { statement } = this.#state;
    if (statement === null) return 0;
    return await runStatement(db, "execute", statement, options);
  }
}

/**
 * The insert's statement with the RETURNING list of both versions, and the
 * sources they decode as.
 *
 * @param {Inserting} insert
 */
function upserts(insert) {
  const { shape, statement } = stateOf(insert);
  const { items, sources } = versions(shape);
  return { statement: statement && statement.returning(items, { oldAs: OLD, newAs: NEW }), sources };
}

/**
 * The SQL and values `insert` runs, or the ones its upsert terminal, `named`,
 * runs.
 *
 * @param {Inserting} insert
 * @param {unknown} terminal
 * @param {string} named
 */
function inspected(insert, terminal, named) {
  if (terminal === undefined) return written(stateOf(insert).statement).inspect();
  if (terminal !== named) throw new TypeError(`an insert inspects itself, or its "${named}"`);
  return written(upserts(insert).statement).inspect();
}

/** A model's INSERT of one row: Rust's `Insert<A, OneRow>`. */
export class ModelInsert extends Inserting {
  /**
   * The SQL and values it runs, or the ones `returningUpsert` runs.
   *
   * @param {"returningUpsert"} [terminal]
   */
  inspect(terminal) {
    return inspected(this, terminal, "returningUpsert");
  }

  /**
   * What the insert did with its row, as `exec_returning_upsert` says:
   * inserted it, updated the row that was there, or — null — wrote nothing,
   * its conflict clause holding it back.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningUpsert(db, options) {
    const { statement, sources } = upserts(this);
    const pair = await decodeOptional(db, written(statement), sources, options);
    return pair === null ? null : upserted(pair);
  }
}

/** A model's INSERT of a batch: Rust's `Insert<A, ManyRows>`. */
export class ModelInsertMany extends Inserting {
  /**
   * The SQL and values it runs, or the ones `returningUpserts` runs.
   *
   * @param {"returningUpserts"} [terminal]
   */
  inspect(terminal) {
    return inspected(this, terminal, "returningUpserts");
  }

  /**
   * What the insert did with each row it wrote, as `exec_returning_upserts`
   * says; a row its conflict clause held back is left out, and a batch of
   * none sends nothing.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningUpserts(db, options) {
    const { statement, sources } = upserts(this);
    if (statement === null) return [];
    return (await decodeAll(db, statement, sources, options)).map(upserted);
  }
}

/** A model's UPDATE. */
export class ModelUpdate {
  /** @type {import("./model-shape.js").Shape} */
  #shape;
  /** @type {import("./writes.js").Update} */
  #statement;

  /**
   * @param {import("./model-shape.js").Shape} shape
   * @param {import("./writes.js").Update} statement
   * @param {symbol} token
   */
  constructor(shape, statement, token) {
    if (token !== MADE) throw new TypeError("a ModelUpdate comes from a model's update()");
    this.#shape = shape;
    this.#statement = statement;
  }

  get statement() {
    return this.#statement;
  }

  /**
   * The SQL and values it runs, or the ones the change terminals run.
   *
   * @param {"returningChange" | "returningChanges"} [terminal]
   */
  inspect(terminal) {
    if (terminal === undefined) return this.#statement.inspect();
    if (terminal === "returningChange" || terminal === "returningChanges") return this.#versions().statement.inspect();
    throw new TypeError('an update inspects itself, or its "returningChange" or "returningChanges"');
  }

  /** @param {unknown} predicate */
  where(predicate) {
    return new ModelUpdate(this.#shape, this.#statement.where(predicateOf(predicate)), MADE);
  }

  allRows() {
    return new ModelUpdate(this.#shape, this.#statement.allRows(), MADE);
  }

  /** @param {...string} fields */
  returning(...fields) {
    const picks = picked(this.#shape, fields);
    return new ModelRows(this.#statement.returning(returned(picks)), picks, MADE);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    return await runStatement(db, "execute", this.#statement, options);
  }

  #versions() {
    const { items, sources } = versions(this.#shape);
    return { statement: this.#statement.returning(items, { oldAs: OLD, newAs: NEW }), sources };
  }

  /**
   * The one row the update wrote, before and after; any other count is a
   * `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningChange(db, options) {
    const { statement, sources } = this.#versions();
    return change(await decodeOne(db, statement, sources, options));
  }

  /**
   * Every row the update wrote, before and after.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async returningChanges(db, options) {
    const { statement, sources } = this.#versions();
    return (await decodeAll(db, statement, sources, options)).map(change);
  }
}

/** A model's DELETE. */
export class ModelDelete {
  /** @type {import("./model-shape.js").Shape} */
  #shape;
  /** @type {import("./writes.js").Delete} */
  #statement;

  /**
   * @param {import("./model-shape.js").Shape} shape
   * @param {import("./writes.js").Delete} statement
   * @param {symbol} token
   */
  constructor(shape, statement, token) {
    if (token !== MADE) throw new TypeError("a ModelDelete comes from a model's delete()");
    this.#shape = shape;
    this.#statement = statement;
  }

  get statement() {
    return this.#statement;
  }

  inspect() {
    return this.#statement.inspect();
  }

  /** @param {unknown} predicate */
  where(predicate) {
    return new ModelDelete(this.#shape, this.#statement.where(predicateOf(predicate)), MADE);
  }

  allRows() {
    return new ModelDelete(this.#shape, this.#statement.allRows(), MADE);
  }

  /** @param {...string} fields */
  returning(...fields) {
    const picks = picked(this.#shape, fields);
    return new ModelRows(this.#statement.returning(returned(picks)), picks, MADE);
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async execute(db, options) {
    return await runStatement(db, "execute", this.#statement, options);
  }
}

/**
 * @param {import("./model-shape.js").Shape} shape
 */
function unaliased(shape) {
  if (shape.alias !== null) throw new ConstructionError("an insert names its model's table, unaliased");
}

/**
 * `statement`, the INSERT of rows setting `columns`, with `rows` after them,
 * each setting the same fields; the first row of an insert of none names
 * them, and the statement it starts takes `conflict`.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {import("./writes.js").Insert | null} statement
 * @param {readonly string[] | null} columns
 * @param {readonly unknown[]} rows
 * @param {Conflict | ConflictUpdate | null} [conflict]
 * @returns {[import("./writes.js").Insert | null, readonly string[] | null]}
 */
function appended(shape, statement, columns, rows, conflict = null) {
  unaliased(shape);
  let [current, names] = [statement, columns];
  for (const row of rows) {
    const set = assignments(shape, row, "insert");
    const sql = set.map(([field]) => field.sql);
    if (current === null || names === null) {
      const started = conflict === null ? insert(shape.table) : insert(shape.table).onConflict(conflict);
      if (sql.length === 0) {
        [current, names] = [started.defaultValues(), []];
        continue;
      }
      [current, names] = [started.columns(...sql), sql];
    } else if (names.length === 0) {
      throw new ConstructionError("rows that set no field are inserted one at a time");
    } else if (sql.length !== names.length || sql.some((name, index) => name !== names?.[index])) {
      throw new ConstructionError("every row of an insert sets the same fields");
    }
    current = current.values(...set.map(([, value]) => value));
  }
  return [current, names];
}

/**
 * An INSERT of one row, Rust's `Insert::one`.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} values
 */
export function insertOne(shape, values) {
  return new ModelInsert(shape, ...appended(shape, null, null, [values]), null, MADE);
}

/**
 * An INSERT of every row of `rows`, which set the same fields, Rust's
 * `Insert::many`; no rows writes nothing.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} rows
 */
export function insertMany(shape, rows) {
  if (!Array.isArray(rows)) throw new TypeError("insertMany takes an array of rows");
  return new ModelInsertMany(shape, ...appended(shape, null, null, rows), null, MADE);
}

/**
 * An UPDATE setting the fields `values` names; it needs a `where` or
 * `allRows()` before it runs.
 *
 * @param {import("./model-shape.js").Shape} shape
 * @param {unknown} values
 */
export function updateSet(shape, values) {
  const set = assignments(shape, values, "update");
  if (set.length === 0) throw new ConstructionError("an update sets at least one field");
  let statement = update(shape.table);
  for (const [field, value] of set) statement = statement.set(field.sql, value);
  return new ModelUpdate(shape, statement, MADE);
}

/**
 * A DELETE; it needs a `where` or `allRows()` before it runs.
 *
 * @param {import("./model-shape.js").Shape} shape
 */
export function deleteRows(shape) {
  return new ModelDelete(shape, deleteFrom(shape.table), MADE);
}
