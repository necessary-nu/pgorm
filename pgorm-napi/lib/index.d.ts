/**
 * pgorm from Node.js and Deno, through a Node-API addon.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]

import type { Select } from "./select.d.ts";
import type { Merge } from "./merge.d.ts";
import type { Delete, Insert, Update } from "./writes.d.ts";
import type { DDL } from "./schema.d.ts";
import type { Pipeline } from "./pipeline.d.ts";
import type {
  CopyColumn,
  CopyFrom,
  CopyOutStream,
  CopyRows,
  CopyRowSink,
  CopyRowStream,
  CopySource,
  CopyTo,
} from "./copy.d.ts";
import type { Listener, ListenerOptions } from "./listen.d.ts";
import type { Paginator } from "./graphs.d.ts";
import type { Model } from "./models.d.ts";

export * from "./expressions.d.ts";
export * from "./json.d.ts";
export * from "./merge.d.ts";
export * from "./schema.d.ts";
export * from "./migrations.d.ts";
export * as pipeline from "./pipeline.d.ts";
export * from "./windows.d.ts";
export * from "./select.d.ts";
export * from "./writes.d.ts";
export * from "./models.d.ts";
export * from "./graphs.d.ts";
export * from "./entities.d.ts";
export * from "./disjoint.d.ts";
export * from "./one-of.d.ts";
export * from "./copy.d.ts";
export * from "./listen.d.ts";
export * from "./metrics.d.ts";

/** The addon's version, which is the pgorm release it is built from. */
export declare const version: string;

// [spec:pgorm:req:bindings.connections+3]
/**
 * The channel pgorm-migration's runner notifies once a batch that applied a
 * migration commits: give it as {@link PoolOptions.statementCacheChannel} for
 * a pool to clear its cached statements after a migration run elsewhere.
 */
export declare const STATEMENT_CACHE_CHANNEL: string;

// [spec:pgorm:req:bindings.postgres-target]
/**
 * The PostgreSQL release this build targets: 18, unless the addon was built
 * with its `pg-19` feature, which forwards pgorm's and makes it 19. It is
 * Rust's `pgorm_query::POSTGRES_TARGET`.
 */
export declare const POSTGRES_TARGET: 18 | 19;

/** The base class of every error pgorm-napi rejects with. */
export declare class PgormError extends Error {
  /**
   * Whether the transaction this failure ended is worth running again, as
   * pgorm's `Error::is_retryable` answers: a serialization failure (`40001`)
   * or a deadlock (`40P01`). Only a `DatabaseError` can be.
   */
  get isRetryable(): boolean;
}

/** The server could not be reached, or the connection to it broke. */
export declare class ConnectionError extends PgormError {}

/**
 * An argument could not become the value pgorm sends. One the driver refused
 * as it encoded a parameter, pgorm's `BindRefusal`, carries which `$N` it was
 * and why; any other carries `null` for both.
 */
export declare class ConstructionError extends PgormError {
  constructor(message: string, details?: { parameter?: number; reason?: string | null });
  /** The one-based `$N` of the parameter the driver refused. */
  readonly parameter: number | null;
  /** The codec's refusal of the value. */
  readonly reason: string | null;
}

/**
 * A result could not be decoded into the JavaScript value asked for. One the
 * driver could not read, pgorm's `DecodeRefusal`, carries what it was reading
 * and why; any other carries `null` for both.
 */
export declare class DecodeError extends PgormError {
  constructor(message: string, details?: { reading?: string; reason?: string | null });
  /** What the driver was reading, e.g. `"error deserializing column 0"`. */
  readonly reading: string | null;
  /** The decoder's refusal of the value. */
  readonly reason: string | null;
}

/**
 * pgorm or the binding failed in a way no input should cause, such as a panic
 * in its native code. Never an expected rejection of a caller's input.
 */
export declare class InternalError extends PgormError {}

/**
 * A pool, connection, transaction or stream was used after it closed, or
 * while another operation held it: a second statement on a busy connection, a
 * parent transaction used while a savepoint of it is open.
 */
export declare class LifecycleError extends PgormError {}

/** No connection became free within the pool's `acquireTimeout`. */
export declare class TimeoutError extends PgormError {}

/** The diagnostic fields of an error PostgreSQL reported. */
export interface DatabaseErrorDetails {
  /** The five-character SQLSTATE, e.g. `"22003"` for numeric_value_out_of_range. */
  readonly sqlstate: string;
  /** The severity the server reported, e.g. `"ERROR"` or `"FATAL"`. */
  readonly severity: string;
  readonly detail: string | null;
  readonly hint: string | null;
  readonly schema: string | null;
  readonly table: string | null;
  readonly column: string | null;
  /** The domain whose `CHECK` or `NOT NULL` refused a value. */
  readonly datatype: string | null;
  readonly constraint: string | null;
  /** Whether pgorm's `Error::is_retryable` says the transaction is worth running again. */
  readonly isRetryable?: boolean;
}

/**
 * PostgreSQL rejected the statement. A constraint violation rejects with the
 * subclass named for it.
 */
export declare class DatabaseError extends PgormError implements DatabaseErrorDetails {
  constructor(message: string, details: DatabaseErrorDetails);
  readonly sqlstate: string;
  readonly severity: string;
  readonly detail: string | null;
  readonly hint: string | null;
  readonly schema: string | null;
  readonly table: string | null;
  readonly column: string | null;
  readonly datatype: string | null;
  readonly constraint: string | null;
}

/** A duplicate key in a unique index or primary key (SQLSTATE `23505`). */
export declare class UniqueViolationError extends DatabaseError {}

/** A foreign key with no matching referenced row, or a referenced row still in use (`23503`). */
export declare class ForeignKeyViolationError extends DatabaseError {}

/** A table's or a domain's `CHECK` constraint refused a value (`23514`); a domain's is named in `datatype`. */
export declare class CheckViolationError extends DatabaseError {}

/**
 * A null in a `NOT NULL` column, named in `column`, or in a domain that does
 * not allow one, named in `datatype` (`23502`).
 */
export declare class NotNullViolationError extends DatabaseError {}

/** A row conflicting with another under an exclusion constraint or a `WITHOUT OVERLAPS` key (`23P01`). */
export declare class ExclusionViolationError extends DatabaseError {}

// [spec:pgorm:req:bindings.values]

/** The built-in range kinds, named as PostgreSQL names the types. */
export type RangeKind = "int4range" | "int8range" | "numrange" | "daterange" | "tsrange" | "tstzrange";

/** The built-in multirange kinds. */
export type MultirangeKind =
  | "int4multirange"
  | "int8multirange"
  | "nummultirange"
  | "datemultirange"
  | "tsmultirange"
  | "tstzmultirange";

/**
 * A value kind with no type name, spelled as pgorm-python spells it. Each is
 * one of pgorm's value variants.
 *
 * | Kind | PostgreSQL | JavaScript |
 * | --- | --- | --- |
 * | `bool` | `boolean` | `boolean` |
 * | `i8`, `i16`, `i32`, `u32` | `"char"`, `int2`, `int4`, `oid` | `number`, an integer in range |
 * | `i64`, `u64` | `int8` | `bigint` (or a safe-integer `number`, in) |
 * | `f32`, `f64` | `float4`, `float8` | `number` (`f32` only when exact) |
 * | `text`, `char` | `text`, `varchar`, `bpchar`, `name` | `string` (`char`: one code point) |
 * | `bytes` | `bytea` | `Uint8Array` |
 * | `json` | `json`, `jsonb` | {@link JsonValue} |
 * | `decimal` | `numeric` | {@link Decimal} |
 * | `uuid` | `uuid` | {@link Uuid} |
 * | `date` | `date` | `Temporal.PlainDate` |
 * | `time` | `time` | `Temporal.PlainTime` |
 * | `datetime` | `timestamp` | `Temporal.PlainDateTime` |
 * | `datetime_utc` | `timestamptz` | `Temporal.Instant` |
 * | `interval` | `interval` | {@link Interval} (or a `Temporal.Duration`, in) |
 * | `ipnetwork` | `inet`, `cidr` | `string`, address and prefix |
 * | `mac_address` | `macaddr` | `Uint8Array` of six bytes |
 * | `vector` | pgvector's `vector` | `Float32Array` |
 * | a {@link RangeKind} or {@link MultirangeKind} | that type | {@link Range}, {@link Multirange} |
 */
export type ScalarKind =
  | "bool"
  | "i8"
  | "i16"
  | "i32"
  | "i64"
  | "u32"
  | "u64"
  | "f32"
  | "f64"
  | "text"
  | "char"
  | "bytes"
  | "json"
  | "decimal"
  | "uuid"
  | "date"
  | "time"
  | "datetime"
  | "datetime_utc"
  | "interval"
  | "ipnetwork"
  | "mac_address"
  | "vector"
  | RangeKind
  | MultirangeKind;

/** A kind: a scalar kind's name, an enum's {@link TypeName}, or a created range or multirange type. */
export type Kind = ScalarKind | TypeName | CreatedRange | CreatedMultirange;

/**
 * A JSON document as JavaScript. An integer literal past
 * `Number.MAX_SAFE_INTEGER` reads as a `bigint`, exactly; a number that has
 * no exact binary form is a {@link DecodeError}.
 */
export type JsonValue =
  | null
  | boolean
  | number
  | bigint
  | string
  | readonly JsonValue[]
  | { readonly [key: string]: JsonValue };

/** A value as a row or {@link Value.value} gives it: the JavaScript type of its kind, or `null` for SQL NULL. */
export type PlainValue =
  | null
  | boolean
  | number
  | bigint
  | string
  | Uint8Array
  | Float32Array
  | Decimal
  | Uuid
  | Interval
  | Temporal.PlainDate
  | Temporal.PlainTime
  | Temporal.PlainDateTime
  | Temporal.Instant
  | Range
  | Multirange
  | JsonValue
  | readonly PlainValue[];

// [spec:pgorm:req:napi.inference]
/**
 * A statement parameter: a {@link Value}, or a plain value whose kind is
 * inferred. `null` is SQL NULL of no declared kind; a number is an `i64` when
 * it is a safe integer and an `f64` otherwise; a bigint is an `i64`; a string
 * `text`; a `Uint8Array` `bytes`; a plain object `json`; a Temporal value or
 * one of the module's classes its kind; an array an array of the one kind its
 * items infer as. A range, an empty array and `undefined` cannot be inferred
 * and are refused with a {@link ConstructionError}.
 */
export type Param =
  | Value
  | null
  | boolean
  | number
  | bigint
  | string
  | Uint8Array
  | Decimal
  | Uuid
  | Interval
  | Temporal.PlainDate
  | Temporal.PlainTime
  | Temporal.PlainDateTime
  | Temporal.Instant
  | Temporal.Duration
  | { readonly [key: string]: JsonValue }
  | readonly Param[];

// [spec:pgorm:req:bindings.rows]
// [spec:pgorm:req:napi.records]
/** A row: its columns' values by column name, in column order. */
export type Row = { [column: string]: PlainValue };

/** A row read with `{ tagged: true }`: each column's value with its kind. */
export type TaggedRow = { [column: string]: Value };

/** A PostgreSQL type named by identifier, optionally schema-qualified: an enum's type. */
export declare class TypeName {
  #private;
  constructor(name: string, options?: { schema?: string | null });
  readonly name: string;
  readonly schema: string | null;
  /** The name quoted, as SQL spells it: `"app"."mood"`. */
  toString(): string;
}

/** The subtypes a created range type can range over. */
export type CreatedSubtype =
  | "i16"
  | "i32"
  | "i64"
  | "f32"
  | "f64"
  | "decimal"
  | "text"
  | "date"
  | "time"
  | "datetime"
  | "datetime_utc"
  | "uuid";

/**
 * A range type a schema created with `CREATE TYPE ... AS RANGE`, and the kind
 * of its subtype, which converts each bound. pgorm holds a value of one as its
 * text form, so a statement reads it from text and casts it:
 * `CAST($1::text AS measure.floatrange)`.
 */
export declare class CreatedRange<S extends CreatedSubtype = CreatedSubtype> {
  #private;
  constructor(name: string, subtype: S, options?: { schema?: string | null });
  readonly name: string;
  readonly subtype: S;
  readonly schema: string | null;
}

/** The multirange type PostgreSQL creates beside a created range type, named by its own name. */
export declare class CreatedMultirange<S extends CreatedSubtype = CreatedSubtype> {
  #private;
  constructor(name: string, subtype: S, options?: { schema?: string | null });
  readonly name: string;
  readonly subtype: S;
  readonly schema: string | null;
}

/**
 * An exact decimal: PostgreSQL's `numeric` within pgorm's range, a 96-bit
 * coefficient and at most 28 fractional digits. Made from a string in plain
 * notation or a bigint, never a number; its text keeps its scale (`"19.9900"`).
 * It does not become a number implicitly, which could round it.
 */
export declare class Decimal {
  constructor(value: string | bigint);
  toString(): string;
  toJSON(): string;
}

/** A UUID, held as its lower-case hyphenated text. */
export declare class Uuid {
  constructor(value: string);
  equals(other: unknown): boolean;
  toString(): string;
  toJSON(): string;
}

/**
 * PostgreSQL's `interval`: months, days and microseconds, each with its own
 * sign, because a month has no fixed number of days nor a day of hours. A
 * `Temporal.Duration`'s fields share one sign, so it cannot hold
 * `1 mon -2 days`; this can.
 */
export declare class Interval {
  constructor(fields?: { months?: number; days?: number; microseconds?: bigint | number });
  readonly months: number;
  readonly days: number;
  readonly microseconds: bigint;
  /**
   * From a `Temporal.Duration` or an ISO 8601 duration: years and months
   * become months, weeks and days days, the rest microseconds. A nanosecond
   * digit is a {@link ConstructionError}, never truncated.
   */
  static from(value: Interval | Temporal.Duration | string): Interval;
  /** The `Temporal.Duration` of the same fields; a `RangeError` when their signs differ. */
  toDuration(): Temporal.Duration;
  /** PostgreSQL's `iso_8601` interval style, each field signed: `P1M-2DT3H`. */
  toString(): string;
  toJSON(): string;
}

/**
 * A range: the empty range, or the values between two bounds, `null` on a
 * side being no bound. A side with no bound includes nothing, so its bracket
 * is always `(` or `)`. Its kind is not inferred from its bounds: bind one as
 * `new Value(range, "int4range")`, or with a {@link CreatedRange}.
 */
export declare class Range<T = PlainValue> {
  constructor(lower?: T | null, upper?: T | null, bounds?: "[)" | "[]" | "(]" | "()");
  /** The range containing no value, unlike `new Range()`, which contains every value. */
  static empty<T = PlainValue>(): Range<T>;
  readonly lower: T | null;
  readonly upper: T | null;
  readonly lowerInclusive: boolean;
  readonly upperInclusive: boolean;
  readonly isEmpty: boolean;
  readonly bounds: "[)" | "[]" | "(]" | "()";
  toString(): string;
}

/** A multirange: a set of ranges of one subtype. */
export declare class Multirange<T = PlainValue> implements Iterable<Range<T>> {
  constructor(ranges?: Iterable<Range<T>>);
  readonly ranges: readonly Range<T>[];
  readonly length: number;
  [Symbol.iterator](): Iterator<Range<T>>;
  toString(): string;
}

// [spec:pgorm:req:bindings.value-tags+1]
/**
 * A value with its kind declared: what plain JavaScript cannot tell apart —
 * an integer's width, a typed SQL NULL, JSON's null, an empty array, an enum's
 * type, a range's type — and any other kind written out explicitly.
 *
 * Construction converts at once, and throws a {@link ConstructionError} for
 * data the kind cannot hold exactly. A value is immutable and can be bound
 * any number of times.
 */
export declare class Value {
  /** A value of `kind`, or, with no kind, of the kind `data` infers as. */
  constructor(data: unknown, kind?: Kind);
  /** SQL NULL of a kind. */
  static null(kind: Kind): Value;
  /** A JSON document; `null` here is JSON's null, not SQL NULL. */
  static json(data: JsonValue): Value;
  /**
   * An array of a declared element kind, the empty array included; `items`
   * `null` is an SQL NULL array that still names its element kind.
   */
  static array(kind: Kind, items: readonly unknown[] | null): Value;
  /** The kind's name: a {@link ScalarKind}, `enum`, `created_range`, `created_multirange` or `array`. */
  readonly kind: string;
  readonly isNull: boolean;
  /** The type an enum or a created range is named by, an array's being its element kind's. */
  readonly typeName: TypeName | null;
  /** A created range's or multirange's type. */
  readonly createdType: CreatedRange | CreatedMultirange | null;
  /** An array's element kind. */
  readonly elementType: Kind | null;
  /** The plain JavaScript value, independently owned. */
  readonly value: PlainValue;
  /** An array's items, each a value of the element kind; `null` for an SQL NULL array. */
  items(): Value[] | null;
  /** The same kind and the same value; floats compare by their bits. */
  equals(other: unknown): boolean;
  toString(): string;
}

// [spec:pgorm:req:napi.lifecycle+3]
/** How a {@link Pool} connects. Durations are in milliseconds. */
// [spec:pgorm:req:bindings.tls]
/**
 * How a connection uses TLS, named as libpq's `sslmode` names it:
 * `"verify-full"` verifies the server's certificate chain and host name;
 * `"verify-ca"` its chain to a `ca` the program gives, whatever host it names;
 * `"require"` is TLS with no certificate verified; `"prefer"` TLS when the
 * server offers it and plaintext when it does not, nothing verified; and
 * `"disable"` plaintext. libpq's `allow` is not offered.
 */
export type TlsMode = "disable" | "prefer" | "require" | "verify-ca" | "verify-full";

// [spec:pgorm:req:bindings.tls]
/** How a pool or a listener of its own reaches its server, and over what TLS. */
export interface ConnectionOptions {
  /**
   * The TLS mode. Without it the connection string's `sslmode` decides:
   * `disable` is plaintext, and `prefer`, `require` or none is
   * `"verify-full"`, which never falls back to plaintext. A mode that verifies
   * less is only ever this option's.
   */
  readonly tls?: TlsMode;
  /** PEM certificates a verifying mode trusts, in place of the platform's trust store; `"verify-ca"` needs it. */
  readonly ca?: string | Uint8Array;
  /** A PEM client certificate chain, shown to a server that asks for one; given with its {@link key}. */
  readonly cert?: string | Uint8Array;
  /** The {@link cert}'s PEM private key: PKCS #8, PKCS #1 or SEC1, unencrypted. */
  readonly key?: string | Uint8Array;
  /** How long opening a connection may take, in milliseconds; 10 000 by default. */
  readonly connectTimeout?: number;
}

// [spec:pgorm:req:bindings.pool-hooks]
/**
 * A pool's hook: called on this thread with the connection it is about, it
 * may return a promise, which the hook waits for. One that throws or rejects
 * refuses the connection.
 */
export type PoolHook = (connection: HookConnection) => void | Promise<void>;

// [spec:pgorm:req:bindings.connections+3]
/** What one pool takes of its own: the options Rust's `PoolBuilder` closure sets, and how long acquiring waits. */
export interface PoolShapeOptions {
  /** Connections the pool keeps at most; 10 by default. */
  readonly maxSize?: number;
  /**
   * How long acquiring a connection may take in all, in milliseconds, before a
   * {@link TimeoutError}; 30 000 by default.
   */
  readonly acquireTimeout?: number;
  /** How long acquiring waits for a free connection, in milliseconds; unbounded unless given. */
  readonly waitTimeout?: number;
  /** How long opening a new connection may take, in milliseconds; unbounded unless given. */
  readonly createTimeout?: number;
  /** How long checking a returned connection may take, in milliseconds; unbounded unless given. */
  readonly recycleTimeout?: number;
  /** Lend the idle connection returned first (`"fifo"`, the default) or last (`"lifo"`). */
  readonly queueMode?: "fifo" | "lifo";
  /** Run on each connection as it opens, before the pool lends it. */
  readonly postCreate?: PoolHook;
  /** Run on a returned connection before the pool checks it for reuse. */
  readonly preRecycle?: PoolHook;
  /** Run on a returned connection once it passed its check, before the pool lends it again. */
  readonly postRecycle?: PoolHook;
}

// [spec:pgorm:req:bindings.connections+3]
/** What every pool built from one connection string shares. */
export interface SharedPoolOptions extends ConnectionOptions {
  /** Prepared statements each connection caches; 0 disables the cache; 128 by default. */
  readonly statementCacheSize?: number;
  /**
   * How a returned connection is checked before reuse: `"verified"` (the
   * default) runs an empty query, `"fast"` sends nothing, and `"clean"` also
   * resets the session — Rust's `RecyclingMethod::Clean`.
   */
  readonly recycle?: "verified" | "fast" | "clean";
  /**
   * SQL of the program's own, run on each returned connection before reuse:
   * Rust's `RecyclingMethod::Custom`. A `TypeError` beside {@link recycle}.
   */
  readonly recycleSql?: string;
  /**
   * A channel of 1 to 63 bytes the pool listens on, through a connection of
   * its own opened by its first acquisition and closed with the pool. Every
   * notification there, whatever its payload, clears every connection's
   * cached statements as {@link Pool.clearStatementCaches} does; the
   * connection reconnects if it drops, clearing once each time it listens
   * again. Absent by default. {@link STATEMENT_CACHE_CHANNEL} is the channel
   * pgorm-migration's runner notifies.
   */
  readonly statementCacheChannel?: string;
}

/** How a {@link Pool} is built. */
export interface PoolOptions extends SharedPoolOptions, PoolShapeOptions {
  /** The pool's name, which {@link Pool.tag} reports; pgorm names an untagged pool `default-N`. */
  readonly tag?: string;
}

/** A snapshot of a pool's capacity and use. */
export interface PoolStatus {
  readonly maxSize: number;
  readonly size: number;
  readonly available: number;
  readonly waiting: number;
}

// [spec:pgorm:req:napi.cancellation+2]
/** Options of the operations that wait on the server or the pool: an `AbortSignal` that ends one. */
export interface OperationOptions {
  /**
   * Aborting before the operation settles rejects with the signal's reason,
   * even where the work had finished. Stopped part way, its outcome is
   * unknown — a write may or may not have happened — so its connection is
   * discarded; finished, its result is discarded. Either way a transaction
   * it ran in ends with nothing committed.
   */
  readonly signal?: AbortSignal;
}

// [spec:pgorm:req:napi.records]
/** Options of the row terminals. */
export interface QueryOptions extends OperationOptions {
  /** Give each column as a {@link Value} carrying its kind. */
  readonly tagged?: boolean;
}

/** The row a terminal gives for its options. */
export type RowFor<O extends QueryOptions> = O extends { readonly tagged: true } ? TaggedRow : Row;

// [spec:pgorm:req:napi.transactions+3]
/** How a transaction opens; a savepoint inherits its transaction's. */
export interface TransactionOptions extends OperationOptions {
  /**
   * `"default"` inherits the session's; `"readWrite"` and `"readOnly"` set the
   * access mode; `"deferrable"` is `SERIALIZABLE READ ONLY DEFERRABLE`.
   */
  readonly mode?: "default" | "readWrite" | "readOnly" | "deferrable";
  /** An isolation level, with `"readWrite"` or `"readOnly"` only. */
  readonly isolation?: "readUncommitted" | "readCommitted" | "repeatableRead" | "serializable" | null;
}

/** How a retrying transaction opens, and how many times it runs again. */
export interface RetryOptions extends TransactionOptions {
  /** How many more times the transaction may run after its first attempt. */
  readonly retries: number;
}

// [spec:pgorm:req:bindings.statements]
/**
 * A statement a builder made, which a terminal runs as `inspect()` builds it,
 * its values bound.
 */
export type Statement = Select | Insert | Update | Delete | Merge | DDL | Pipeline;

// [spec:pgorm:req:bindings.sql-check]
/**
 * `text`, held to PostgreSQL 18's grammar and returned unchanged, as Rust's
 * `sql!` holds a literal while its crate compiles; a script is held to it
 * statement by statement. What the grammar rejects is a
 * {@link ConstructionError} whose message reprints the line with a caret
 * under the token the parser stopped at. Call it once, where a module
 * defines its statements.
 */
export declare function sql<T extends string>(text: T): T;

// [spec:pgorm:req:bindings.prql-text]
/**
 * `text`, PRQL, compiled to PostgreSQL SQL held to the grammar, with `values`
 * for its placeholders in order, as Rust's `prql!` expands: one value per
 * distinct placeholder. The pair runs as SQL text and its array do.
 */
export declare function prql(text: string, values?: readonly Param[]): { readonly sql: string; readonly values: readonly Param[] };

// [spec:pgorm:req:bindings.raw-models]
/**
 * A raw statement whose rows decode into a declared model's records, as
 * Rust's `SelectorRaw` decodes them — each field read from the column of its
 * SQL name and held to its declared kind, other columns left unread — or,
 * from {@link fromStatement}, are plain rows. pgorm adds nothing to the SQL,
 * so `one` and `optional` add no `LIMIT`. `paginate` and `count` read the
 * statement as a subquery, a {@link ConstructionError} unless it is one
 * row-returning `SELECT`, as Rust's `SelectorRaw::paginate` refuses one.
 */
export declare class SelectorRaw<R> {
  private constructor();
  inspect(): { readonly sql: string; readonly values: readonly Param[] };
  all(db: Queryable, options?: OperationOptions): Promise<R[]>;
  one(db: Queryable, options?: OperationOptions): Promise<R>;
  optional(db: Queryable, options?: OperationOptions): Promise<R | null>;
  stream(db: Queryable, options?: OperationOptions): RowStream<R>;
  paginate(pageSize: number): Paginator<R>;
  count(db: Queryable, options?: OperationOptions): Promise<number>;
}

/** SQL text and its parameters as a raw statement of plain rows, which pages and counts. */
export declare function fromStatement(sql: string, params?: readonly Param[]): SelectorRaw<Row>;

/**
 * What a pool, a connection and a transaction run bound SQL through: SQL text
 * and its parameters, or a {@link Statement} and its options. A parameter is
 * bound, never interpolated.
 */
export declare abstract class Queryable {
  /** Run a statement and resolve with the number of rows it affected. */
  execute(statement: Statement, options?: OperationOptions): Promise<number>;
  execute(sql: string, params?: readonly Param[], options?: OperationOptions): Promise<number>;
  /** Every row. */
  query<O extends QueryOptions = QueryOptions>(statement: Statement, options?: O): Promise<RowFor<O>[]>;
  query<O extends QueryOptions = QueryOptions>(sql: string, params?: readonly Param[], options?: O): Promise<RowFor<O>[]>;
  /** Exactly one row; any other count is a {@link DecodeError}. */
  one<O extends QueryOptions = QueryOptions>(statement: Statement, options?: O): Promise<RowFor<O>>;
  one<O extends QueryOptions = QueryOptions>(sql: string, params?: readonly Param[], options?: O): Promise<RowFor<O>>;
  /** At most one row, or `null`; more is a {@link DecodeError}. */
  optional<O extends QueryOptions = QueryOptions>(statement: Statement, options?: O): Promise<RowFor<O> | null>;
  optional<O extends QueryOptions = QueryOptions>(
    sql: string,
    params?: readonly Param[],
    options?: O,
  ): Promise<RowFor<O> | null>;
  // [spec:pgorm:req:bindings.copy]
  /**
   * Run a `COPY … FROM STDIN`, writing the bytes `source` yields, and resolve
   * with the rows it wrote. The source is pulled only as the COPY takes its
   * data, so a large one streams. A source that throws, or yields anything
   * but a `Uint8Array`, abandons the COPY — nothing is written, and inside a
   * transaction the transaction is aborted — and the promise rejects with
   * its error. A row the server refuses rejects with a {@link DatabaseError}
   * once the data is sent. The connection or transaction is held until the
   * COPY ends, refusing other work with a {@link LifecycleError}; a pool's
   * COPY takes a connection of its own.
   */
  copyIn(statement: CopyFrom, source: CopySource, options?: OperationOptions): Promise<number>;
  /**
   * Run a `COPY … TO STDOUT` and resolve, once the server accepts it, with
   * its data as a {@link CopyOutStream}, which holds the connection or
   * transaction until it ends or closes; a pool's holds a connection of its
   * own.
   */
  copyOut(statement: CopyTo, options?: OperationOptions): Promise<CopyOutStream>;
  // [spec:pgorm:req:bindings.copy-rows+2]
  /**
   * Run a binary `COPY … FROM STDIN` whose rows are arrays of values, one for
   * each column the server describes for the statement, in order, and
   * resolve with the rows it wrote. Each value is read as a {@link Param} is
   * and written against its column's type as a parameter is bound; a row of
   * another width, or a value its column's type does not take, is a
   * {@link ConstructionError} that abandons the COPY, as a source that throws
   * does. The rows are pulled only as the COPY takes them, a batch at a time.
   */
  copyInRows(statement: CopyFrom<"binary">, rows: CopyRows, options?: OperationOptions): Promise<number>;
  /**
   * With no rows, resolve once the server accepts the COPY with a
   * {@link CopyRowSink} the program writes a row at a time, which holds the
   * connection or transaction until it finishes or closes; a pool's holds a
   * connection of its own. The signal, if any, abandons the COPY whenever it
   * fires before the COPY finishes.
   */
  copyInRows(statement: CopyFrom<"binary">, rows?: undefined, options?: OperationOptions): Promise<CopyRowSink>;
  /**
   * The columns a `COPY … FROM STDIN` in any format takes, in order, as the
   * server describes them, without starting it. A table or column that does
   * not exist is the server's {@link DatabaseError}, which leaves the
   * connection or transaction as a failed statement does.
   */
  copyInColumns(statement: CopyFrom, options?: OperationOptions): Promise<readonly CopyColumn[]>;
  /** The columns a `COPY … TO STDOUT` in any format gives, in order, as the server describes them, without starting it. */
  copyOutColumns(statement: CopyTo, options?: OperationOptions): Promise<readonly CopyColumn[]>;
  /**
   * Run a binary `COPY … TO STDOUT` and resolve, once the server accepts it,
   * with its rows as a {@link CopyRowStream} of arrays of values, each field
   * decoded by its column's type as a result column of that type is.
   */
  copyOutRows(statement: CopyTo<"binary">, options?: OperationOptions): Promise<CopyRowStream>;
  // [spec:pgorm:req:bindings.listen+3]
  /**
   * Send `payload` on `channel` through `SELECT pg_notify($1, $2)`, both
   * bound; the channel is exact, as {@link Listener.listen} names it. In a
   * transaction it is delivered when the transaction commits, and never if it
   * rolls back. A payload of 8000 bytes or more, an empty channel and one
   * longer than 63 bytes are the server's {@link DatabaseError}, SQLSTATE
   * `22023`.
   */
  notify(channel: string, payload: string, options?: OperationOptions): Promise<void>;
  // [spec:pgorm:req:bindings.batch-execute]
  /**
   * Run `script`, statements separated by `;` with no parameters, as Rust's
   * `batch_execute` does: sent as it is through the simple-query protocol,
   * nothing prepared. It is for DDL and fixed scripts, never for values a
   * user supplied.
   */
  batchExecute(script: string, options?: OperationOptions): Promise<void>;
  // [spec:pgorm:req:bindings.verify]
  /**
   * Check `model` against the result the server describes for `statement`,
   * without running it, as Rust's `verify` does: every field's column
   * returned, of a type the field's kind reads, or a {@link DecodeError}
   * naming the field. The statement's values are neither bound nor checked.
   */
  verify(sql: string, model: Model<any>, options?: OperationOptions): Promise<void>;
}

/**
 * A pool of connections to one server, opened as they are first needed: the
 * constructor sends nothing, and throws a {@link ConstructionError} for an
 * unusable connection string or option. Its statements each run on a
 * connection of their own. Close it to release its connections; it is also
 * `await using`-disposable.
 */
export declare class Pool extends Queryable implements AsyncDisposable {
  constructor(dsn: string, options?: PoolOptions);
  readonly closed: boolean;
  /** The pool's {@link PoolOptions.tag}, or the `default-N` pgorm named it. */
  readonly tag: string;
  status(): PoolStatus;
  /**
   * Drop every connection's cached prepared statements, so each statement is
   * prepared again: an idle connection's before it is next lent, and a
   * checked-out one's before it is lent again after its return. Call it after
   * DDL a cached plan does not see, such as a domain's changed default.
   */
  clearStatementCaches(): void;
  /** A connection of the caller's own until it closes it. */
  acquire(options?: OperationOptions): Promise<Connection>;
  /** Run `use` with a connection, closed when it settles. */
  connection<T>(use: (connection: Connection) => Promise<T>, options?: OperationOptions): Promise<T>;
  /** Run `use` in a transaction on a connection of its own: committed when it resolves, rolled back when it throws. */
  transaction<T>(use: (transaction: Transaction) => Promise<T>, options?: TransactionOptions): Promise<T>;
  /** {@link Connection.transactionWithRetry} on a connection of its own. */
  transactionWithRetry<T>(use: (transaction: Transaction) => Promise<T>, options: RetryOptions): Promise<T>;
  /** The rows of a statement, one per pull, over a connection the stream holds until it ends. */
  stream<O extends QueryOptions = QueryOptions>(statement: Statement, options?: O): RowStream<RowFor<O>>;
  stream<O extends QueryOptions = QueryOptions>(sql: string, params?: readonly Param[], options?: O): RowStream<RowFor<O>>;
  /** Whether the server answers. */
  ping(options?: OperationOptions): Promise<boolean>;
  /**
   * A {@link Listener} on a connection of its own, opened with the pool's
   * configuration and closed with the pool; it is not one the pool lends.
   */
  listener(options?: ListenerOptions): Promise<Listener>;
  /** Refuse new work, cancel what runs, and resolve once every connection and listener is released. */
  close(): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
}

/** A {@link Pool} whose server has answered a ping. */
export declare function connect(dsn: string, options?: PoolOptions): Promise<Pool>;

/**
 * One connection checked out of a pool, the caller's until it closes it. One
 * operation runs on it at a time: another started meanwhile is a
 * {@link LifecycleError}, as is any while a transaction or stream holds it.
 */
export declare class Connection extends Queryable implements AsyncDisposable {
  private constructor();
  readonly closed: boolean;
  /**
   * Run `use` in a transaction, which holds the connection until `use`
   * settles: committed when it resolves, rolled back when it throws. Rejects
   * with what `use` threw, or with a `SuppressedError` over it if the
   * ROLLBACK failed; a failed commit or ROLLBACK discards the connection.
   */
  transaction<T>(use: (transaction: Transaction) => Promise<T>, options?: TransactionOptions): Promise<T>;
  // [spec:pgorm:req:bindings.transaction-retry]
  /**
   * Run `use` in a transaction, and again from `BEGIN`, up to `retries` more
   * times, while it fails with a serialization failure or a deadlock, as
   * Rust's `transaction_with_retry` does: `use` rejecting with an error whose
   * {@link PgormError.isRetryable} is true, or a `COMMIT` failing so. `use`
   * receives each attempt's {@link Transaction}, and may run more than once,
   * so whatever it does outside the database happens again each time.
   */
  transactionWithRetry<T>(use: (transaction: Transaction) => Promise<T>, options: RetryOptions): Promise<T>;
  /**
   * The rows of `sql`, one per pull. The stream holds the connection until its
   * last row, which frees it, or until it is closed early, which discards it.
   */
  stream<O extends QueryOptions = QueryOptions>(statement: Statement, options?: O): RowStream<RowFor<O>>;
  stream<O extends QueryOptions = QueryOptions>(sql: string, params?: readonly Param[], options?: O): RowStream<RowFor<O>>;
  ping(options?: OperationOptions): Promise<boolean>;
  /** Return the connection to its pool, ending a transaction or stream that holds it; resolves once released. */
  close(): Promise<void>;
  // [spec:pgorm:req:bindings.connections+3]
  /**
   * Close the connection and discard it rather than return it to its pool, as
   * Rust's `DatabaseConnection::discard` does: for one whose session state the
   * next borrower should not find. The pool opens a fresh one in its place.
   */
  discard(): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
}

// [spec:pgorm:req:bindings.pool-hooks]
/**
 * The connection a pool's hook is about, while the hook runs. Its statements
 * run on that connection, through pgorm's statement cache, as a
 * {@link Connection}'s do; once the hook has returned they reject with a
 * {@link LifecycleError}.
 */
export declare class HookConnection {
  private constructor();
  /** Milliseconds since the connection opened. */
  readonly age: number;
  /** How many times the pool has recycled the connection. */
  readonly recycleCount: number;
  /** Whether the hook has returned. */
  readonly closed: boolean;
  /** Run a statement and resolve with the number of rows it affected. */
  execute(statement: Statement): Promise<number>;
  execute(sql: string, params?: readonly Param[]): Promise<number>;
  /** Every row. */
  query<O extends { readonly tagged?: boolean } = {}>(statement: Statement, options?: O): Promise<RowFor<O>[]>;
  query<O extends { readonly tagged?: boolean } = {}>(sql: string, params?: readonly Param[], options?: O): Promise<RowFor<O>[]>;
  /** Exactly one row; any other count is a {@link DecodeError}. */
  one<O extends { readonly tagged?: boolean } = {}>(statement: Statement, options?: O): Promise<RowFor<O>>;
  one<O extends { readonly tagged?: boolean } = {}>(sql: string, params?: readonly Param[], options?: O): Promise<RowFor<O>>;
  /** At most one row, or `null`; more is a {@link DecodeError}. */
  optional<O extends { readonly tagged?: boolean } = {}>(statement: Statement, options?: O): Promise<RowFor<O> | null>;
  optional<O extends { readonly tagged?: boolean } = {}>(
    sql: string,
    params?: readonly Param[],
    options?: O,
  ): Promise<RowFor<O> | null>;
}

// [spec:pgorm:req:bindings.multi-pool]
/**
 * Several pools against one server, one per property of `pools`, each tagged
 * with its key and shaped by its options, every one sharing `options`, as
 * Rust's `connect_multi_with` builds them. Building one sends nothing; which
 * pool a statement runs on is the program's choice.
 */
export declare class MultiPool implements AsyncDisposable {
  constructor(dsn: string, pools: Readonly<Record<string, PoolShapeOptions>>, options?: SharedPoolOptions);
  /** Whether every pool is closed. */
  readonly closed: boolean;
  /** The pool tagged `tag`, or `null`. */
  get(tag: string): Pool | null;
  /** Every pool with its tag, in the tags' order. */
  entries(): [string, Pool][];
  /** Each pool's status, by tag. */
  status(): Record<string, PoolStatus>;
  /** Close every pool. */
  close(): Promise<void>;
  [Symbol.asyncDispose](): Promise<void>;
}

/**
 * A transaction, or a savepoint inside one, open for the callback a
 * `transaction(fn)` runs and ended as that callback settles. It borrows its
 * parent exclusively: while a savepoint is open, its transaction refuses
 * statements and other savepoints with a {@link LifecycleError}, and so does
 * a transaction already running a statement.
 */
export declare class Transaction extends Queryable {
  private constructor();
  readonly closed: boolean;
  /**
   * The connection, or for a savepoint the transaction, this one borrows;
   * `null` for the transaction a migration's `up` is lent, which
   * pgorm-migration's runner opened.
   */
  readonly parent: Connection | Transaction | null;
  /**
   * Run `use` in a savepoint, which holds this transaction until `use`
   * settles: released when it resolves, rolled back to when it throws.
   */
  transaction<T>(use: (transaction: Transaction) => Promise<T>, options?: OperationOptions): Promise<T>;
  // [spec:pgorm:req:bindings.streams+1]
  /**
   * The rows of `sql`, one per pull. The stream holds the transaction until
   * its last row, refusing other work meanwhile; closed before then, it
   * discards the transaction with its connection.
   */
  stream<O extends QueryOptions = QueryOptions>(statement: Statement, options?: O): RowStream<RowFor<O>>;
  stream<O extends QueryOptions = QueryOptions>(sql: string, params?: readonly Param[], options?: O): RowStream<RowFor<O>>;
}

// [spec:pgorm:req:bindings.streams+1]
/**
 * A statement's rows as an async iterator, one row per pull. Its connection
 * is released at the last row, by `close()`, or when a `for await` loop leaves
 * early; closed before its last row, the connection is discarded.
 */
export declare class RowStream<R = Row> implements AsyncIterableIterator<R>, AsyncDisposable {
  private constructor();
  readonly closed: boolean;
  next(): Promise<IteratorResult<R, undefined>>;
  return(): Promise<IteratorResult<R, undefined>>;
  close(): Promise<void>;
  [Symbol.asyncIterator](): AsyncIterableIterator<R>;
  [Symbol.asyncDispose](): Promise<void>;
}
