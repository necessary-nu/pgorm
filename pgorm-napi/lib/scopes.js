// @ts-check

// Scopes, the dialect of Rust's `Scope`, `Every` and `ScopeContext`: the
// program's own scopes a scoped table's statements are held to, and the
// contexts a statement is built under. `scope("CompanyId")` declares one,
// named as Rust names one by its type; calling it makes a value of it, a
// context holding itself; `every(UserId)` holds every value of a scope,
// lifted on purpose; and a plain object of them is a context holding each.
// What a context holds a statement to is pgorm-bindings' to decide; this
// module reads a context into the items it decides it from.
// [spec:pgorm:req:napi.scopes]

/** Marks a construction from `scope()` or `every()`. */
const MADE = Symbol("pgorm-napi scope");

/**
 * Each scope's name. A scope is a function, so that calling it makes a
 * value, given `Scope.prototype` rather than built by `new Scope`, which a
 * private field would need.
 *
 * @type {WeakMap<object, string>}
 */
const names = new WeakMap();

/**
 * A scope, made by {@link scope}: callable, `CompanyId(7n)` making a value
 * of it.
 */
export class Scope {
  /** @param {symbol} token */
  constructor(token) {
    if (token !== MADE || new.target === Scope) throw new TypeError('a scope is made by scope("CompanyId")');
  }

  /** The scope's name, which a context names it by. */
  get scopeName() {
    return /** @type {string} */ (names.get(this));
  }
}

/** One value of a scope: a context holding itself. */
export class ScopeValue {
  /**
   * @param {Scope} scope
   * @param {unknown} value
   * @param {symbol} token
   */
  constructor(scope, value, token) {
    if (token !== MADE) throw new TypeError("a scope's value is made by calling its scope: CompanyId(7n)");
    /** The scope the value is of. */
    this.scope = scope;
    this.value = value;
    Object.freeze(this);
  }
}

/** Every value of a scope, lifted on purpose: made by {@link every}. */
export class Every {
  /**
   * @param {Scope} scope
   * @param {symbol} token
   */
  constructor(scope, token) {
    if (token !== MADE) throw new TypeError("every value of a scope is every(scope)");
    /** The scope every value of which this holds. */
    this.scope = scope;
    Object.freeze(this);
  }
}

/**
 * Declare the scope `name`.
 *
 * @param {unknown} name
 */
export function scope(name) {
  if (typeof name !== "string") throw new TypeError("a scope's name is a string");
  if (!/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
    throw new TypeError("a scope's name is a type's name, such as CompanyId");
  }
  /** @param {unknown} value */
  const made = (value) => new ScopeValue(/** @type {Scope} */ (/** @type {unknown} */ (made)), value, MADE);
  Object.setPrototypeOf(made, Scope.prototype);
  names.set(made, name);
  return Object.freeze(made);
}

/**
 * Every value of `of`, lifted on purpose: a statement under it is held to
 * no value of the scope, and an insert takes the scope's column from its
 * row, which must set it.
 *
 * @param {unknown} of
 */
export function every(of) {
  if (!(of instanceof Scope)) throw new TypeError("every takes a scope");
  return new Every(of, MADE);
}

/**
 * One item of a context, refused unless it is a scope's value or an
 * `every(..)`.
 *
 * @param {unknown} value
 * @returns {[string, unknown, boolean]}
 */
function item(value) {
  if (value instanceof ScopeValue) return [value.scope.scopeName, value.value, false];
  if (value instanceof Every) return [value.scope.scopeName, null, true];
  throw new TypeError("a context's field is a scope's value or an every(..)");
}

/**
 * `cx` as the native functions read a context: what it holds of each
 * scope, and the name a refusal names it by, an object's being its fields.
 *
 * @param {unknown} cx
 * @returns {{ label: string, held: [string, unknown, boolean][] }}
 */
export function heldOf(cx) {
  if (cx instanceof ScopeValue) return { label: cx.scope.scopeName, held: [item(cx)] };
  if (cx instanceof Every) return { label: `every(${cx.scope.scopeName})`, held: [item(cx)] };
  if (typeof cx === "object" && cx !== null && Object.getPrototypeOf(cx) === Object.prototype) {
    return { label: `{ ${Object.keys(cx).join(", ")} }`, held: Object.values(cx).map(item) };
  }
  throw new TypeError("a context is a scope's value, an every(..) or an object of them");
}
