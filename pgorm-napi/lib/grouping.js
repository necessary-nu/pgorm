// @ts-check

// Grouping elements: the items of a GROUP BY list beyond a bare expression,
// each made by the static factory named for the pgorm-query GroupingElement
// constructor that builds it. Each stands for a list of grouping sets, and a
// GROUP BY list groups by every combination of its items' sets, so two items
// can reach one set twice — which `groupByDistinct()` groups once.
// [spec:pgorm:req:bindings.select+4]

import { arg, args, Handle, trusted, TRUSTED } from "./builder.js";
import { native } from "./operations.js";

/** An item of a GROUP BY list that is not a bare expression. */
export class GroupingElement extends Handle {
  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    trusted(
      token,
      "GroupingElement",
      "GroupingElement.set(), .empty(), .rollup(), .cube() and .sets()",
    );
    super(handle);
  }

  /**
   * One grouping set, `(a, b)`; one expression renders bare, and none renders
   * `()`, as `empty()` does.
   *
   * @param {...import("./expressions.js").Expr} expressions
   */
  static set(...expressions) {
    return new GroupingElement(native.groupingNew("set", args(expressions)), TRUSTED);
  }

  /** The empty grouping set, `()`: the whole input as one group. */
  static empty() {
    return new GroupingElement(native.groupingNew("empty", []), TRUSTED);
  }

  /**
   * `ROLLUP (a, b)`: the sets `(a, b)`, `(a)` and `()`, every leading prefix —
   * the subtotals of a hierarchy. A `tuple` item is one unit of it.
   *
   * @param {...import("./expressions.js").Expr} expressions
   */
  static rollup(...expressions) {
    return new GroupingElement(native.groupingNew("rollup", args(expressions)), TRUSTED);
  }

  /**
   * `CUBE (a, b)`: every subset of the list, `()` included.
   *
   * @param {...import("./expressions.js").Expr} expressions
   */
  static cube(...expressions) {
    return new GroupingElement(native.groupingNew("cube", args(expressions)), TRUSTED);
  }

  /**
   * `GROUPING SETS (..)`: exactly the sets its items stand for, an expression
   * being the set of itself. The first item is taken before the rest, so the
   * empty list PostgreSQL refuses cannot be written.
   *
   * @param {import("./expressions.js").Expr | GroupingElement} first
   * @param {...(import("./expressions.js").Expr | GroupingElement)} rest
   */
  static sets(first, ...rest) {
    return new GroupingElement(native.groupingSets(arg(first), args(rest)), TRUSTED);
  }
}
