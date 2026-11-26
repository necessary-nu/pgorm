/**
 * Scopes, the dialect of Rust's `Scope`, `Every` and `ScopeContext`: the
 * program's own scopes a scoped table's statements are held to, and the
 * contexts a statement is built under.
 *
 * @module
 */

// [spec:pgorm:req:napi.scopes]

/**
 * A scope, made by {@link scope}: the program's own, named `N` as Rust names
 * one by its type, whose values are `V`. Called, it makes a value of it.
 */
export declare class Scope<N extends string = string, V = unknown> {
  private constructor();
  /** The scope's name, which a context names it by. */
  readonly scopeName: N;
  /** The type of its values, for the types alone. */
  private readonly values?: V;
}

/** A scope as it is called: `CompanyId(7n)` is one value of it. */
export type ScopeOf<N extends string = string, V = unknown> = Scope<N, V> & ((value: V) => ScopeValue<N, V>);

/** One value of a scope, a context holding itself. */
export declare class ScopeValue<N extends string = string, V = unknown> {
  private constructor();
  readonly scope: ScopeOf<N, V>;
  readonly value: V;
}

/** Every value of a scope, lifted on purpose: made by {@link every}. */
export declare class Every<N extends string = string> {
  private constructor();
  readonly scope: ScopeOf<N, any>;
}

/**
 * Declare the scope `name`, whose values are `V`: `scope("CompanyId")`, or
 * `scope<"CompanyId", bigint>("CompanyId")` to type its values.
 */
export declare function scope<const N extends string, V = unknown>(name: N): ScopeOf<N, V>;

/**
 * Every value of `scope`: a statement under it is held to none of them, and
 * an insert takes the scope's column from its row, which must set it.
 */
export declare function every<N extends string>(scope: ScopeOf<N, any>): Every<N>;

/** What a context holds of one scope: one value, or every value. */
export type ScopeHolding = ScopeValue<any, any> | Every<any>;

/** A context: one scope's value, an `every(..)`, or an object of them. */
export type ScopeContext = ScopeHolding | { readonly [field: string]: ScopeHolding };

/** The scope a context's field holds, by name. */
type HeldName<H> = H extends ScopeValue<infer N, any> ? N : H extends Every<infer N> ? N : never;

/** The scopes a context holds, by name. */
export type HeldBy<Cx> = Cx extends ScopeHolding ? HeldName<Cx> : { [K in keyof Cx]: HeldName<Cx[K]> }[keyof Cx];

/**
 * `unknown` where the context `Cx` holds each of the scopes `S`, and
 * otherwise the scopes it misses, which a statement's context is refused
 * by.
 */
export type Holding<S extends string, Cx> = [Exclude<S, HeldBy<Cx>>] extends [never] ? unknown
  : { readonly "missing scopes": Exclude<S, HeldBy<Cx>> };
