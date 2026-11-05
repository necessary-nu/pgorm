// @ts-check

// The error classes every failure rejects or throws with.
// [spec:pgorm:req:bindings.errors+5]
// [spec:pgorm:req:napi.errors+3]

/** The base class of every error pgorm-napi rejects with. */
export class PgormError extends Error {}

/** The server could not be reached, or the connection to it broke. */
export class ConnectionError extends PgormError {}

/** An argument could not become the value pgorm sends. */
export class ConstructionError extends PgormError {}

/** A result could not be decoded into the JavaScript value asked for. */
export class DecodeError extends PgormError {}

/** pgorm or the binding failed in a way no input should cause. */
export class InternalError extends PgormError {}

/** A pool, connection, transaction or stream was used after it closed, or while another operation held it. */
export class LifecycleError extends PgormError {}

/** No connection became free within the pool's acquire budget. */
export class TimeoutError extends PgormError {}

/** PostgreSQL rejected the statement. */
export class DatabaseError extends PgormError {
  /**
   * @param {string} message
   * @param {import("./index.d.ts").DatabaseErrorDetails} details
   */
  constructor(message, details) {
    super(message);
    this.sqlstate = details.sqlstate;
    this.severity = details.severity;
    this.detail = details.detail;
    this.hint = details.hint;
    this.schema = details.schema;
    this.table = details.table;
    this.column = details.column;
    this.datatype = details.datatype;
    this.constraint = details.constraint;
  }
}

/** A duplicate key in a unique index or primary key (SQLSTATE `23505`). */
export class UniqueViolationError extends DatabaseError {}

/** A foreign key with no matching referenced row, or a referenced row still in use (`23503`). */
export class ForeignKeyViolationError extends DatabaseError {}

/** A table's or a domain's `CHECK` constraint refused a value (`23514`). */
export class CheckViolationError extends DatabaseError {}

/** A null in a `NOT NULL` column, or in a domain that does not allow one (`23502`). */
export class NotNullViolationError extends DatabaseError {}

/** A row conflicting with another under an exclusion constraint or a `WITHOUT OVERLAPS` key (`23P01`). */
export class ExclusionViolationError extends DatabaseError {}

for (
  const type of [
    PgormError,
    ConnectionError,
    ConstructionError,
    DecodeError,
    InternalError,
    LifecycleError,
    TimeoutError,
    DatabaseError,
    UniqueViolationError,
    ForeignKeyViolationError,
    CheckViolationError,
    NotNullViolationError,
    ExclusionViolationError,
  ]
) {
  Object.defineProperty(type.prototype, "name", {
    value: type.name,
    writable: true,
    configurable: true,
  });
}

/** @type {Record<string, new (message: string, details: any) => PgormError>} */
const classes = {
  ConnectionError,
  ConstructionError,
  DecodeError,
  InternalError,
  LifecycleError,
  TimeoutError,
  DatabaseError,
  UniqueViolationError,
  ForeignKeyViolationError,
  CheckViolationError,
  NotNullViolationError,
  ExclusionViolationError,
};

/**
 * Build the error a failure of `kind` rejects or throws with.
 *
 * @param {string} kind
 * @param {string} message
 * @param {any} details
 */
export function makeError(kind, message, details) {
  return new (classes[kind] ?? PgormError)(message, details);
}
