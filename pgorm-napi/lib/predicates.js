// @ts-check

// The predicates over an expression: NULL tests, membership, ranges of
// values, patterns and text tests. LIKE's family also takes `any(..)` or
// `all(..)` over patterns, which carry no escape.
// [spec:pgorm:req:bindings.expressions+4]
// [spec:pgorm:req:bindings.quantifiers+3]

import { arg, args, Handle, made, trusted, TRUSTED } from "./builder.js";
import { native } from "./operations.js";
import { Operand } from "./operands.js";

/**
 * A LIKE, ILIKE or SIMILAR TO pattern and its `ESCAPE`, as pgorm-query's
 * `LikeExpr` holds them: the pattern is bound, and the escape, a backslash
 * until `escape` or `noEscape` replaces it, is written as a literal.
 */
export class LikeExpr extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "LikeExpr", "likeExpr(pattern)");
    super(handle);
  }

  /**
   * A pattern matching exactly `text`, its `%`, `_` and escape character
   * escaped.
   *
   * @param {string} text
   */
  static literal(text) {
    return new LikeExpr(native.likeExprText(text, "literal"), TRUSTED);
  }

  /**
   * `%text%` over literal text.
   *
   * @param {string} text
   */
  static containing(text) {
    return new LikeExpr(native.likeExprText(text, "containing"), TRUSTED);
  }

  /**
   * `text%` over literal text.
   *
   * @param {string} text
   */
  static startingWith(text) {
    return new LikeExpr(native.likeExprText(text, "startingWith"), TRUSTED);
  }

  /**
   * `%text` over literal text.
   *
   * @param {string} text
   */
  static endingWith(text) {
    return new LikeExpr(native.likeExprText(text, "endingWith"), TRUSTED);
  }

  /**
   * Escape with `character`, `ESCAPE 'c'`, in place of the default backslash.
   *
   * @param {string} character
   */
  escape(character) {
    return new LikeExpr(native.likeExprEscape(arg(this), character), TRUSTED);
  }

  /** Turn escaping off, `ESCAPE ''`: a backslash in the pattern is then itself. */
  noEscape() {
    return new LikeExpr(native.likeExprNoEscape(arg(this)), TRUSTED);
  }
}

/**
 * A pattern with the default backslash escape, which `escape` or `noEscape`
 * replaces.
 *
 * @param {string} pattern
 */
export function likeExpr(pattern) {
  return new LikeExpr(native.likeExprNew(pattern), TRUSTED);
}

/**
 * @param {Predicate} expr
 * @param {string} operator
 * @param {unknown} pattern
 */
function like(expr, operator, pattern) {
  return made.expr(native.exprLike(arg(expr), operator, arg(pattern)));
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

  /** @param {string | LikeExpr | import("./expressions.js").Quantified} pattern */
  like(pattern) {
    return like(this, "like", pattern);
  }

  /** @param {string | LikeExpr | import("./expressions.js").Quantified} pattern */
  notLike(pattern) {
    return like(this, "notLike", pattern);
  }

  /** @param {string | LikeExpr | import("./expressions.js").Quantified} pattern */
  ilike(pattern) {
    return like(this, "ilike", pattern);
  }

  /** @param {string | LikeExpr | import("./expressions.js").Quantified} pattern */
  notIlike(pattern) {
    return like(this, "notIlike", pattern);
  }

  /**
   * `SIMILAR TO`: whether the whole string matches a pattern in SQL's
   * regular-expression dialect, LIKE's `%` and `_` with `|`, `*`, `+`, `?`,
   * repetition, grouping and bracket classes; bound, with LIKE's escape.
   *
   * @param {string | LikeExpr} pattern
   */
  similarTo(pattern) {
    return like(this, "similarTo", pattern);
  }

  /** @param {string | LikeExpr} pattern */
  notSimilarTo(pattern) {
    return like(this, "notSimilarTo", pattern);
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
