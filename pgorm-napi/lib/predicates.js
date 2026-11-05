// @ts-check

// The predicates over an expression: NULL tests, membership, ranges of
// values, patterns and text tests. LIKE's family also takes `any(..)` or
// `all(..)` over patterns, with no escape.
// [spec:pgorm:req:bindings.expressions+2]
// [spec:pgorm:req:bindings.quantifiers+2]

import { arg, args, made } from "./builder.js";
import { native } from "./operations.js";
import { Operand } from "./operands.js";

/**
 * @param {Predicate} expr
 * @param {string} operator
 * @param {unknown} pattern
 * @param {{ escape?: string }} options
 */
function like(expr, operator, pattern, { escape } = {}) {
  return made.expr(native.exprLike(arg(expr), operator, arg(pattern), escape));
}

/**
 * @param {Predicate} expr
 * @param {boolean} negated
 * @param {unknown} lower
 * @param {unknown} upper
 * @param {{ symmetric?: boolean }} options
 */
function between(expr, negated, lower, upper, { symmetric = false } = {}) {
  return made.expr(native.exprBetween(arg(expr), negated, symmetric === true, arg(lower), arg(upper)));
}

/**
 * @param {Predicate} expr
 * @param {boolean} negated
 * @param {unknown} source
 */
function membership(expr, negated, source) {
  const items = Array.isArray(source) ? args(source) : arg(source);
  return made.expr(native.exprIn(arg(expr), negated, items));
}

/** An expression's predicates. Every method returns a new expression. */
export class Predicate extends Operand {
  isNull() {
    return made.expr(native.exprUnary(arg(this), "isNull"));
  }

  isNotNull() {
    return made.expr(native.exprUnary(arg(this), "isNotNull"));
  }

  /** @param {readonly unknown[] | import("./select.js").Select} source */
  isIn(source) {
    return membership(this, false, source);
  }

  /** @param {readonly unknown[] | import("./select.js").Select} source */
  isNotIn(source) {
    return membership(this, true, source);
  }

  /**
   * @param {unknown} lower
   * @param {unknown} upper
   * @param {{ symmetric?: boolean }} [options]
   */
  between(lower, upper, options) {
    return between(this, false, lower, upper, options);
  }

  /**
   * @param {unknown} lower
   * @param {unknown} upper
   * @param {{ symmetric?: boolean }} [options]
   */
  notBetween(lower, upper, options) {
    return between(this, true, lower, upper, options);
  }

  /**
   * @param {string | import("./expressions.js").Quantified} pattern
   * @param {{ escape?: string }} [options]
   */
  like(pattern, options) {
    return like(this, "like", pattern, options);
  }

  /**
   * @param {string | import("./expressions.js").Quantified} pattern
   * @param {{ escape?: string }} [options]
   */
  notLike(pattern, options) {
    return like(this, "notLike", pattern, options);
  }

  /**
   * @param {string | import("./expressions.js").Quantified} pattern
   * @param {{ escape?: string }} [options]
   */
  ilike(pattern, options) {
    return like(this, "ilike", pattern, options);
  }

  /**
   * @param {string | import("./expressions.js").Quantified} pattern
   * @param {{ escape?: string }} [options]
   */
  notIlike(pattern, options) {
    return like(this, "notIlike", pattern, options);
  }

  /**
   * `SIMILAR TO`: whether the whole string matches a pattern in SQL's
   * regular-expression dialect, LIKE's `%` and `_` with `|`, `*`, `+`, `?`,
   * repetition, grouping and bracket classes; bound, with LIKE's escape.
   *
   * @param {string} pattern
   * @param {{ escape?: string }} [options]
   */
  similarTo(pattern, options) {
    return like(this, "similarTo", pattern, options);
  }

  /**
   * @param {string} pattern
   * @param {{ escape?: string }} [options]
   */
  notSimilarTo(pattern, options) {
    return like(this, "notSimilarTo", pattern, options);
  }

  /** @param {unknown} text */
  startsWith(text) {
    return made.expr(native.exprText(arg(this), "startsWith", arg(text)));
  }

  /** @param {unknown} text */
  endsWith(text) {
    return made.expr(native.exprText(arg(this), "endsWith", arg(text)));
  }

  /** @param {unknown} text */
  containsText(text) {
    return made.expr(native.exprText(arg(this), "containsText", arg(text)));
  }
}
