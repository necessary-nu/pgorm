// @ts-check

// Comparison, arithmetic and logic: the operators every expression has. An
// operand that is not an expression is a value, bound as a parameter; that of
// an operator PostgreSQL reads a quantifier after may also be `any(..)` or
// `all(..)`.
// [spec:pgorm:req:bindings.expressions+4]
// [spec:pgorm:req:bindings.quantifiers+4]

import { arg, Builder, made } from "./builder.js";
import { native } from "./operations.js";

/**
 * @param {Operand} expr
 * @param {string} operator
 * @param {unknown} other
 */
function binary(expr, operator, other) {
  return made.expr(native.exprBinary(arg(expr), operator, arg(other)));
}

/** What every expression is: the operators. Every method returns a new expression. */
export class Operand extends Builder {
  /** @param {unknown} other */
  eq(other) {
    return binary(this, "eq", other);
  }

  /** @param {unknown} other */
  ne(other) {
    return binary(this, "ne", other);
  }

  /** @param {unknown} other */
  lt(other) {
    return binary(this, "lt", other);
  }

  /** @param {unknown} other */
  lte(other) {
    return binary(this, "lte", other);
  }

  /** @param {unknown} other */
  gt(other) {
    return binary(this, "gt", other);
  }

  /** @param {unknown} other */
  gte(other) {
    return binary(this, "gte", other);
  }

  /** @param {unknown} other */
  add(other) {
    return binary(this, "add", other);
  }

  /** @param {unknown} other */
  sub(other) {
    return binary(this, "sub", other);
  }

  /** @param {unknown} other */
  mul(other) {
    return binary(this, "mul", other);
  }

  /** @param {unknown} other */
  div(other) {
    return binary(this, "div", other);
  }

  /** @param {unknown} other */
  mod(other) {
    return binary(this, "mod", other);
  }

  /** @param {unknown} other */
  concat(other) {
    return binary(this, "concat", other);
  }

  /**
   * `@>`: a range, multirange or array that contains the other.
   * [spec:pgorm:req:bindings.ranges]
   *
   * @param {unknown} other
   */
  contains(other) {
    return binary(this, "contains", other);
  }

  /**
   * `<@`: contained by the other.
   *
   * @param {unknown} other
   */
  containedBy(other) {
    return binary(this, "containedBy", other);
  }

  /**
   * `&&`: shares a value with the other.
   *
   * @param {unknown} other
   */
  overlaps(other) {
    return binary(this, "overlaps", other);
  }

  /**
   * `<<`: a range or multirange lying wholly left of the other.
   * [spec:pgorm:req:bindings.ranges]
   *
   * @param {unknown} other
   */
  strictlyLeftOf(other) {
    return binary(this, "strictlyLeftOf", other);
  }

  /**
   * `>>`: a range or multirange lying wholly right of the other.
   *
   * @param {unknown} other
   */
  strictlyRightOf(other) {
    return binary(this, "strictlyRightOf", other);
  }

  /**
   * `&<`: extending no further right than the other.
   *
   * @param {unknown} other
   */
  notExtendRightOf(other) {
    return binary(this, "notExtendRightOf", other);
  }

  /**
   * `&>`: extending no further left than the other.
   *
   * @param {unknown} other
   */
  notExtendLeftOf(other) {
    return binary(this, "notExtendLeftOf", other);
  }

  /**
   * `-|-`: adjacent to the other, sharing no value with nothing between.
   *
   * @param {unknown} other
   */
  adjacentTo(other) {
    return binary(this, "adjacentTo", other);
  }

  /**
   * `~`: whether the text matches a POSIX regular expression, case
   * sensitively; the pattern is bound unless it is an expression.
   *
   * @param {unknown} pattern
   */
  regex(pattern) {
    return binary(this, "regex", pattern);
  }

  /**
   * `!~`: the text does not match the regular expression.
   *
   * @param {unknown} pattern
   */
  notRegex(pattern) {
    return binary(this, "notRegex", pattern);
  }

  /**
   * `~*`: the text matches the regular expression, case ignored.
   *
   * @param {unknown} pattern
   */
  iregex(pattern) {
    return binary(this, "iregex", pattern);
  }

  /**
   * `!~*`: the text does not match the regular expression, case ignored.
   *
   * @param {unknown} pattern
   */
  notIregex(pattern) {
    return binary(this, "notIregex", pattern);
  }

  /**
   * `@@`: a `tsvector` against a `tsquery`, text against text as PostgreSQL's
   * own full-text conversion reads it, or a `jsonb` against a `jsonpath`
   * predicate. The operand is bound unless it is an expression; a `jsonpath`
   * is written cast, as a bound string is text.
   *
   * @param {unknown} other
   */
  matches(other) {
    return binary(this, "matches", other);
  }

  /**
   * `@?`: whether a `jsonpath` returns any item for this `jsonb`; the path,
   * a string, is bound as text cast to `jsonpath`, or `any(..)` or `all(..)`
   * ranges over `jsonpath` values.
   *
   * @param {string | import("./expressions.js").Quantified} path
   */
  pathExists(path) {
    return made.expr(native.exprPathExists(arg(this), arg(path)));
  }

  /**
   * `->`: the field `key` names, or the array element an integer index does,
   * as `jsonb`; the key is bound unless it is an expression.
   * [spec:pgorm:req:bindings.json-operators]
   *
   * @param {unknown} key
   */
  getJsonField(key) {
    return made.expr(native.exprJson(arg(this), "getJsonField", arg(key)));
  }

  /**
   * `->>`: the field or element as `text`.
   *
   * @param {unknown} key
   */
  castJsonField(key) {
    return made.expr(native.exprJson(arg(this), "castJsonField", arg(key)));
  }

  /**
   * `#>`: the value at `path`, keys bound as one `text[]`.
   *
   * @param {readonly string[]} path
   */
  getJsonPath(path) {
    return made.expr(native.exprJson(arg(this), "getJsonPath", path));
  }

  /**
   * `#>>`: the value at `path` as `text`.
   *
   * @param {readonly string[]} path
   */
  castJsonPath(path) {
    return made.expr(native.exprJson(arg(this), "castJsonPath", path));
  }

  /**
   * `?`: whether `key` is a top-level key, or a string element.
   *
   * @param {string} key
   */
  hasJsonKey(key) {
    return made.expr(native.exprJson(arg(this), "hasJsonKey", key));
  }

  /**
   * `?|`: whether any of `keys` is a top-level key or a string element.
   *
   * @param {readonly string[]} keys
   */
  hasAnyJsonKeys(keys) {
    return made.expr(native.exprJson(arg(this), "hasAnyJsonKeys", keys));
  }

  /**
   * `?&`: whether every one of `keys` is.
   *
   * @param {readonly string[]} keys
   */
  hasAllJsonKeys(keys) {
    return made.expr(native.exprJson(arg(this), "hasAllJsonKeys", keys));
  }

  /** @param {unknown} other */
  isDistinctFrom(other) {
    return binary(this, "isDistinctFrom", other);
  }

  /** @param {unknown} other */
  isNotDistinctFrom(other) {
    return binary(this, "isNotDistinctFrom", other);
  }

  /** @param {import("./expressions.js").Expr} other */
  and(other) {
    return made.expr(native.exprLogic(arg(this), "and", arg(other)));
  }

  /** @param {import("./expressions.js").Expr} other */
  or(other) {
    return made.expr(native.exprLogic(arg(this), "or", arg(other)));
  }

  not() {
    return made.expr(native.exprUnary(arg(this), "not"));
  }
}
