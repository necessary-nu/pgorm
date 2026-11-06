// @ts-check

// A model query's projections: items of its columns, its joined tables'
// columns and expressions, each decoded as its declaration says, and the
// partial models that declare a projection once.
// [spec:pgorm:req:bindings.model-projections]

import { ConstructionError } from "./errors.js";
import { Expr } from "./expressions.js";
import { Clauses, countOf, keyedCursor, pagesOf, recipeOf, selectOf, sourcesOf } from "./model-clauses.js";
import { Column, ModelColumn, selected } from "./model-columns.js";
import { decodeAll, decodeCount, decodeOne, decodeOptional, decodeStream } from "./model-decode.js";
import { sameTable, shapeOf } from "./model-shape.js";
import { isIdentifier } from "./values.js";

/** Marks a construction from within the module. */
const MADE = Symbol("pgorm-napi projection");

/** @typedef {import("./model-clauses.js").Item} Item */
/** @typedef {import("./model-clauses.js").Recipe} Recipe */

/**
 * @param {unknown} name
 * @returns {string}
 */
function itemName(name) {
  if (typeof name !== "string") throw new TypeError("an item's name is a string");
  if (!isIdentifier(name)) throw new ConstructionError("an identifier is 1–63 UTF-8 bytes without NUL");
  return name;
}

/**
 * `expr` read under the result column `result` and the key `key`, decoded
 * as `declaration` says: read as the declaration's casts read it, as a
 * model's field is, and aliased where `aliased` says so or the read casts it.
 *
 * @param {Expr} expr
 * @param {string} key
 * @param {string} result
 * @param {Column} declaration
 * @param {boolean} aliased
 * @returns {Item}
 */
function itemOf(expr, key, result, declaration, aliased) {
  const read = selected(declaration, expr);
  /** @type {Expr | import("./expressions.js").Aliased} */
  const projected = aliased || read !== expr ? read.as(result) : expr;
  return { field: { field: key, sql: result, column: declaration }, projected };
}

/**
 * A model column as an item: under its field's key and its column's own
 * name by default, or under `name`, aliased always when `always` says so,
 * as Rust's `column_as` aliases, and otherwise where the name is not the
 * column's.
 *
 * @param {unknown} column
 * @param {string | null} name
 * @param {boolean} always
 * @returns {Item}
 */
function columnItem(column, name, always) {
  if (!(column instanceof ModelColumn)) {
    throw new TypeError("a projection's column is a model's column, model.col(field)");
  }
  const sql = column.column.name ?? column.field;
  const key = name ?? column.field;
  const result = name ?? sql;
  return itemOf(column, key, result, column.column, always || result !== sql);
}

/**
 * `declaration` as an expression's: a column declaration that says only how
 * the value decodes, its kind, nullability, array and values.
 *
 * @param {unknown} declaration
 * @returns {Column}
 */
function expressionDeclaration(declaration) {
  if (!(declaration instanceof Column)) {
    throw new TypeError("an expression's declaration is a column(kind, options)");
  }
  if (declaration.name !== null || declaration.primaryKey || declaration.default || declaration.generated !== null) {
    throw new ConstructionError("an expression's declaration says how it decodes: its kind, nullable, array and values");
  }
  return declaration;
}

/**
 * A model query's projection: items the program chooses, read once
 * `intoTuple`, `intoValues` or `intoModel` says what each row becomes. Every
 * method returns a new projection.
 */
export class ModelProjection extends Clauses {
  /**
   * @param {Recipe} recipe
   * @param {symbol} token
   */
  constructor(recipe, token) {
    if (token !== MADE) throw new TypeError("a ModelProjection comes from a model query's selectOnly()");
    super(recipe, (changed) => new ModelProjection(changed, MADE));
  }

  /** The SELECT it runs. */
  get statement() {
    return selectOf(recipeOf(this));
  }

  inspect() {
    return this.statement.inspect();
  }

  /** @param {Item[]} added */
  #added(added) {
    const recipe = recipeOf(this);
    const items = [.../** @type {readonly Item[]} */ (recipe.items), ...added];
    const keys = new Set();
    const results = new Set();
    for (const { field } of items) {
      if (keys.has(field.field)) throw new ConstructionError(`two items of the projection are named ${JSON.stringify(field.field)}`);
      if (results.has(field.sql)) throw new ConstructionError(`two items of the projection read the column ${JSON.stringify(field.sql)}`);
      keys.add(field.field);
      results.add(field.sql);
    }
    return new ModelProjection({ ...recipe, items }, MADE);
  }

  /**
   * A model's column, the queried one's or a joined table's, read under its
   * field's name and decoded as its field declares.
   *
   * @param {unknown} column
   */
  column(column) {
    return this.#added([columnItem(column, null, false)]);
  }

  /** @param {...unknown} columns */
  columns(...columns) {
    return this.#added(columns.map((column) => columnItem(column, null, false)));
  }

  /**
   * A model's column under `name`, decoded as its field declares.
   *
   * @param {unknown} column
   * @param {unknown} name
   */
  columnAs(column, name) {
    return this.#added([columnItem(column, itemName(name), true)]);
  }

  /**
   * An expression under `name`, decoded as `declaration` says: an
   * aggregate's or a function's result, whose type the program states as
   * Rust states it in the type `into_tuple` decodes into.
   *
   * @param {unknown} expr
   * @param {unknown} name
   * @param {unknown} declaration
   */
  exprAs(expr, name, declaration) {
    if (!(expr instanceof Expr)) throw new TypeError("exprAs projects an expression");
    const key = itemName(name);
    return this.#added([itemOf(expr, key, key, expressionDeclaration(declaration), true)]);
  }

  /** @param {(record: any) => unknown} read */
  #selector(read) {
    const recipe = recipeOf(this);
    if (/** @type {readonly Item[]} */ (recipe.items).length === 0) {
      throw new ConstructionError("a projection selects at least one item");
    }
    return new ModelSelector(recipe, read, MADE);
  }

  /** Each row as an array of the items' values, in projection order. */
  intoTuple() {
    const keys = /** @type {readonly Item[]} */ (recipeOf(this).items).map((item) => item.field.field);
    return this.#selector((record) => keys.map((key) => record[key]));
  }

  /**
   * Each row as an array of the values of the items `names` names, in that
   * order: Rust's `into_values`, which reads by a `Column` enum's names.
   *
   * @param {...unknown} names
   */
  intoValues(...names) {
    const known = /** @type {readonly Item[]} */ (recipeOf(this).items).map((item) => item.field.field);
    if (names.length === 0) throw new ConstructionError("intoValues names at least one item");
    for (const [index, name] of names.entries()) {
      if (typeof name !== "string") throw new TypeError("intoValues names items by string");
      if (!known.includes(name)) throw new ConstructionError(`the projection has no item ${JSON.stringify(name)}`);
      if (names.indexOf(name) !== index) throw new ConstructionError(`intoValues names ${JSON.stringify(name)} twice`);
    }
    const keys = /** @type {string[]} */ (names);
    return this.#selector((record) => keys.map((key) => record[key]));
  }

  /**
   * Each row as an object of the items' values keyed by name, as Rust's
   * `into_model` reads a `FromQueryResult` struct by name, or what `make`
   * makes of that object.
   *
   * @param {unknown} [make]
   */
  intoModel(make) {
    if (make !== undefined && typeof make !== "function") throw new TypeError("intoModel takes a function of the row");
    return this.#selector(make === undefined ? (record) => record : /** @type {(record: any) => unknown} */ (make));
  }
}

/**
 * A projection whose rows are read: Rust's `Selector`. Its terminals decode
 * each row's items as declared, a column of another kind, a NULL where an
 * item is not nullable, or a label it does not list being a `DecodeError`,
 * and make each row what the projection chose.
 */
export class ModelSelector {
  /** @type {Recipe} */
  #recipe;
  /** @type {(record: any) => unknown} */
  #read;

  /**
   * @param {Recipe} recipe
   * @param {(record: any) => unknown} read
   * @param {symbol} token
   */
  constructor(recipe, read, token) {
    if (token !== MADE) throw new TypeError("a ModelSelector comes from a projection's intoTuple(), intoValues() or intoModel()");
    this.#recipe = recipe;
    this.#read = read;
    Object.freeze(this);
  }

  /** The SELECT it runs. */
  get statement() {
    return selectOf(this.#recipe);
  }

  /**
   * The SQL and values the rows are read with, or with `"count"` the ones
   * `count` runs.
   *
   * @param {"count"} [terminal]
   */
  inspect(terminal) {
    if (terminal === undefined) return this.statement.inspect();
    if (terminal === "count") return countOf(this.statement).inspect();
    throw new TypeError('a model selector inspects itself, or its "count"');
  }

  /**
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async all(db, options) {
    return (await decodeAll(db, this.statement, sourcesOf(this.#recipe), options)).map(this.#read);
  }

  /**
   * Exactly one row; any other count is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async one(db, options) {
    return this.#read(await decodeOne(db, this.statement, sourcesOf(this.#recipe), options));
  }

  /**
   * At most one row, or null; more is a `DecodeError`.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async optional(db, options) {
    const record = await decodeOptional(db, this.statement, sourcesOf(this.#recipe), options);
    return record === null ? null : this.#read(record);
  }

  /**
   * The rows, one per pull, over a pool's or a connection's connection.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  stream(db, options) {
    return decodeStream(db, this.statement, sourcesOf(this.#recipe), options, this.#read);
  }

  /**
   * How many rows the projection reads, its limit and offset aside.
   *
   * @param {unknown} db
   * @param {{ signal?: AbortSignal }} [options]
   */
  async count(db, options) {
    return await decodeCount(db, countOf(this.statement), options);
  }

  /**
   * A keyset cursor over the rows ordered by `fields` of the queried model,
   * each row read as the selector reads it.
   *
   * @param {...string} fields
   */
  cursor(...fields) {
    return keyedCursor(this.#recipe, fields, sourcesOf(this.#recipe), this.#read);
  }

  /**
   * Pages of `pageSize` rows.
   *
   * @param {number} pageSize
   */
  paginate(pageSize) {
    return pagesOf(this.#recipe, sourcesOf(this.#recipe), this.#read, pageSize);
  }
}

/**
 * The projection of `recipe`, whose items it holds.
 *
 * @param {Recipe} recipe
 */
export function projectionOf(recipe) {
  return new ModelProjection(recipe, MADE);
}

/** A partial model's field read from an expression, decoded as its declaration says: Rust's `from_expr`. */
// [spec:pgorm:req:bindings.partial-models]
export class FromExpr {
  /**
   * @param {unknown} expr
   * @param {unknown} declaration
   * @param {symbol} token
   */
  constructor(expr, declaration, token) {
    if (token !== MADE) throw new TypeError("a FromExpr is made by fromExpr(expr, column)");
    if (!(expr instanceof Expr)) throw new TypeError("fromExpr reads an expression");
    /** @readonly */
    this.expr = expr;
    /** @readonly */
    this.column = expressionDeclaration(declaration);
    Object.freeze(this);
  }
}

/**
 * A partial model's field read from `expr`, decoded as `column` declares.
 *
 * @param {unknown} expr
 * @param {unknown} column
 */
export function fromExpr(expr, column) {
  return new FromExpr(expr, column, MADE);
}

/** A partial model's field read from no column, holding its default in every row: Rust's `skip`. */
// [spec:pgorm:req:bindings.partial-models]
export class Skip {
  /**
   * @param {unknown} value
   * @param {symbol} token
   */
  constructor(value, token) {
    if (token !== MADE) throw new TypeError("a Skip is made by skip(value)");
    /** @readonly */
    this.default = value;
    Object.freeze(this);
  }
}

/**
 * A partial model's field read from no column, `value` in every row.
 *
 * @param {unknown} value
 */
export function skip(value) {
  return new Skip(value, MADE);
}

/** @type {(shape: PartialModel, recipe: Recipe) => ModelSelector} */
let selectorOf;

/**
 * A read shape over a model: each field one of the model's columns (Rust's
 * `from_col`, a field named for its column being Rust's plain field), an
 * expression (`fromExpr`) or nothing (`skip`). `intoPartialModel` selects
 * what it declares and decodes each row into an object of its fields, in
 * declaration order.
 */
// [spec:pgorm:req:bindings.partial-models]
export class PartialModel {
  /** @type {import("./model-shape.js").Shape} */
  #shape;
  /** @type {readonly [string, ModelColumn | FromExpr | Skip][]} */
  #fields;

  /**
   * @param {unknown} model
   * @param {unknown} fields
   * @param {symbol} token
   */
  constructor(model, fields, token) {
    if (token !== MADE) throw new TypeError("a PartialModel is made by partialModel(model, fields)");
    this.#shape = shapeOf(model);
    if (typeof fields !== "object" || fields === null || Array.isArray(fields)) {
      throw new TypeError("a partial model's fields are an object of names to fields");
    }
    const declared = Object.entries(fields).map(([name, field]) => {
      itemName(name);
      if (!(field instanceof ModelColumn) && !(field instanceof FromExpr) && !(field instanceof Skip)) {
        throw new TypeError(`${name} is a model column, a fromExpr() or a skip()`);
      }
      return /** @type {[string, ModelColumn | FromExpr | Skip]} */ ([name, field]);
    });
    if (!declared.some(([, field]) => !(field instanceof Skip))) {
      throw new ConstructionError("a partial model reads at least one field");
    }
    this.#fields = Object.freeze(declared);
    Object.freeze(this);
  }

  get model() {
    return this.#shape.model;
  }

  get fields() {
    return Object.freeze(Object.fromEntries(this.#fields));
  }

  static {
    selectorOf = (shape, recipe) => shape.#selector(recipe);
  }

  /**
   * `recipe`'s query read as this shape: the model's own, refused for
   * another.
   *
   * @param {Recipe} recipe
   */
  #selector(recipe) {
    if (!sameTable(recipe.shape, this.#shape)) {
      throw new ConstructionError(`the partial model reads ${this.#shape.name}, and the query ${recipe.shape.name}`);
    }
    /** @type {Item[]} */
    const items = [];
    for (const [name, field] of this.#fields) {
      if (field instanceof ModelColumn) items.push(columnItem(field, name, false));
      else if (field instanceof FromExpr) items.push(itemOf(field.expr, name, name, field.column, true));
    }
    const fields = this.#fields;
    /** @param {Record<string, unknown>} record */
    const read = (record) =>
      Object.fromEntries(fields.map(([name, field]) => [name, field instanceof Skip ? field.default : record[name]]));
    return new ModelSelector({ ...recipe, items }, read, MADE);
  }
}

/**
 * Declare a read shape over `model`.
 *
 * @param {unknown} model
 * @param {unknown} fields
 */
export function partialModel(model, fields) {
  return new PartialModel(model, fields, MADE);
}

/**
 * `recipe`'s query read as the partial model `shape`.
 *
 * @param {unknown} shape
 * @param {Recipe} recipe
 */
export function partialSelector(shape, recipe) {
  if (!(shape instanceof PartialModel)) throw new TypeError("intoPartialModel takes a partialModel()");
  return selectorOf(shape, recipe);
}
