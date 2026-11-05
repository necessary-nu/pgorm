// @ts-check

// Conditions and CASE: boolean combinations of predicates, and the two forms
// of `CASE`, which exist only once they have an arm.
// [spec:pgorm:req:bindings.expressions+3]
// [spec:pgorm:req:bindings.case]

import { arg, args, Builder, trusted, TRUSTED } from "./builder.js";
import { Expr } from "./expressions.js";
import { native } from "./operations.js";

/**
 * A boolean combination of predicates: `Condition.all` is their AND, true
 * when empty; `Condition.any` their OR, false when empty.
 */
export class Condition extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "Condition", "Condition.all() or Condition.any()");
    super(handle);
  }

  /** @param {...(Expr | Condition)} items */
  static all(...items) {
    return new Condition(native.conditionNew("all", args(items)), TRUSTED);
  }

  /** @param {...(Expr | Condition)} items */
  static any(...items) {
    return new Condition(native.conditionNew("any", args(items)), TRUSTED);
  }

  /** @param {Expr | Condition} item */
  add(item) {
    return new Condition(native.conditionAdd(arg(this), arg(item)), TRUSTED);
  }

  not() {
    return new Condition(native.conditionNot(arg(this)), TRUSTED);
  }
}

/** A searched `CASE`: `CASE WHEN condition THEN result .. END`. */
export class SearchedCase extends Expr {
  /**
   * @param {Expr | Condition} condition
   * @param {unknown} result
   */
  when(condition, result) {
    return new SearchedCase(native.caseWhen(arg(this), arg(condition), arg(result)), TRUSTED);
  }

  /** @param {unknown} result */
  else(result) {
    return new Expr(native.caseElse(arg(this), arg(result)), TRUSTED);
  }
}

/** A simple `CASE`: `CASE operand WHEN value THEN result .. END`. */
export class SimpleCase extends Expr {
  /**
   * @param {unknown} value
   * @param {unknown} result
   */
  when(value, result) {
    return new SimpleCase(native.caseOfWhen(arg(this), arg(value), arg(result)), TRUSTED);
  }

  /** @param {unknown} result */
  else(result) {
    return new Expr(native.caseElse(arg(this), arg(result)), TRUSTED);
  }
}

/** The operand of a simple `CASE`, which needs an arm to be an expression. */
export class CaseOperand extends Builder {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(token, "CASE operand", "caseOf(operand)");
    super(handle);
  }

  /**
   * @param {unknown} value
   * @param {unknown} result
   */
  when(value, result) {
    return new SimpleCase(native.caseOfWhen(arg(this), arg(value), arg(result)), TRUSTED);
  }
}

/**
 * A searched `CASE` with its first arm.
 *
 * @param {Expr | Condition} condition
 * @param {unknown} result
 */
export function caseWhen(condition, result) {
  return new SearchedCase(native.caseWhen(null, arg(condition), arg(result)), TRUSTED);
}

/**
 * A simple `CASE` over `operand`, which takes its arms from `when`.
 *
 * @param {unknown} operand
 */
export function caseOf(operand) {
  return new CaseOperand(native.caseOf(arg(operand)), TRUSTED);
}
