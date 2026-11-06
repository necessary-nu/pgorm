// @ts-check

// pgorm-migration's runner: a program's migrations applied in order, each
// once, and recorded in the ledger, as Rust's `Migrations` applies them. The
// list, the ledger, the lock and the checksums are the addon's; what lives
// here is the crossing, `dispatch`, which runs a migration's `up` on the
// JavaScript thread with the transaction the runner lends it.
// [spec:pgorm:req:bindings.migrations]

import { fileURLToPath } from "node:url";

import { Pool, poolHandle, Transaction } from "./connections.js";
import { runJob } from "./runs.js";
import { knownOptions, native, TRUSTED } from "./operations.js";

/**
 * A path a migration is read from, as the addon takes it.
 *
 * @param {unknown} path
 * @param {string} what
 * @returns {string}
 */
function pathOf(path, what) {
  const resolved = path instanceof URL ? fileURLToPath(path) : path;
  if (typeof resolved !== "string") throw new TypeError(`${what} is a path, as a string or a file URL`);
  return resolved;
}

/** How a `SqlMigration`'s handle is read by the migrator it is listed in. */
const NATIVE = Symbol("pgorm-napi migration");

/** A migration whose whole body is one SQL script, pgorm-migration's `SqlMigration`. */
export class SqlMigration {
  /** @type {unknown} */
  #handle;

  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    if (token !== TRUSTED) throw new TypeError("a SqlMigration is made by sqlMigration, SqlMigration.read or SqlMigration.readDir");
    this.#handle = handle;
    const [name, sql, checksum, commitsAfter] = /** @type {[string, string, string, boolean]} */ (
      native.sqlMigrationParts(handle)
    );
    /** The migration's name, its file's stem when it was read from one. */
    this.name = name;
    /** The script `up` runs. */
    this.sql = sql;
    /** The SHA-256 digest of the script, which the ledger records. */
    this.checksum = checksum;
    /** Whether the script opens with `-- pgorm: commits-after`. */
    this.commitsAfter = commitsAfter;
    Object.freeze(this);
  }

  /**
   * The migration the `.sql` file at `path` holds, named for its stem.
   *
   * @param {string | URL} path
   * @returns {SqlMigration}
   */
  static read(path) {
    return new SqlMigration(native.sqlMigrationRead(pathOf(path, "a migration file")), TRUSTED);
  }

  /**
   * Every `.sql` file directly inside `directory`, in file-name order.
   *
   * @param {string | URL} directory
   * @returns {SqlMigration[]}
   */
  static readDir(directory) {
    return /** @type {unknown[]} */ (native.sqlMigrationReadDir(pathOf(directory, "a migrations directory"))).map(
      (handle) => new SqlMigration(handle, TRUSTED),
    );
  }

  /** @returns {unknown} */
  get [NATIVE]() {
    return this.#handle;
  }
}

/**
 * The migration `name` running `sql`.
 *
 * @param {string} name
 * @param {string} sql
 * @returns {SqlMigration}
 */
export function sqlMigration(name, sql) {
  if (typeof name !== "string") throw new TypeError("a migration's name is a string");
  if (typeof sql !== "string") throw new TypeError("a migration's SQL is a string");
  return new SqlMigration(native.sqlMigration(name, sql), TRUSTED);
}

/**
 * Run a migration's `up` with the transaction the runner lends it, telling
 * the runner how it settled through `done`, which is always called: a `up`
 * that throws as it is called is a rejection like any other.
 *
 * @param {{ up(tx: Transaction): unknown }} migration
 * @param {unknown} handle
 * @param {(rejected: boolean, rejection?: { error: unknown }) => void} done
 */
function dispatch(migration, handle, done) {
  /** @type {Promise<unknown>} */
  let settled;
  try {
    settled = Promise.resolve(migration.up(new Transaction(handle, null, TRUSTED)));
  } catch (error) {
    done(true, { error });
    return;
  }
  settled.then(
    () => done(false),
    (error) => done(true, { error }),
  );
}

const MIGRATIONS_OPTIONS = ["migrationTableName", "migrationTableSchema", "statementCacheChannel"];

/** A migrator built at run time, pgorm-migration's `Migrations`. */
export class Migrations {
  /** @type {unknown} */
  #handle;

  /**
   * @param {unknown} handle
   * @param {symbol} [token]
   */
  constructor(handle, token) {
    if (token !== TRUSTED) throw new TypeError("a Migrations is made by migrations()");
    this.#handle = handle;
  }

  /**
   * Apply the pending migrations, at most `steps` of them when given, on
   * connections of `pool`'s own.
   *
   * @param {Pool} pool
   * @param {number | null} [steps]
   * @returns {Promise<void>}
   */
  async up(pool, steps = null) {
    if (!(pool instanceof Pool)) throw new TypeError("migrations run on a Pool");
    await native.migrationsUp(this.#handle, poolHandle(pool), steps, dispatch);
  }

  /**
   * Create the ledger, and its schema, where either is missing.
   *
   * @param {import("./index.d.ts").Queryable} db
   * @param {import("./index.d.ts").OperationOptions} [options]
   * @returns {Promise<void>}
   */
  async install(db, options) {
    await runJob(db, native.migrationsJob(this.#handle, true), options);
  }

  /**
   * Every migration with its status, read from the ledger and nothing else.
   *
   * @param {import("./index.d.ts").Queryable} db
   * @param {import("./index.d.ts").OperationOptions} [options]
   * @returns {Promise<import("./index.d.ts").Migration[]>}
   */
  async getMigrationWithStatus(db, options) {
    const found = /** @type {[string, boolean][]} */ (
      await runJob(db, native.migrationsJob(this.#handle, false), options)
    );
    return found.map(([name, applied]) => Object.freeze({ name, status: applied ? "applied" : "pending" }));
  }

  /**
   * The migrations the ledger does not record, in order.
   *
   * @param {import("./index.d.ts").Queryable} db
   * @param {import("./index.d.ts").OperationOptions} [options]
   * @returns {Promise<import("./index.d.ts").Migration[]>}
   */
  async getPendingMigrations(db, options) {
    return (await this.getMigrationWithStatus(db, options)).filter((migration) => migration.status === "pending");
  }

  /**
   * The migrations the ledger records, in order.
   *
   * @param {import("./index.d.ts").Queryable} db
   * @param {import("./index.d.ts").OperationOptions} [options]
   * @returns {Promise<import("./index.d.ts").Migration[]>}
   */
  async getAppliedMigrations(db, options) {
    return (await this.getMigrationWithStatus(db, options)).filter((migration) => migration.status === "applied");
  }

  /**
   * Log each migration's name and status with `logger.info`, as Rust's
   * `status` writes its `tracing` events.
   *
   * @param {import("./index.d.ts").Queryable} db
   * @param {{ logger?: { info(message: string): void }, signal?: AbortSignal }} [options]
   * @returns {Promise<void>}
   */
  async status(db, options = {}) {
    knownOptions(options, ["logger", "signal"]);
    const { logger = console, signal } = options;
    if (typeof logger !== "object" || logger === null || typeof logger.info !== "function") {
      throw new TypeError("logger has an info method");
    }
    logger.info("Checking migration status");
    for (const { name, status } of await this.getMigrationWithStatus(db, signal === undefined ? {} : { signal })) {
      logger.info(`Migration '${name}'... ${status === "applied" ? "Applied" : "Pending"}`);
    }
  }
}

/**
 * A migrator applying `list` in order, each a `SqlMigration` or an object
 * with a `name` and an `up(tx)`, and optionally a `checksum` and a
 * `commitsAfter`, all read once, now.
 *
 * @param {readonly (SqlMigration | import("./index.d.ts").MigrationTrait)[]} list
 * @param {import("./index.d.ts").MigrationsOptions} [options]
 * @returns {Migrations}
 */
export function migrations(list, options = {}) {
  if (!Array.isArray(list)) throw new TypeError("migrations takes an array of migrations");
  knownOptions(options, MIGRATIONS_OPTIONS);
  const { migrationTableName = null, migrationTableSchema = null } = options;
  const channel = "statementCacheChannel" in options ? options.statementCacheChannel : native.statementCacheChannel;
  const entries = list.map((migration) => (migration instanceof SqlMigration ? migration[NATIVE] : migration));
  return new Migrations(
    native.migrationsNew(entries, migrationTableName, migrationTableSchema, channel ?? null),
    TRUSTED,
  );
}
