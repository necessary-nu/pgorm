// @ts-check

// Pipeline expressions: columns, introduced names, literals, operators,
// casts, CASE and the aggregate and window functions pgorm's pipeline has.
// They are not the statement builders' expressions, and the two do not mix.
// A value an operand is given is bound by the stage that takes it; only
// `literal()` writes one into the SQL.
// [spec:pgorm:req:bindings.pipeline-expressions+3]

import { arg, args, Handle, trusted, TRUSTED } from "./builder.js";
import { registeredColumn } from "./entity-queries.js";
import { ConstructionError } from "./errors.js";
import { columnValue } from "./model-columns.js";
import { fieldOf, isModel, shapeOf } from "./model-shape.js";
import { native } from "./operations.js";

/** @type {(handle: unknown) => PipelineExpr} */
let exprOf;

/** A pipeline expression; every method returns a new one. */
export class PipelineExpr extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "pipeline expression", "pipeline.col(), pipeline.alias() and the other pipeline functions");
    super(handle);
  }

  static {
    exprOf = (handle) => new PipelineExpr(handle, TRUSTED);
  }

  /**
   * @param {string} operator
   * @param {unknown} other
   */
  #binary(operator, other) {
    return exprOf(native.pipelineBinary(arg(this), operator, arg(other)));
  }

  /** @param {unknown} other */
  eq(other) {
    return this.#binary("eq", other);
  }

  /** @param {unknown} other */
  ne(other) {
    return this.#binary("ne", other);
  }

  /** @param {unknown} other */
  gt(other) {
    return this.#binary("gt", other);
  }

  /** @param {unknown} other */
  gte(other) {
    return this.#binary("gte", other);
  }

  /** @param {unknown} other */
  lt(other) {
    return this.#binary("lt", other);
  }

  /** @param {unknown} other */
  lte(other) {
    return this.#binary("lte", other);
  }

  /** @param {unknown} other */
  and(other) {
    return this.#binary("and", other);
  }

  /** @param {unknown} other */
  or(other) {
    return this.#binary("or", other);
  }

  /** @param {unknown} other */
  coalesce(other) {
    return this.#binary("coalesce", other);
  }

  /** @param {unknown} other */
  add(other) {
    return this.#binary("add", other);
  }

  /** @param {unknown} other */
  sub(other) {
    return this.#binary("sub", other);
  }

  /** @param {unknown} other */
  mul(other) {
    return this.#binary("mul", other);
  }

  /** @param {unknown} other */
  div(other) {
    return this.#binary("div", other);
  }

  /** @param {unknown} other */
  rem(other) {
    return this.#binary("rem", other);
  }

  not() {
    return exprOf(native.pipelineUnary(arg(this), "not"));
  }

  neg() {
    return exprOf(native.pipelineUnary(arg(this), "neg"));
  }

  isNull() {
    return exprOf(native.pipelineUnary(arg(this), "isNull"));
  }

  isNotNull() {
    return exprOf(native.pipelineUnary(arg(this), "isNotNull"));
  }

  asc() {
    return exprOf(native.pipelineUnary(arg(this), "asc"));
  }

  desc() {
    return exprOf(native.pipelineUnary(arg(this), "desc"));
  }

  /** @param {readonly unknown[]} values */
  inArray(values) {
    return exprOf(native.pipelineIn(arg(this), args(values)));
  }

  /** @param {string} type */
  cast(type) {
    return exprOf(native.pipelineCast(arg(this), type));
  }

  /**
   * Compare this expression as the type `type` names, as a registered column
   * of that type compares: the column and each value compared with it cast
   * to the type, which a column whose domain is over an enum, or a `money`
   * column compared with a `Decimal`, needs where no registration types it.
   *
   * @param {string | import("./values.js").TypeName} type
   */
  comparesAs(type) {
    return exprOf(native.pipelineComparesAs(arg(this), type));
  }

  /** @param {string | PipelineExpr} name */
  as(name) {
    return exprOf(native.pipelineAs(arg(this), arg(name)));
  }
}

/**
 * A declared model's column in a pipeline: a value it is compared with, in a
 * comparison, `coalesce` or `inArray`, converts through the field's
 * declaration as the model column's own comparisons convert one, and is
 * bound cast as pgorm's `col_typed` casts a value compared with a column of
 * that kind. Only the column itself converts: an operator's result is a
 * plain expression.
 */
// [spec:pgorm:req:bindings.pipeline-models]
class ModelPipelineColumn extends PipelineExpr {
  /** @type {import("./model-shape.js").Field} */
  #field;

  /**
   * @param {unknown} handle
   * @param {import("./model-shape.js").Field} field
   */
  constructor(handle, field) {
    super(handle, TRUSTED);
    this.#field = field;
  }

  /** @param {unknown} value */
  #operand(value) {
    if (value instanceof PipelineExpr) return value;
    if (value === null) {
      throw new ConstructionError(`${this.#field.field} compared with null is never true: test it with isNull()`);
    }
    const converted = columnValue(this.#field.column, this.#field.field, value);
    return exprOf(native.pipelineDeclaredValue(arg(converted)));
  }

  /** @override @param {unknown} other */
  eq(other) {
    return super.eq(this.#operand(other));
  }

  /** @override @param {unknown} other */
  ne(other) {
    return super.ne(this.#operand(other));
  }

  /** @override @param {unknown} other */
  gt(other) {
    return super.gt(this.#operand(other));
  }

  /** @override @param {unknown} other */
  gte(other) {
    return super.gte(this.#operand(other));
  }

  /** @override @param {unknown} other */
  lt(other) {
    return super.lt(this.#operand(other));
  }

  /** @override @param {unknown} other */
  lte(other) {
    return super.lte(this.#operand(other));
  }

  /** @override @param {unknown} other */
  coalesce(other) {
    return super.coalesce(this.#operand(other));
  }

  /** @override @param {readonly unknown[]} values */
  inArray(values) {
    if (!Array.isArray(values)) throw new TypeError("inArray takes an array");
    return super.inArray(values.map((value) => this.#operand(value)));
  }
}

/**
 * A column of the relation `table` names: prqlc has no catalog, so every
 * column is qualified. A registered entity's column (`entity.col(name)`) is
 * the entity's own, compared as the entity types it; a name only qualifies.
 * Given a declared model, `column` is its field: the column its declaration
 * names, qualified by the model's alias or table name, and compared as its
 * declared kind.
 *
 * @param {string | PipelineExpr | object} table
 * @param {string | PipelineExpr | import("./entity-queries.js").EntityColumn} column
 */
export function col(table, column) {
  if (isModel(table)) {
    const shape = shapeOf(table);
    const field = fieldOf(shape, column);
    const { kind, array, values } = field.column;
    return new ModelPipelineColumn(
      native.pipelineModelCol(shape.qualifier, field.sql, kind, array, values),
      field,
    );
  }
  const registered = registeredColumn(column);
  if (registered) return exprOf(native.pipelineRegisteredCol(arg(table), ...registered));
  return exprOf(native.pipelineCol(arg(table), arg(column)));
}

/** @param {string} name */
export function alias(name) {
  return exprOf(native.pipelineAlias(name));
}

/** @param {unknown} value */
export function literal(value) {
  return exprOf(native.pipelineLiteral(value));
}

/** @param {string | PipelineExpr} column */
export function thisColumn(column) {
  return exprOf(native.pipelineRole("this", arg(column)));
}

/** @param {string | PipelineExpr} column */
export function thatColumn(column) {
  return exprOf(native.pipelineRole("that", arg(column)));
}

/**
 * @param {readonly (readonly [unknown, unknown])[]} arms
 * @param {unknown} otherwise
 */
export function caseWhen(arms, otherwise) {
  return exprOf(native.pipelineCase(arms.map((arm) => args(arm)), arg(otherwise)));
}

/**
 * @param {string} name
 * @param {...unknown} operands
 */
function call(name, ...operands) {
  return exprOf(native.pipelineFunction(name, ...args(operands)));
}

/** @param {unknown} value */
export const sum = (value) => call("sum", value);
/** @param {unknown} value */
export const min = (value) => call("min", value);
/** @param {unknown} value */
export const max = (value) => call("max", value);
/** @param {unknown} value */
export const average = (value) => call("average", value);
/** @param {unknown} value */
export const stddev = (value) => call("stddev", value);
/** @param {unknown} value */
export const count = (value) => call("count", value);
/** @param {unknown} value */
export const countDistinct = (value) => call("countDistinct", value);
export const countRows = () => call("countRows");
export const rowNumber = () => call("rowNumber");
/** @param {unknown} value */
export const rank = (value) => call("rank", value);
/** @param {unknown} value */
export const rankDense = (value) => call("rankDense", value);
/** @param {unknown} value */
export const first = (value) => call("first", value);
/** @param {unknown} value */
export const last = (value) => call("last", value);

/**
 * @param {number} offset
 * @param {unknown} value
 */
export const lag = (offset, value) => call("lag", offset, value);

/**
 * @param {number} offset
 * @param {unknown} value
 */
export const lead = (offset, value) => call("lead", offset, value);
