/**
 * pgorm-migration's runner: a program's migrations applied in order, each
 * once, and recorded in the ledger, as Rust's `Migrations` applies them.
 *
 * @module
 */

import type { OperationOptions, Pool, Queryable, Transaction } from "./index.d.ts";

// [spec:pgorm:req:bindings.migrations+1]
/**
 * A migration written in JavaScript: a `name`, the migration's identity in
 * the ledger, and an `up` the runner awaits in the batch's transaction. An
 * optional `checksum` is recorded and checked as Rust's
 * `pgorm_migration::Migration::checksum` is, and an optional `commitsAfter` commits the
 * batch once the migration has run. All three are read once, as the
 * migrator is built.
 */
export interface Migration {
  readonly name: string;
  up(tx: Transaction): Promise<void>;
  readonly checksum?: string | null;
  readonly commitsAfter?: boolean;
}

/** A migration's name and whether the ledger records it applied. */
export interface MigrationWithStatus {
  readonly name: string;
  readonly status: "pending" | "applied";
}

/** A migration whose whole body is one SQL script, pgorm-migration's `SqlMigration`. */
export declare class SqlMigration {
  private constructor();
  /** The migration's name, its file's stem when it was read from one. */
  readonly name: string;
  /** The script `up` runs, whole, in the batch's transaction. */
  readonly sql: string;
  /** The SHA-256 digest of the script, which the ledger records. */
  readonly checksum: string;
  /** Whether the script opens with `-- pgorm: commits-after`. */
  readonly commitsAfter: boolean;
  /**
   * The migration the `.sql` file at `path` holds, named for its stem, its
   * CRLF line endings read as LF. A path naming no file stem is a
   * `ConstructionError`, and any other failure to read an `Error`.
   */
  static read(path: string | URL): SqlMigration;
  /** Every `.sql` file directly inside `directory`, each read as `read` reads one, in file-name order. */
  static readDir(directory: string | URL): SqlMigration[];
}

/** The migration `name` running `sql`. */
export declare function sqlMigration(name: string, sql: string): SqlMigration;

/**
 * What a migrator does when the ledger records a migration it does not
 * know, as Rust's `UnknownMigrations` says it: `"refuse"` fails the read
 * naming each, and `"warn"` writes a warning naming each with
 * `console.warn` and goes on, its `up` refusing only when migrations of its
 * own are pending.
 */
export type UnknownMigrations = "refuse" | "warn";

/** Where `status` writes its listing: `console` unless another is given. */
export interface MigrationLogger {
  info(message: string): void;
  /** Where a warning of a migration the migrator does not know goes; `info` when absent. */
  warn?(message: string): void;
}

/**
 * Where a migrator's ledger lives, the channel its batches notify, and
 * what it does about a migration it does not know.
 */
export interface MigrationsOptions {
  /** The ledger's table name, `pgorm_migrations` unless given. */
  readonly migrationTableName?: string | null;
  /** The ledger's schema; unless given, wherever `search_path` resolves its name. */
  readonly migrationTableSchema?: string | null;
  /** The channel a batch that applied a migration notifies, `STATEMENT_CACHE_CHANNEL` unless given; `null` notifies nothing. */
  readonly statementCacheChannel?: string | null;
  /**
   * What a read does when the ledger records a migration this migrator does
   * not know, `"refuse"` unless given. `"warn"` is for an older build of a
   * program started during a rolling update against a database a newer
   * build migrated: it goes on, and `up` still refuses to apply migrations
   * of its own past ones it does not know.
   */
  readonly unknownMigrations?: UnknownMigrations;
}

/** A migrator built at run time, pgorm-migration's `Migrations`. */
export declare class Migrations {
  private constructor();
  /**
   * Apply the pending migrations, at most `steps` of them when given, as
   * pgorm-migration's `up` applies them: in one transaction on a connection
   * of `pool`'s own, under the advisory lock, committing after a migration
   * whose `commitsAfter` is true, then clearing the pool's statement caches
   * and notifying the channel. A migration's rejection rolls the batch back,
   * and `up` rejects with it. Under `unknownMigrations: "warn"` each
   * migration the ledger records and this migrator does not know is a
   * warning written with `console.warn`.
   */
  up(pool: Pool, steps?: number | null): Promise<void>;
  /** Create the ledger, and its schema, where either is missing. */
  install(db: Queryable, options?: OperationOptions): Promise<void>;
  /**
   * Every migration with its status, read from the ledger and nothing else.
   * An edited migration whose checksum no longer matches is a
   * `ConstructionError`, and so is a ledger row with no migration unless
   * the migrator warns of one, writing the warning with `console.warn`.
   */
  getMigrationWithStatus(db: Queryable, options?: OperationOptions): Promise<MigrationWithStatus[]>;
  /** The migrations the ledger does not record, in order. */
  getPendingMigrations(db: Queryable, options?: OperationOptions): Promise<MigrationWithStatus[]>;
  /** The migrations the ledger records, in order. */
  getAppliedMigrations(db: Queryable, options?: OperationOptions): Promise<MigrationWithStatus[]>;
  /**
   * Log each migration's name and status with `logger.info`, `console`
   * unless given, and then each warning of a migration the migrator does
   * not know with `logger.warn`, or `logger.info` when it has no `warn`.
   */
  status(
    db: Queryable,
    options?: { readonly logger?: MigrationLogger; readonly signal?: AbortSignal },
  ): Promise<void>;
}

/**
 * A migrator applying `list` in order, each a `SqlMigration` or a migration
 * written in JavaScript.
 */
export declare function migrations(
  list: readonly (SqlMigration | Migration)[],
  options?: MigrationsOptions,
): Migrations;
