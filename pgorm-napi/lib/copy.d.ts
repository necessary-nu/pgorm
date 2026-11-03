/**
 * COPY: `COPY … FROM STDIN` and `COPY … TO STDOUT`, built over pgorm-query's
 * statements and run by `copyIn` and `copyOut` on a pool, a connection or a
 * transaction.
 *
 * A COPY's format is named as it is made, and is the statement's type
 * parameter: an option its format does not take — a quote on a text COPY, a
 * header on a binary one — is no method of its type, and called anyway it is
 * refused with a `ConstructionError`, as is one its direction does not
 * take. COPY takes no bind parameters, so `inspect()` gives the statement's
 * SQL, every value in its `WHERE` or its query a literal, and no values.
 *
 * @module
 */

// [spec:pgorm:req:napi.typing+1]
// [spec:pgorm:req:bindings.copy]
// [spec:pgorm:req:bindings.copy-rows+1]

import type { Builder, Predicate } from "./expressions.d.ts";
import type { Param, PlainValue } from "./index.d.ts";
import type { Merge } from "./merge.d.ts";
import type { TableRef } from "./schema.d.ts";
import type { Select } from "./select.d.ts";
import type { Delete, Insert, Update } from "./writes.d.ts";

/** COPY's three data formats: PostgreSQL's own lines of text, CSV, and its binary tuple format. */
export type CopyFormat = "text" | "csv" | "binary";

/** The formats that are lines of delimited text, and take a delimiter, a NULL string, a header and an encoding. */
export type CopyDelimitedFormat = "text" | "csv";

/**
 * `ON_ERROR ignore`'s options: `rejectLimit`, a whole number from 1, fails the
 * COPY once more rows than it have been skipped; `logVerbosity` is how much
 * the server reports about them.
 */
export interface CopyIgnoreOptions {
  readonly rejectLimit?: number | bigint;
  readonly logVerbosity?: "verbose" | "silent";
}

/**
 * What a COPY TO reads: a table, or the rows of a `Select`, or of an INSERT,
 * UPDATE, DELETE or MERGE through its RETURNING list. A write without one is
 * refused with a `ConstructionError` when the COPY is built.
 */
export type CopyToSource = TableRef | Select | Insert | Update | Delete | Merge;

/**
 * A `COPY table [(columns)] FROM STDIN`, run by `copyIn`; every method
 * returns a new statement.
 */
export declare class CopyFrom<F extends CopyFormat = CopyFormat> extends Builder {
  private constructor();
  readonly format: F;
  /**
   * The columns each row supplies, in order, added to those named before; a column left out takes its default.
   * A generated column takes no data: a list naming one is refused when the COPY starts, closing its connection.
   * With no list, the generated columns are left out.
   */
  columns(columns: readonly string[]): CopyFrom<F>;
  /** `FREEZE`: write the rows already frozen; the server refuses it unless the table was made or truncated in this transaction. */
  freeze(): CopyFrom<F>;
  /** Only the rows `predicate` holds for, ANDed to a condition already there. */
  where(predicate: Predicate): CopyFrom<F>;
  /** `DELIMITER`, a one-character string: tab in text and comma in CSV by default. */
  delimiter(this: CopyFrom<CopyDelimitedFormat>, delimiter: string): CopyFrom<F>;
  /** `NULL`: the field that reads as SQL NULL. */
  nullString(this: CopyFrom<CopyDelimitedFormat>, text: string): CopyFrom<F>;
  /** `DEFAULT`: the field that reads as the column's default. */
  defaultString(this: CopyFrom<CopyDelimitedFormat>, text: string): CopyFrom<F>;
  /** `HEADER true`: the first line is a header, and is skipped. */
  header(this: CopyFrom<CopyDelimitedFormat>): CopyFrom<F>;
  /** `HEADER match`: the first line is a header naming the columns in order, or the COPY fails. */
  headerMatch(this: CopyFrom<CopyDelimitedFormat>): CopyFrom<F>;
  /** `ENCODING`: the data's encoding, by the server's name for it, when it is not the client's. */
  encoding(this: CopyFrom<CopyDelimitedFormat>, encoding: string): CopyFrom<F>;
  /** `ON_ERROR ignore`: skip a row whose field does not convert rather than fail the COPY. */
  onErrorIgnore(this: CopyFrom<CopyDelimitedFormat>, options?: CopyIgnoreOptions): CopyFrom<F>;
  /** `QUOTE`, a one-character string, `"` by default. */
  quote(this: CopyFrom<"csv">, quote: string): CopyFrom<F>;
  /** `ESCAPE`, a one-character string, the quote by default. */
  escape(this: CopyFrom<"csv">, escape: string): CopyFrom<F>;
  /** `FORCE_NOT_NULL`: these columns' fields never read as NULL; an empty list forces none. */
  forceNotNull(this: CopyFrom<"csv">, columns: readonly string[]): CopyFrom<F>;
  forceNotNullAll(this: CopyFrom<"csv">): CopyFrom<F>;
  /** `FORCE_NULL`: these columns' fields matching the NULL string read as NULL even quoted; an empty list forces none. */
  forceNull(this: CopyFrom<"csv">, columns: readonly string[]): CopyFrom<F>;
  forceNullAll(this: CopyFrom<"csv">): CopyFrom<F>;
}

/**
 * A `COPY { table | (query) } TO STDOUT`, run by `copyOut`; every method
 * returns a new statement. It has no column list: a subset of a table's
 * columns is a `Select` of them.
 */
export declare class CopyTo<F extends CopyFormat = CopyFormat> extends Builder {
  private constructor();
  readonly format: F;
  /** `DELIMITER`, a one-character string: tab in text and comma in CSV by default. */
  delimiter(this: CopyTo<CopyDelimitedFormat>, delimiter: string): CopyTo<F>;
  /** `NULL`: the field written for SQL NULL. */
  nullString(this: CopyTo<CopyDelimitedFormat>, text: string): CopyTo<F>;
  /** `HEADER true`: write the column names as the first line. */
  header(this: CopyTo<CopyDelimitedFormat>): CopyTo<F>;
  /** `ENCODING`: the encoding to write in, by the server's name for it. */
  encoding(this: CopyTo<CopyDelimitedFormat>, encoding: string): CopyTo<F>;
  /** `QUOTE`, a one-character string, `"` by default. */
  quote(this: CopyTo<"csv">, quote: string): CopyTo<F>;
  /** `ESCAPE`, a one-character string, the quote by default. */
  escape(this: CopyTo<"csv">, escape: string): CopyTo<F>;
  /** `FORCE_QUOTE`: quote every non-NULL value of these columns; an empty list forces none. */
  forceQuote(this: CopyTo<"csv">, columns: readonly string[]): CopyTo<F>;
  forceQuoteAll(this: CopyTo<"csv">): CopyTo<F>;
}

/** `COPY table FROM STDIN` in `format`, the table a name or a `Table` without an alias. */
export declare function copyFrom<F extends CopyFormat>(table: TableRef, format: F): CopyFrom<F>;

/** `COPY … TO STDOUT` in `format`, reading a table or a statement's rows; a write needs a RETURNING list. */
export declare function copyTo<F extends CopyFormat>(source: CopyToSource, format: F): CopyTo<F>;

/**
 * What `copyIn` writes: the bytes of the rows in the statement's format,
 * split anywhere — a `Uint8Array`, or an iterable or async iterable of them,
 * a web `ReadableStream<Uint8Array>` or a Node.js stream of buffers among
 * them. A string is refused with a `TypeError`.
 */
export type CopySource = Uint8Array | Iterable<Uint8Array> | AsyncIterable<Uint8Array>;

/**
 * A COPY TO's data as an async iterator of `Uint8Array` chunks, one per pull,
 * so the server is held back while nothing asks. The chunks are pieces of one
 * byte stream in the statement's format, none promised to end at a row. It
 * holds what the COPY runs on until its end, which frees it, or until
 * `close()`, `return()` — a `for await` loop leaving early — or a failed
 * pull, which discard the connection if data remained. A second pull while
 * one is pending is a `LifecycleError`.
 */
export declare class CopyOutStream implements AsyncIterableIterator<Uint8Array>, AsyncDisposable {
  private constructor();
  readonly closed: boolean;
  next(): Promise<IteratorResult<Uint8Array, undefined>>;
  return(): Promise<IteratorResult<Uint8Array, undefined>>;
  close(): Promise<void>;
  [Symbol.asyncIterator](): AsyncIterableIterator<Uint8Array>;
  [Symbol.asyncDispose](): Promise<void>;
}

/**
 * What `copyInRows` writes: rows, each an array of its columns' values in
 * the order the server describes the statement's columns, from an iterable
 * or async iterable.
 */
export type CopyRows = Iterable<readonly Param[]> | AsyncIterable<readonly Param[]>;

/**
 * A binary COPY TO's rows as an async iterator of arrays of values, one row
 * per pull, each field decoded by its column's type as a result column of
 * that type is. `columns` names the columns each row holds. It holds what the
 * COPY runs on until its end, which frees it, or until `close()`, `return()`
 * or a failed pull — a field that does not decode among them, a
 * `DecodeError` — which discard the connection if data remained.
 */
export declare class CopyRowStream implements AsyncIterableIterator<PlainValue[]>, AsyncDisposable {
  private constructor();
  readonly columns: readonly string[];
  readonly closed: boolean;
  next(): Promise<IteratorResult<PlainValue[], undefined>>;
  return(): Promise<IteratorResult<PlainValue[], undefined>>;
  close(): Promise<void>;
  [Symbol.asyncIterator](): AsyncIterableIterator<PlainValue[]>;
  [Symbol.asyncDispose](): Promise<void>;
}
